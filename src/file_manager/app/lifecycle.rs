use std::{
    collections::{BTreeMap, BTreeSet},
    env, io,
    path::PathBuf,
};

use crate::config::{AppConfig, LoadedConfig, StartupLinemode, StartupSort};

use crate::file_manager::{
    bookmark::{BookmarkStore, bookmark_file_path},
    filesystem_watcher::FilesystemWatcher,
    layout::LayoutNode,
    operation_history::{DEFAULT_HISTORY_LIMIT, OperationHistory},
    pane::{LineMode, PaneState, SortMode},
    task_history::{load_task_history, save_task_history, task_history_file_path},
    tools::external_tool_statuses,
    trash::TrashStore,
    vcs::VcsManager,
    zoxide::ZoxideTracker,
};

use super::{
    state::{App, RenameMode, UpdateBadgeInfo},
    status::unix_time_ms_now,
    tasks::TaskState,
};

impl App {
    /// 建立一個新的應用程式狀態。
    ///
    /// 參數：
    /// - `cwd: PathBuf`，啟動時第一個 pane 要打開的目錄。
    /// - `loaded_config: LoadedConfig`，啟動時已載入的設定與來源資訊。
    ///
    /// 回傳：`io::Result<App>`。
    /// - 成功時回傳完整初始化的應用程式狀態。
    /// - 失敗時回傳建立第一個 pane 或載入目錄時的 I/O 錯誤。
    pub(crate) fn new(cwd: PathBuf, loaded_config: LoadedConfig) -> io::Result<Self> {
        let LoadedConfig {
            config,
            source,
            base_dir,
        } = loaded_config;
        let app_base = if base_dir.as_os_str().is_empty() {
            &cwd
        } else {
            &base_dir
        };
        let trash_store = TrashStore::new(app_base)?;
        let _ = crate::file_manager::undo_backup::resolve_undo_backup_dir();
        let config_source = source
            .clone()
            .unwrap_or_else(|| app_base.join("config.toml"));
        let task_history_path = task_history_file_path(app_base, source.as_deref());
        let (mut task_log, task_history_warning) = match load_task_history(&task_history_path) {
            Ok(tasks) => (tasks, None),
            Err(error) => (
                Vec::new(),
                Some(format!("task history could not be loaded: {error}")),
            ),
        };
        let now = unix_time_ms_now();
        let mut recovered_interrupted_tasks = 0usize;
        for task in &mut task_log {
            // 上次程序若在 task 尚未完成時被關閉，背景 thread 已隨 process 消失。
            // 不可把它繼續顯示成 RUNNING，更不可未經確認就重新覆寫 SMB 目標。
            if task.state == TaskState::Running {
                task.state = TaskState::Interrupted;
                task.finished_at_unix_ms = Some(now);
                task.detail = format!("{}; interrupted when PaneFM closed", task.detail);
                recovered_interrupted_tasks += 1;
            }
            // 新 session 只建立 panel #1；把歷史歸到可見 panel，避免舊 panel id 讓
            // 使用者按 T 後找不到紀錄。原始目標仍完整保存在 title/detail。
            task.pane_id = 1;
        }
        if task_log.len() > 200 {
            let overflow = task_log.len() - 200;
            task_log.drain(0..overflow);
        }
        let next_task_id = task_log
            .iter()
            .map(|task| task.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let bookmark_store = BookmarkStore::load(bookmark_file_path(app_base, source.as_deref()))
            .map_err(|error| io::Error::other(error.to_string()))?;
        let zoxide_tracker = ZoxideTracker::new();
        zoxide_tracker.track(&cwd);
        let mut pane = PaneState::new(cwd.clone())?;
        apply_config_to_pane(&config, &mut pane);
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);
        let theme_preset = config.ui.theme_preset;
        #[cfg(not(test))]
        let (filesystem_watcher, watcher_startup_warning) = if config.watcher.enabled {
            match FilesystemWatcher::new(config.watcher.fallback_poll_interval) {
                Ok(watcher) => (Some(watcher), None),
                Err(error) => (
                    None,
                    Some(format!("filesystem watcher unavailable: {error}")),
                ),
            }
        } else {
            (None, None)
        };
        // 單元測試會直接注入變更目錄驗證刷新邏輯，不為每個 App 測試建立兩條
        // 作業系統 watcher thread，避免數百個測試同時消耗平台資源。
        #[cfg(test)]
        let (filesystem_watcher, watcher_startup_warning): (
            Option<FilesystemWatcher>,
            Option<String>,
        ) = (None, None);
        let startup_status = match source {
            Some(path) => format!("loaded config: {}", path.display()),
            None => String::from("normal mode"),
        };
        let missing_tools = external_tool_statuses()
            .into_iter()
            .filter(|tool| !tool.installed)
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        let mut startup_status = if missing_tools.is_empty() {
            startup_status
        } else {
            format!(
                "{startup_status}; missing dependencies: {}",
                missing_tools.join(", ")
            )
        };
        if let Some(warning) = task_history_warning {
            startup_status = format!("{startup_status}; {warning}");
        } else if recovered_interrupted_tasks > 0 {
            startup_status = format!(
                "{startup_status}; recovered {recovered_interrupted_tasks} interrupted task(s)"
            );
        }
        if let Some(warning) = watcher_startup_warning {
            startup_status = format!("{startup_status}; {warning}");
        }

        let (update_badge_info, update_check_rx) = {
            let cache = crate::updater::load_update_cache(None);
            let current_version = env!("CARGO_PKG_VERSION");
            let badge_info = cache.as_ref().and_then(|c| {
                if crate::updater::is_newer_version(&c.latest_version, current_version) {
                    Some(UpdateBadgeInfo {
                        latest_version: c.latest_version.clone(),
                        download_url: c.download_url.clone(),
                        asset_name: c.asset_name.clone(),
                    })
                } else {
                    None
                }
            });

            #[cfg(not(test))]
            let rx = {
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if crate::updater::should_check_remote(
                    cache.as_ref(),
                    now_secs,
                    crate::updater::DEFAULT_CHECK_INTERVAL_SECS,
                ) {
                    Some(crate::updater::spawn_background_update_check(5, None))
                } else {
                    None
                }
            };
            #[cfg(test)]
            let rx = None;

            (badge_info, rx)
        };

        let vcs_manager = VcsManager::new();
        if config.ui.vcs.enabled {
            vcs_manager.request_query(1, cwd.clone());
        }

        let app = Self {
            config,
            config_source,
            theme: theme_preset.into(),
            theme_preset,
            trash_store,
            bookmark_store,
            panes,
            layout: LayoutNode::Leaf { pane_id: 1 },
            focused_pane: 1,
            next_pane_id: 2,
            status: startup_status,
            command_mode: false,
            command_buffer: String::new(),
            text_input_mode: RenameMode::Insert,
            text_input_cursor: 0,
            command_suggestion_selected: 0,
            command_completion_cycle: None,
            pending_count: None,
            pending_g: false,
            pending_y: false,
            pending_bookmark: None,
            clipboard: None,
            operation_history: OperationHistory::new(DEFAULT_HISTORY_LIMIT),
            filter: None,
            preview_search: None,
            list_find: None,
            global_search: None,
            global_search_rx: None,
            global_search_cancelled: None,
            active_global_search_task_id: None,
            diff_job_rx: None,
            diff_job_cancelled: None,
            network_goto_rx: None,
            active_network_goto_task_id: None,
            file_job_receivers: BTreeMap::new(),
            active_file_job_busy_paths: BTreeMap::new(),
            directory_size_jobs: BTreeMap::new(),
            directory_load_jobs: BTreeMap::new(),
            directory_entry_cache: BTreeMap::new(),
            visual_selection: None,
            pending_action: None,
            help_return: None,
            pending_launch: None,
            pending_fzf_jump: None,
            task_log,
            next_task_id,
            task_history_path,
            zoxide_tracker,
            filesystem_watcher,
            pending_watched_directories: BTreeSet::new(),
            filesystem_refresh_deadline: None,
            full_redraw_requested: false,
            latest_pane_area: None,
            update_badge_info,
            update_check_rx,
            in_app_update_rx: None,
            in_app_updating: false,
            vcs_manager,
        };
        if recovered_interrupted_tasks > 0 {
            save_task_history(&app.task_history_path, &app.task_log)?;
        }
        Ok(app)
    }

    /// 執行正常關閉前的 task 收尾與持久化。
    ///
    /// 尚在背景 thread 執行的工作不能跨 process 真正暫停；PaneFM 會把它們標記為
    /// `Interrupted`，保留當下 byte 進度與診斷資料，但不在下次啟動時自動覆寫目的地。
    /// 即使程式被強制關閉、來不及執行本函數，啟動載入也會把磁碟上的 `Running`
    /// 紀錄轉成 `Interrupted`。
    ///
    /// 參數：無。
    /// 回傳：`io::Result<()>`，確保離開主迴圈前最後一份歷史已寫入磁碟。
    pub(crate) fn prepare_for_shutdown(&mut self) -> io::Result<()> {
        let finished_at = unix_time_ms_now();
        for task in &mut self.task_log {
            if task.state == TaskState::Running {
                task.state = TaskState::Interrupted;
                task.finished_at_unix_ms = Some(finished_at);
                task.detail = format!("{}; interrupted when PaneFM closed", task.detail);
            }
        }
        save_task_history(&self.task_history_path, &self.task_log)
    }
}

/// 將設定檔中的啟動偏好套用到新建立的 pane。
///
/// 參數：
/// - `config: &AppConfig`，目前啟動所使用的設定。
/// - `pane: &mut PaneState`，要被套用預設值的 pane。
///
/// 回傳：`()`
pub(crate) fn apply_config_to_pane(config: &AppConfig, pane: &mut PaneState) {
    pane.set_show_hidden(config.pane.show_hidden);
    pane.set_sort_mode(sort_mode_from_config(
        config.pane.default_sort,
        config.pane.default_sort_reverse,
    ));
    pane.set_line_mode(linemode_from_config(config.pane.default_linemode));
}

/// 將設定檔中的 linemode 偏好轉成 pane 實際使用的模式。
pub(crate) fn linemode_from_config(linemode: StartupLinemode) -> LineMode {
    match linemode {
        StartupLinemode::Mtime => LineMode::Mtime,
        StartupLinemode::Btime => LineMode::Btime,
        StartupLinemode::Size => LineMode::Size,
        StartupLinemode::Permissions => LineMode::Permissions,
        StartupLinemode::None => LineMode::None,
    }
}

/// 將設定檔中的排序偏好轉成 pane 實際使用的排序模式。
///
/// 參數：
/// - `sort: StartupSort`，設定檔指定的排序種類。
/// - `reverse: bool`，是否使用反向排序。
///
/// 回傳：`SortMode`，可直接套用到 pane 的排序模式。
pub(crate) fn sort_mode_from_config(sort: StartupSort, reverse: bool) -> SortMode {
    match sort {
        StartupSort::Alphabetical => SortMode::Alphabetical { reverse },
        StartupSort::Natural => SortMode::Natural { reverse },
        StartupSort::Size => SortMode::Size { reverse },
        StartupSort::Modified => SortMode::Modified { reverse },
        StartupSort::Created => SortMode::Created { reverse },
        StartupSort::Extension => SortMode::Extension { reverse },
        StartupSort::Random => SortMode::Random,
    }
}

/// 從命令列參數中取出單一書籤按鍵。
///
/// 參數：
/// - `args: &str`，使用者在 `:bookmark ...` 後輸入的內容。
///
/// 回傳：`Option<char>`。
/// - `Some(char)` 代表成功解析出唯一按鍵。
/// - `None` 代表輸入為空，或不是單一字元。
pub(crate) fn parse_bookmark_argument(args: &str) -> Option<char> {
    let trimmed = args.trim();
    let mut chars = trimmed.chars();
    let key = chars.next()?;
    if chars.next().is_some() || key.is_whitespace() {
        return None;
    }
    Some(key)
}

/// 從命令列參數中取出 pane 編號。
///
/// 參數：
/// - `args: &str`，使用者在 `:move-panel ...` 後輸入的內容。
///
/// 回傳：`Option<usize>`。
/// - `Some(usize)` 代表成功解析成有效編號。
/// - `None` 代表輸入為空、不是數字或小於 1。
pub(crate) fn parse_pane_id_argument(args: &str) -> Option<usize> {
    let trimmed = args.trim();
    let id = trimmed.parse::<usize>().ok()?;
    (id > 0).then_some(id)
}

/// 判斷目前 command mode 輸入看起來是不是一條目錄或檔案路徑。
pub(crate) fn looks_like_navigation_path(input: &str) -> bool {
    let trimmed = input.trim();
    !trimmed.is_empty()
        && (trimmed.starts_with('/')
            || trimmed.starts_with("~/")
            || trimmed.starts_with("~\\")
            || trimmed == "~"
            || trimmed.starts_with("./")
            || trimmed.starts_with(".\\")
            || trimmed.starts_with("../")
            || trimmed.starts_with("..\\")
            || is_windows_drive_path(trimmed)
            || is_unc_path(trimmed)
            || trimmed.starts_with("smb://"))
}

/// 判斷字串是否為 Windows 磁碟機開頭的絕對路徑，例如 `C:/work` 或 `D:\\repo`。
pub(crate) fn is_windows_drive_path(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

/// 判斷字串是否為 UNC 路徑，例如 `\\\\server\\share` 或 `//server/share`。
pub(crate) fn is_unc_path(input: &str) -> bool {
    input.starts_with("\\\\") || input.starts_with("//")
}

/// 展開 `~` 開頭的家目錄路徑，讓 command mode 也能直接輸入家目錄捷徑。
pub(crate) fn expand_tilde_path(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed == "~" {
        return command_home_dir().map(|home| home.to_string_lossy().to_string());
    }

    let suffix = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))?;
    let home = command_home_dir()?;
    let mut path = home.to_string_lossy().to_string();
    if !path.ends_with(std::path::MAIN_SEPARATOR) {
        path.push(std::path::MAIN_SEPARATOR);
    }
    path.push_str(suffix);
    Some(path)
}

/// 取得 command mode 需要用來展開 `~` 的使用者家目錄。
pub(crate) fn command_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
