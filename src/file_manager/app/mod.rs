//! PaneFM 的應用狀態機、命令分派與使用者操作流程。
//!
//! [`App`] 是第一層協調者：每個 panel 的瀏覽資料由 `PaneState` 保存，全域只保留
//! layout、焦點、剪貼簿與背景 task。`handle_key` 會按照「暫時 UI -> 文字輸入 ->
//! panel 模式 -> 一般列表」的優先順序分派事件，避免同一按鍵同時觸發兩種行為。
//!
//! 新功能應盡量把檔案系統或平台細節放進對應模組，這裡只保留狀態轉換；所有會
//! 阻塞的搜尋、外部程式或網路操作都必須排入背景流程，不能卡住 TUI 主執行緒。

use std::{
    collections::{BTreeMap, BTreeSet},
    env, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    config::{AppConfig, LoadedConfig, StartupLinemode, StartupSort, persist_theme},
    theme::{Theme, ThemePreset},
};

use super::{
    archive::{
        ExtractedArchive, compress_entries_to_zip, compress_entries_to_zip_with_progress,
        default_extract_output_path, detect_archive_format, extract_entries,
        extract_entries_with_progress,
    },
    bookmark::{BookmarkEntry, BookmarkStore, BookmarkTarget, bookmark_file_path},
    copy::{CopyAction, build_copy_text, copy_action_status_label, copy_picker_options},
    debug_timing_log, debug_timing_message,
    diff::{DiffJobEvent, DiffMatrixState, launch_content_diff_spec, spawn_background_diff},
    filesystem_watcher::FilesystemWatcher,
    fuzzy::{fuzzy_matched_indices, fuzzy_matched_indices_by_fields},
    layout::{LayoutNode, SplitDirection, SplitPlacement, pane_spatial_cmp},
    open::{
        LaunchSpec, OpenAction, OpenPickerAction, OpenPickerOption, OpenTarget,
        build_custom_launch_spec, build_launch_spec, build_terminal_launch_spec,
        custom_action_applies_to_target, default_open_action, open_picker_options,
    },
    operation_history::{
        DEFAULT_HISTORY_LIMIT, FileOperation, FileOperationKind, OperationHistory, OperationItem,
    },
    pane::{
        DirectoryLoadProgress, FilterMode, LineMode, PaneState, SortDetailKind, SortMode,
        TransferProgress,
    },
    platform::{read_text_from_system_clipboard, write_text_to_system_clipboard},
    search::{
        GlobalSearchEntry, GlobalSearchEvent, stream_content_search_entries, stream_search_entries,
    },
    smb::{ResolvedSmbLocation, build_smb_mount_launch, parse_smb_location},
    task_history::{load_task_history, save_task_history, task_history_file_path},
    tools::external_tool_statuses,
    trash::{TrashListEntry, TrashStore},
    ui::{
        CommandPaletteState, CommandSuggestionLine, HelpPanelLine, InlineEditorState,
        InlinePickerState, PaneListState, SearchListState, render_bookmark_action_picker,
        render_bookmark_picker, render_command_palette, render_confirm_dialog, render_diff_matrix,
        render_filter_input, render_global_search_panel, render_go_picker, render_linemode_picker,
        render_pane, render_paste_overwrite_dialog, render_preview_search_input,
        render_theme_command_picker, render_theme_picker, render_trash_confirm_dialog,
        render_window_picker, render_window_resize_picker, render_yank_picker,
        render_zoxide_picker, visible_list_window_range,
    },
    zoxide::{ZoxideTracker, query_zoxide_directories},
};

#[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
use super::smb::resolve_smb_location;

#[cfg(any(all(not(target_os = "windows"), not(target_os = "macos")), test))]
use super::smb::resolve_smb_location_with_mount_root;

/// 表示 rename 輸入框目前採用的編輯模式。
///
/// `Insert` 代表可以直接插入文字，游標會顯示成細線；
/// `Normal` 代表遵循 Vim 的一般模式，只負責移動游標與切換模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameMode {
    Insert,
    Normal,
}

/// 表示共用文字輸入器處理按鍵後的結果。
///
/// 各輸入 UI 只需要處理自己的 Enter 與關閉行為；字元插入、刪除、游標移動及
/// Vim 模式切換都由這個結果統一描述，避免不同介面各自實作後產生操作差異。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextEditResult {
    Changed,
    Consumed,
    PassThrough,
}

/// 表示目前剪貼簿保存的是複製還是剪下操作。
///
/// 這個模式會決定 `p` 貼上時，是保留來源還是把來源移動到新位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardOperation {
    Copy,
    Cut,
}

/// 記錄目前暫存在檔案管理器內部剪貼簿中的單一項目。
///
/// 這一版先支援單一檔案或資料夾，之後若要擴充多選，
/// 可以再把這個結構改成清單形式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardEntry {
    pub(crate) source_path: PathBuf,
    pub(crate) display_name: String,
}

/// 表示目前內部剪貼簿保存的一批項目與其操作模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardState {
    pub(crate) entries: Vec<ClipboardEntry>,
    pub(crate) operation: ClipboardOperation,
}

/// 記錄目前 filter 的目標 pane、查詢字串、比對模式與是否仍在輸入中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
    pub(crate) mode: FilterMode,
}

/// 記錄目前 preview search 的目標 pane、查詢字串與是否仍在輸入中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreviewSearchState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
}

/// 記錄目前列表內 find-next 的目標 pane 與輸入中的查詢字串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListFindState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
}

/// 描述目前 pane 已排隊、準備交給主事件迴圈執行的 `fzf` 跳轉請求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FzfJumpRequest {
    pub(crate) pane_id: usize,
    pub(crate) root_dir: PathBuf,
    pub(crate) show_hidden: bool,
    pub(crate) follow_links: bool,
    pub(crate) task_id: usize,
}

/// 記錄目前 global search 的目標 pane、查詢文字與搜尋結果狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalSearchState {
    pub(crate) pane_id: usize,
    pub(crate) root_dir: PathBuf,
    pub(crate) mode: SearchMode,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
    pub(crate) loading: bool,
    pub(crate) searched: bool,
    pub(crate) selected: usize,
    pub(crate) results: Vec<GlobalSearchEntry>,
    /// 只過濾已回傳結果的模糊 filter，不會重新執行 `fd` 或 `rg` 搜尋。
    pub(crate) filter: PanelSearchState,
    pub(crate) preview_scroll: Option<usize>,
    pub(crate) preview_current_match: Option<usize>,
    pub(crate) task_id: Option<usize>,
}

/// 表示目前搜尋面板是在找路徑，還是在找檔案內容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchMode {
    Path,
    Content,
}

impl SearchMode {
    /// 回傳狀態列與說明文字使用的搜尋模式標籤。
    pub(crate) fn status_label(self) -> &'static str {
        match self {
            SearchMode::Path => "global search",
            SearchMode::Content => "content search",
        }
    }

    /// 回傳搜尋輸入框標題。
    fn panel_title(self, _editing: bool) -> &'static str {
        match self {
            SearchMode::Path => " Global search file by fd ",
            SearchMode::Content => " Global search content by rg ",
        }
    }
}

/// 描述排隊中的外部命令與它對應的 task id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueuedLaunch {
    pub(crate) task_id: usize,
    pub(crate) launch: LaunchSpec,
}

/// 描述目前已知可升級的新版本資訊（用於渲染頂部黃底紅字徽章）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateBadgeInfo {
    pub(crate) latest_version: String,
    pub(crate) download_url: String,
    pub(crate) asset_name: String,
}

/// 描述一次 UNC 網路路徑背景跳轉的完成訊息。
///
/// worker 會在主執行緒之外複製並載入 [`PaneState`]；主迴圈收到結果後，只有在
/// task 尚未被取消且目標 panel 仍存在時才套用，避免失聯 SMB 主機凍結整個 TUI。
#[derive(Debug)]
pub(crate) struct NetworkGotoEvent {
    /// 對應 task manager 中的任務編號。
    task_id: usize,
    /// 啟動跳轉時的 active panel 編號。
    pane_id: usize,
    /// 使用者輸入的 UNC 目標，供狀態列與錯誤訊息顯示。
    target: PathBuf,
    /// 背景載入完成的 panel 狀態，或作業系統回傳的 I/O 錯誤。
    result: io::Result<PaneState>,
}

/// 記錄目前是否處於範圍標記模式，以及起點和目前游標位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisualSelectionState {
    pub(crate) pane_id: usize,
    pub(crate) anchor: usize,
    pub(crate) current: usize,
}

/// 描述暫時面板中的搜尋輸入狀態。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PanelSearchState {
    pub(crate) buffer: String,
    pub(crate) editing: bool,
}

/// 表示目前正在等待使用者完成的暫時互動。
///
/// 只要有 pending action，輸入會先被它攔截，
/// 而不會直接進到一般檔案瀏覽模式。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PendingAction {
    ConfirmDelete {
        pane_id: usize,
        target_name: String,
        permanent: bool,
        warning_message: Option<String>,
    },
    ConfirmPasteOverwrite {
        pane_id: usize,
        target_name: String,
        entry_count: usize,
        operation: ClipboardOperation,
    },
    ConfirmTrashAction {
        action: TrashConfirmAction,
        target_name: String,
        entry_count: usize,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    },
    SortPicker {
        pane_id: usize,
    },
    GoPicker {
        pane_id: usize,
    },
    WindowPicker {
        pane_id: usize,
    },
    WindowResize {
        pane_id: usize,
    },
    LineModePicker {
        pane_id: usize,
    },
    YankPicker {
        pane_id: usize,
    },
    ThemePicker {
        selected: usize,
        original: ThemePreset,
    },
    ThemeCommandPicker {
        pane_id: usize,
    },
    TrashPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    },
    HelpPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
        custom_title: Option<String>,
        custom_entries: Option<Vec<HelpEntry>>,
    },
    TaskPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
        marked_ids: Vec<usize>,
        visual_anchor: Option<usize>,
    },
    BookmarkPicker {
        pane_id: usize,
    },
    BookmarkList {
        pane_id: usize,
        selected: usize,
        mode: BookmarkListMode,
        search: PanelSearchState,
    },
    ZoxideList {
        pane_id: usize,
        selected: usize,
        entries: Vec<PathBuf>,
        search: PanelSearchState,
    },
    ToolPanel {
        pane_id: usize,
        selected: usize,
    },
    CopyPicker {
        pane_id: usize,
        target: OpenTarget,
        selected: usize,
    },
    OpenPicker {
        pane_id: usize,
        target: OpenTarget,
        selected: usize,
        options: Vec<OpenPickerOption>,
    },
    Rename {
        pane_id: usize,
        original_name: String,
        buffer: String,
        cursor: usize,
        mode: RenameMode,
    },
    CreateEntry {
        pane_id: usize,
        buffer: String,
        cursor: usize,
        mode: RenameMode,
    },
    RegexRename {
        pane_id: usize,
        pattern: String,
        replacement: String,
        selected: usize,
        previews: Vec<RegexRenamePreview>,
    },
    DiffMatrix(DiffMatrixState),
    EasyMotion {
        pane_id: usize,
        target_char: Option<char>,
        labels: Vec<(char, usize)>,
    },
}

/// 表示整個應用程式的核心狀態。
///
/// 這個結構整合了設定、主題、視窗布局、焦點與互動模式，
/// 是整個 TUI 運作時最主要的狀態容器。
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) config: AppConfig,
    pub(crate) config_source: PathBuf,
    pub(crate) theme: Theme,
    pub(crate) theme_preset: ThemePreset,
    pub(crate) trash_store: TrashStore,
    pub(crate) bookmark_store: BookmarkStore,
    pub(crate) panes: BTreeMap<usize, PaneState>,
    pub(crate) layout: LayoutNode,
    pub(crate) focused_pane: usize,
    pub(crate) next_pane_id: usize,
    pub(crate) status: String,
    pub(crate) command_mode: bool,
    pub(crate) command_buffer: String,
    /// 所有非 inline 文字輸入 UI 共用的 Vim 編輯模式。
    pub(crate) text_input_mode: RenameMode,
    /// 所有非 inline 文字輸入 UI 共用的字元游標位置，不是 UTF-8 byte offset。
    pub(crate) text_input_cursor: usize,
    pub(crate) command_suggestion_selected: usize,
    pub(crate) command_completion_cycle: Option<CommandCompletionCycle>,
    pub(crate) pending_count: Option<usize>,
    pub(crate) pending_g: bool,
    pub(crate) pending_y: bool,
    pub(crate) pending_bookmark: Option<BookmarkPrompt>,
    pub(crate) clipboard: Option<ClipboardState>,
    /// 全域檔案操作歷史；跨 panel 的 copy/move 仍應以同一批次復原。
    pub(crate) operation_history: OperationHistory,
    pub(crate) filter: Option<FilterState>,
    pub(crate) preview_search: Option<PreviewSearchState>,
    pub(crate) list_find: Option<ListFindState>,
    pub(crate) global_search: Option<GlobalSearchState>,
    pub(crate) global_search_rx: Option<Receiver<GlobalSearchEvent>>,
    pub(crate) global_search_cancelled: Option<Arc<AtomicBool>>,
    pub(crate) active_global_search_task_id: Option<usize>,
    pub(crate) diff_job_rx: Option<Receiver<DiffJobEvent>>,
    pub(crate) diff_job_cancelled: Option<Arc<AtomicBool>>,
    /// 目前 UNC `goto` 背景工作的接收端；`None` 代表沒有等待中的網路跳轉。
    network_goto_rx: Option<Receiver<NetworkGotoEvent>>,
    /// 目前 UNC `goto` 對應的 task id，供 Esc 與 task panel 取消後捨棄晚到結果。
    active_network_goto_task_id: Option<usize>,
    /// 所有大型 paste/compress/extract 工作接收端，以 task id 區分並允許並行完成。
    file_job_receivers: BTreeMap<usize, Receiver<FileJobEvent>>,
    /// 記錄目前正在由背景工作處理（寫入、壓縮、解壓、刪除）的路徑集合，用來防止使用者在傳輸中途進入未完成的目錄。
    active_file_job_busy_paths: BTreeMap<usize, Vec<PathBuf>>,
    /// 每個 panel 各自擁有的 linemode size 背景掃描，不會互相覆蓋或阻塞 TUI。
    directory_size_jobs: BTreeMap<usize, DirectorySizeJob>,
    /// 每個 panel 最新一次非阻塞目錄讀取；新導航會取代舊 worker 並即時取消舊掃描。
    directory_load_jobs: BTreeMap<usize, DirectoryLoadJob>,
    /// 已成功讀取的目錄清單快取；重複進出大型目錄時先立即顯示，再由背景結果校正。
    directory_entry_cache: BTreeMap<PathBuf, Vec<super::entry::FileEntry>>,
    pub(crate) visual_selection: Option<VisualSelectionState>,
    pub(crate) pending_action: Option<PendingAction>,
    pub(crate) help_return: Option<HelpReturnState>,
    pub(crate) pending_launch: Option<QueuedLaunch>,
    pub(crate) pending_fzf_jump: Option<FzfJumpRequest>,
    pub(crate) task_log: Vec<TaskRecord>,
    pub(crate) next_task_id: usize,
    /// task 歷史的實際檔案位置；每次狀態變更與關閉前都會同步寫入。
    pub(crate) task_history_path: PathBuf,
    /// 非阻塞記錄瀏覽目錄，避免同步啟動 zoxide 拖慢 TUI。
    pub(crate) zoxide_tracker: ZoxideTracker,
    /// 監看 Finder、Explorer 與其他程式對目前 panel 目錄造成的外部變更。
    filesystem_watcher: Option<FilesystemWatcher>,
    /// watcher 短時間內回報的目錄先集中在這裡，等 debounce 到期再一起刷新。
    pending_watched_directories: BTreeSet<PathBuf>,
    /// 下一次允許套用 watcher 刷新的時間；`None` 代表目前沒有待處理事件。
    filesystem_refresh_deadline: Option<Instant>,
    /// 要求主事件迴圈在下一幀前清除實體 terminal 與 ratatui buffer。
    pub(crate) full_redraw_requested: bool,
    /// 記錄最近一次 render 時 panels 所分配到的區域（不含頂部 tabs 與底部 status/hint）。
    pub(crate) latest_pane_area: Option<Rect>,
    /// 描述目前已知可升級的新版本資訊（用於渲染頂部黃底紅字徽章）。
    pub(crate) update_badge_info: Option<UpdateBadgeInfo>,
    /// 背景版本檢查接收端。
    pub(crate) update_check_rx: Option<Receiver<crate::updater::UpdateCheckResult>>,
    /// 內部就地升級工作接收端。
    pub(crate) in_app_update_rx: Option<Receiver<InAppUpdateMsg>>,
    /// 目前是否正在下載與安裝更新。
    pub(crate) in_app_updating: bool,
    /// 版本控制（Git 與 SVN）背景管理與查詢 worker。
    pub(crate) vcs_manager: super::vcs::VcsManager,
}

/// 應用程式內部就地升級通訊訊息。
#[derive(Debug)]
pub(crate) enum InAppUpdateMsg {
    /// 下載進度事件（已下載位元組, 總位元組）。
    Progress {
        downloaded: usize,
        total: Option<u64>,
    },
    /// 升級結束事件（成功回傳新版本字串，失敗回傳錯誤訊息）。
    Completed(Result<String, String>),
}

mod commands;
mod completion;
mod file_ops;
mod fs_jobs;
mod help;
mod keys;
mod keys_util;
mod line_editor;
mod navigation;
mod pickers;
mod polling;
mod rename;
mod selection;
mod status;
mod tasks;
#[cfg(test)]
mod tests;
mod trash;

pub(crate) use completion::*;
pub(crate) use fs_jobs::*;
pub(crate) use help::*;
pub(crate) use keys_util::*;
pub(crate) use line_editor::*;
pub(crate) use pickers::*;
pub(crate) use rename::*;
pub(crate) use status::*;
pub(crate) use tasks::*;
pub(crate) use trash::*;

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

        let vcs_manager = super::vcs::VcsManager::new();
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

    /// 回傳目前 active panel 的目錄；供 OSC 7 終端同步或外部查詢使用。
    pub(crate) fn active_pane_cwd(&self) -> Option<&Path> {
        self.panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.as_path())
    }

    /// 取得目前 panels 分配到的區域，若尚未 render 則回退至終端實際大小。
    pub(crate) fn current_pane_area(&self) -> Rect {
        self.latest_pane_area.unwrap_or_else(|| {
            crossterm::terminal::size()
                .map(|(w, h)| Rect::new(0, 0, w, h.saturating_sub(3)))
                .unwrap_or_else(|_| Rect::new(0, 0, 120, 40))
        })
    }

    /// 根據目前應用程式狀態繪製整個畫面。
    ///
    /// 繪製前會先依 terminal cell 寬度切割狀態文字，再動態計算 status area 高度。
    /// 這讓一般通知仍只占一行，而貼上失敗的完整 destination 與 OS error 可以依實際
    /// 長度展開成多行，不會因終端視窗較窄而遺失錯誤尾端。
    ///
    /// 參數：
    /// - `self: &mut App`，提供目前 panel、輸入模式、狀態文字及 theme 等畫面狀態。
    /// - `frame: &mut ratatui::Frame<'_>`，ratatui 本次更新可使用的繪圖 frame。
    ///
    /// 回傳：`Option<(u16, u16)>`；畫面需要顯示文字輸入游標時回傳其 cell 座標，
    /// 否則回傳 `None`。畫面內容會直接寫入傳入的 `frame`。
    pub(crate) fn render(&mut self, frame: &mut ratatui::Frame<'_>) -> Option<(u16, u16)> {
        let raw_status_text = if self.command_mode {
            format!(":{}", self.command_buffer)
        } else {
            self.status.clone()
        };
        let status_text = wrap_status_text(&raw_status_text, frame.area().width);
        let status_style = if self.command_mode {
            Style::default()
        } else if status_is_error(&raw_status_text) {
            self.theme.danger_style()
        } else {
            Style::default()
        };
        let status_height = status_area_height(&status_text, frame.area().height.saturating_sub(3));
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(status_height),
            ])
            .split(frame.area());

        self.latest_pane_area = Some(outer[0]);
        let mut pane_rects = BTreeMap::new();
        self.layout.render_rects(outer[0], &mut pane_rects);
        // PATH 掃描只在 dependency 面板真正顯示時執行。一般檔案列表每幀都重查四個
        // 外部命令不但沒有畫面用途，也會讓鍵盤回應時間受磁碟與網路 PATH 影響。
        let tool_statuses = matches!(self.pending_action, Some(PendingAction::ToolPanel { .. }))
            .then(external_tool_statuses)
            .unwrap_or_default();
        let mut cursor_position = None;
        for (&pane_id, &rect) in &pane_rects {
            let trash_overlay_state =
                trash_panel_overlay_state_from_pending_action(&self.pending_action, pane_id);
            let trash_lines = if let Some((selected, search, marked_ids, visual_anchor)) =
                trash_overlay_state.as_ref()
            {
                Some(
                    trash_panel_lines(
                        &self.trash_store,
                        &search.buffer,
                        marked_ids,
                        visual_anchor.map(|anchor| (anchor, *selected)),
                    )
                    .unwrap_or_default(),
                )
            } else {
                None
            };
            let task_records = if matches!(
                &self.pending_action,
                Some(PendingAction::TaskPanel {
                    pane_id: action_pane_id,
                    ..
                }) if *action_pane_id == pane_id
            ) {
                Some(self.tasks_for_pane(pane_id))
            } else {
                None
            };
            let task_lines = if let (
                Some(records),
                Some(PendingAction::TaskPanel {
                    pane_id: action_pane_id,
                    search,
                    marked_ids,
                    visual_anchor,
                    selected,
                }),
            ) = (task_records.as_ref(), self.pending_action.as_ref())
            {
                if *action_pane_id == pane_id {
                    let filtered = filtered_task_entries(records, &search.buffer);
                    let mut effective_marked = marked_ids.clone();
                    if let Some(anchor) = visual_anchor {
                        let start = (*anchor).min(*selected);
                        let end = (*anchor).max(*selected);
                        for task in filtered
                            .iter()
                            .skip(start)
                            .take(end.saturating_sub(start) + 1)
                        {
                            if !effective_marked.contains(&task.id) {
                                effective_marked.push(task.id);
                            }
                        }
                    }
                    Some(task_panel_lines(&filtered, &effective_marked))
                } else {
                    None
                }
            } else {
                None
            };
            let help_lines = if let Some(PendingAction::HelpPanel {
                pane_id: action_pane_id,
                search,
                custom_entries,
                ..
            }) = &self.pending_action
            {
                if *action_pane_id == pane_id {
                    Some(if let Some(custom) = custom_entries {
                        filter_custom_help_entries(custom, &search.buffer)
                            .into_iter()
                            .map(|e| e.line)
                            .collect()
                    } else {
                        help_panel_lines(&search.buffer)
                    })
                } else {
                    None
                }
            } else {
                None
            };
            let regex_rename_lines = if let Some(PendingAction::RegexRename {
                pane_id: action_pane_id,
                previews,
                ..
            }) = &self.pending_action
            {
                if *action_pane_id == pane_id {
                    Some(regex_rename_panel_lines(previews))
                } else {
                    None
                }
            } else {
                None
            };
            let global_search_results = self
                .global_search
                .as_ref()
                .filter(|search| search.pane_id == pane_id)
                .map(|search| {
                    filtered_global_search_entries(&search.results, &search.filter.buffer)
                });
            // 工作標籤只可能出現在目前 viewport。舊實作會對大型目錄的全部項目呼叫
            // canonicalize；`deps` 有六萬筆時，單次 render 就產生六萬次同步 I/O，連
            // j/k、mn、T 都會被阻塞。這裡只檢查畫面實際看得到的數十筆路徑。
            let active_job_badges = if self.active_file_job_busy_paths.is_empty() {
                std::collections::HashMap::new()
            } else {
                self.panes
                    .get(&pane_id)
                    .map(|pane| {
                        visible_job_badge_paths(pane, rect.height.saturating_sub(2) as usize)
                            .into_iter()
                            .filter_map(|path| {
                                self.active_job_badge_for_path(&path)
                                    .map(|badge| (path, badge))
                            })
                            .collect::<std::collections::HashMap<_, _>>()
                    })
                    .unwrap_or_default()
            };
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                let rename_buffer = match &self.pending_action {
                    Some(PendingAction::Rename {
                        pane_id: rename_pane_id,
                        buffer,
                        cursor,
                        mode,
                        ..
                    }) if *rename_pane_id == pane_id => Some(InlineEditorState {
                        buffer: buffer.as_str(),
                        cursor: *cursor,
                        title: match mode {
                            RenameMode::Insert => " Rename (insert): ",
                            RenameMode::Normal => " Rename (normal): ",
                        },
                    }),
                    Some(PendingAction::CreateEntry {
                        pane_id: create_pane_id,
                        buffer,
                        cursor,
                        mode,
                    }) if *create_pane_id == pane_id => Some(InlineEditorState {
                        buffer: buffer.as_str(),
                        cursor: *cursor,
                        title: create_editor_title(*mode),
                    }),
                    _ => None,
                };
                let picker_options = match &self.pending_action {
                    Some(PendingAction::CopyPicker {
                        pane_id: copy_pane_id,
                        ..
                    }) if *copy_pane_id == pane_id => Some(
                        copy_picker_options()
                            .into_iter()
                            .map(|option| format!("{} -> {}", option.shortcut, option.label))
                            .collect::<Vec<_>>(),
                    ),
                    Some(PendingAction::OpenPicker {
                        pane_id: open_pane_id,
                        options,
                        ..
                    }) if *open_pane_id == pane_id => Some(
                        options
                            .iter()
                            .map(|option| option.label.clone())
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                };
                let picker_state = match &self.pending_action {
                    Some(PendingAction::CopyPicker {
                        pane_id: copy_pane_id,
                        selected,
                        ..
                    }) if *copy_pane_id == pane_id => {
                        picker_options.as_ref().map(|options| InlinePickerState {
                            title: " Copy: ",
                            options,
                            selected: *selected,
                        })
                    }
                    Some(PendingAction::OpenPicker {
                        pane_id: open_pane_id,
                        selected,
                        ..
                    }) if *open_pane_id == pane_id => {
                        picker_options.as_ref().map(|options| InlinePickerState {
                            title: " Open with: ",
                            options,
                            selected: *selected,
                        })
                    }
                    _ => None,
                };
                let panel_state = if let Some(search) = self.global_search.as_ref() {
                    (search.pane_id == pane_id && (search.loading || search.searched)).then_some(
                        PaneListState::Search(SearchListState {
                            results: global_search_results.as_deref().unwrap_or(&[]),
                            selected: search.selected,
                            loading: search.loading && self.config.search.show_loading,
                            preview_query: matches!(search.mode, SearchMode::Content)
                                .then_some(search.buffer.as_str()),
                            preview_scroll: search.preview_scroll,
                            preview_current_match: search.preview_current_match,
                        }),
                    )
                } else if let Some((selected, search, ..)) = trash_overlay_state.as_ref() {
                    Some(PaneListState::Trash {
                        lines: trash_lines.as_deref().unwrap_or(&[]),
                        selected: *selected,
                        search: &search.buffer,
                        editing: search.editing,
                        cursor: self.text_input_cursor,
                    })
                } else if let Some(PendingAction::TaskPanel {
                    pane_id: action_pane_id,
                    selected,
                    search,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Tasks {
                            lines: task_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                            search: &search.buffer,
                            editing: search.editing,
                            cursor: self.text_input_cursor,
                        })
                    } else {
                        None
                    }
                } else if let Some(PendingAction::HelpPanel {
                    pane_id: action_pane_id,
                    selected,
                    search,
                    custom_title,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Help {
                            lines: help_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                            search: &search.buffer,
                            editing: search.editing,
                            cursor: self.text_input_cursor,
                            custom_title: custom_title.as_deref(),
                        })
                    } else {
                        None
                    }
                } else if let Some(PendingAction::ToolPanel {
                    pane_id: action_pane_id,
                    selected,
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Tools {
                            statuses: &tool_statuses,
                            selected: *selected,
                        })
                    } else {
                        None
                    }
                } else if let Some(PendingAction::RegexRename {
                    pane_id: action_pane_id,
                    selected,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::RegexRename {
                            lines: regex_rename_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let preview_active = pane.is_preview_active();
                let easymotion_labels = match &self.pending_action {
                    Some(PendingAction::EasyMotion {
                        pane_id: action_pane_id,
                        labels,
                        ..
                    }) if *action_pane_id == pane_id && !labels.is_empty() => {
                        Some(labels.as_slice())
                    }
                    _ => None,
                };
                let update_badge = if pane_id == self.focused_pane {
                    if self.in_app_updating {
                        Some(("...", true))
                    } else {
                        self.update_badge_info
                            .as_ref()
                            .map(|info| (info.latest_version.as_str(), false))
                    }
                } else {
                    None
                };
                let pane_cursor = render_pane(
                    frame,
                    rect,
                    pane_id,
                    pane,
                    pane_id == self.focused_pane,
                    preview_active,
                    self.visual_selection.as_ref().and_then(|selection| {
                        (selection.pane_id == pane_id)
                            .then_some((selection.anchor, selection.current))
                    }),
                    panel_state,
                    self.theme,
                    &self.config,
                    rename_buffer,
                    picker_state,
                    self.list_find
                        .as_ref()
                        .filter(|search| search.pane_id == pane_id)
                        .map(|search| search.buffer.as_str()),
                    self.list_find
                        .as_ref()
                        .is_some_and(|search| search.pane_id == pane_id),
                    self.text_input_cursor,
                    &active_job_badges,
                    easymotion_labels,
                    update_badge,
                );
                if cursor_position.is_none() {
                    cursor_position = pane_cursor;
                }
            }
        }

        let shortcut_hints = self.active_status_shortcut_hints();
        let help = Paragraph::new(status_shortcut_line(
            outer[1].width,
            self.theme,
            &shortcut_hints,
        ))
        .block(Block::default().borders(Borders::TOP));
        frame.render_widget(help, outer[1]);

        frame.render_widget(Paragraph::new(status_text).style(status_style), outer[2]);

        if self.command_mode
            && let Some(area) = pane_rects.get(&self.focused_pane)
        {
            let command_suggestions = self.command_suggestions();
            let command_cursor = render_command_palette(
                frame,
                *area,
                self.theme,
                CommandPaletteState {
                    buffer: &self.command_buffer,
                    suggestions: &command_suggestions,
                    selected: self.command_suggestion_selected,
                    cursor: self.text_input_cursor,
                    mode: self.text_input_mode,
                },
            );
            if cursor_position.is_none() {
                cursor_position = Some(command_cursor);
            }
        }

        if let Some(filter) = &self.filter
            && filter.editing
            && let Some(area) = pane_rects.get(&filter.pane_id)
        {
            let title = match filter.mode {
                FilterMode::Normal => " Filter [Normal] (Tab: Fuzzy) ",
                FilterMode::Fuzzy => " Filter [Fuzzy] (Tab: Normal) ",
            };
            let filter_cursor = render_filter_input(
                frame,
                *area,
                self.theme,
                title,
                &filter.buffer,
                self.text_input_cursor,
            );
            if cursor_position.is_none() {
                cursor_position = Some(filter_cursor);
            }
        }

        if let Some(search) = &self.preview_search
            && search.editing
            && let Some(area) = pane_rects.get(&search.pane_id)
        {
            let search_cursor = render_preview_search_input(
                frame,
                *area,
                self.theme,
                &search.buffer,
                self.text_input_cursor,
            );
            if cursor_position.is_none() {
                cursor_position = Some(search_cursor);
            }
        }

        if let Some(search) = &self.global_search
            && let Some(area) = pane_rects.get(&search.pane_id)
        {
            if search.editing {
                let search_cursor = render_global_search_panel(
                    frame,
                    *area,
                    self.theme,
                    search.mode.panel_title(true),
                    &search.buffer,
                    self.text_input_cursor,
                    true,
                );
                if cursor_position.is_none() {
                    cursor_position = Some(search_cursor);
                }
            } else if search.filter.editing {
                let filter_cursor = render_filter_input(
                    frame,
                    *area,
                    self.theme,
                    " Filter Results ",
                    &search.filter.buffer,
                    self.text_input_cursor,
                );
                if cursor_position.is_none() {
                    cursor_position = Some(filter_cursor);
                }
            }
        }

        match &mut self.pending_action {
            Some(PendingAction::ConfirmDelete {
                target_name,
                permanent,
                warning_message,
                ..
            }) => {
                render_confirm_dialog(
                    frame,
                    frame.area(),
                    target_name,
                    *permanent,
                    warning_message.as_deref(),
                    self.theme,
                    &self.config,
                );
            }
            Some(PendingAction::ConfirmPasteOverwrite {
                target_name,
                entry_count,
                ..
            }) => {
                render_paste_overwrite_dialog(
                    frame,
                    frame.area(),
                    target_name,
                    *entry_count,
                    self.theme,
                    &self.config,
                );
            }
            Some(PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                ..
            }) => {
                let confirm_area = trash_confirm_panel_id(action)
                    .and_then(|pane_id| pane_rects.get(&pane_id).copied())
                    .unwrap_or(frame.area());
                render_trash_confirm_dialog(
                    frame,
                    confirm_area,
                    action,
                    target_name,
                    *entry_count,
                    self.theme,
                    &self.config,
                );
            }
            Some(PendingAction::GoPicker { .. }) => {
                render_go_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::ThemeCommandPicker { .. }) => {
                render_theme_command_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::SortPicker { .. }) => {
                super::ui::render_sort_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::WindowPicker { .. }) => {
                render_window_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::WindowResize { .. }) => {
                render_window_resize_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::LineModePicker { .. }) => {
                render_linemode_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::YankPicker { .. }) => {
                render_yank_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::BookmarkPicker { .. }) => {
                render_bookmark_action_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::ThemePicker { selected, .. }) => {
                render_theme_picker(frame, frame.area(), self.theme, *selected, &self.config);
            }
            Some(PendingAction::BookmarkList {
                pane_id,
                selected,
                mode,
                search,
            }) => {
                let filtered =
                    filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer);
                let lines = bookmark_panel_lines(filtered);
                if let Some(area) = pane_rects.get(pane_id) {
                    let (title, empty_message) = bookmark_picker_copy(*mode);
                    let bookmark_cursor = render_bookmark_picker(
                        frame,
                        *area,
                        self.theme,
                        &lines,
                        *selected,
                        title,
                        empty_message,
                        &search.buffer,
                        search.editing,
                        self.text_input_cursor,
                    );
                    if search.editing && cursor_position.is_none() {
                        cursor_position = bookmark_cursor;
                    }
                }
            }
            Some(PendingAction::ZoxideList {
                pane_id,
                selected,
                entries,
                search,
            }) => {
                let filtered = filtered_zoxide_entries(entries, &search.buffer);
                let lines = zoxide_panel_lines(filtered);
                if let Some(area) = pane_rects.get(pane_id) {
                    let zoxide_cursor = render_zoxide_picker(
                        frame,
                        *area,
                        self.theme,
                        &lines,
                        *selected,
                        &search.buffer,
                        search.editing,
                        self.text_input_cursor,
                    );
                    if search.editing && cursor_position.is_none() {
                        cursor_position = zoxide_cursor;
                    }
                }
            }
            Some(PendingAction::DiffMatrix(state)) => {
                render_diff_matrix(frame, frame.area(), state, self.theme);
            }
            Some(PendingAction::TrashPanel { .. })
            | Some(PendingAction::TaskPanel { .. })
            | Some(PendingAction::HelpPanel { .. })
            | Some(PendingAction::ToolPanel { .. })
            | Some(PendingAction::CopyPicker { .. })
            | Some(PendingAction::OpenPicker { .. })
            | Some(PendingAction::RegexRename { .. })
            | Some(PendingAction::EasyMotion { .. }) => {}
            Some(PendingAction::Rename { .. }) | Some(PendingAction::CreateEntry { .. }) => {}
            None => {}
        }

        if let Some((x, y)) = cursor_position {
            frame.set_cursor_position((x, y));
        }
        cursor_position
    }

    /// 回傳目前畫面應該呈現的 rename 游標模式。
    ///
    /// 回傳：`Option<RenameMode>`。
    /// - `Some(RenameMode::Insert)` 代表應顯示細線游標。
    /// - `Some(RenameMode::Normal)` 代表應顯示方塊游標。
    /// - `None` 代表目前沒有 rename 輸入框，不需要特別切換。
    pub(crate) fn rename_cursor_mode(&self) -> Option<RenameMode> {
        match &self.pending_action {
            Some(PendingAction::Rename { mode, .. })
            | Some(PendingAction::CreateEntry { mode, .. }) => Some(*mode),
            Some(PendingAction::TrashPanel { search, .. })
            | Some(PendingAction::HelpPanel { search, .. })
            | Some(PendingAction::TaskPanel { search, .. })
            | Some(PendingAction::BookmarkList { search, .. })
            | Some(PendingAction::ZoxideList { search, .. })
                if search.editing =>
            {
                Some(self.text_input_mode)
            }
            _ if self.command_mode
                || self.filter.as_ref().is_some_and(|filter| filter.editing)
                || self
                    .preview_search
                    .as_ref()
                    .is_some_and(|search| search.editing)
                || self.list_find.is_some() =>
            {
                Some(self.text_input_mode)
            }
            _ if self
                .global_search
                .as_ref()
                .is_some_and(|search| search.editing || search.filter.editing) =>
            {
                Some(self.text_input_mode)
            }
            _ => None,
        }
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

    /// 取出並清除下一幀的完整重畫需求。
    ///
    /// 參數：`self: &mut App`，目前應用程式狀態。
    /// 回傳：`bool`；`true` 代表事件迴圈必須先呼叫 `Terminal::clear()`。
    pub(crate) fn take_full_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.full_redraw_requested)
    }

    /// 取出目前排隊中的外部開啟請求，交給主事件迴圈處理。
    pub(crate) fn take_pending_launch(&mut self) -> Option<QueuedLaunch> {
        self.pending_launch.take()
    }

    /// 取出目前排隊中的 `fzf` 跳轉請求，交給主事件迴圈處理。
    pub(crate) fn take_pending_fzf_jump(&mut self) -> Option<FzfJumpRequest> {
        self.pending_fzf_jump.take()
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

/// 收集目前 viewport 中需要查詢背景工作標籤的檔案路徑。
///
/// 參數：
/// - `pane: &PaneState`：提供 filter 後索引、游標與上一幀捲動位置的 panel。
/// - `viewport_height: usize`：列表實際可顯示的資料列數。
///
/// 回傳：`Vec<PathBuf>`，最多只包含一個 viewport 的路徑。刻意不回傳完整目錄清單，
/// 避免 task badge 查詢在大型目錄每幀對數萬個檔案執行 `canonicalize()`。
pub(crate) fn visible_job_badge_paths(pane: &PaneState, viewport_height: usize) -> Vec<PathBuf> {
    let (start, end) = visible_list_window_range(
        pane.visible_indices.len(),
        pane.selected,
        viewport_height.max(1),
        pane.list_state.offset(),
    );
    pane.visible_indices[start..end]
        .iter()
        .filter_map(|entry_index| pane.entries.get(*entry_index))
        .map(|entry| entry.path.clone())
        .collect()
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

/// 以共用模糊 matcher 過濾 `fd` 或 `rg` 已回傳的搜尋結果。
///
/// 參數：
/// - `entries: &[GlobalSearchEntry]`，背景搜尋目前已串流回主執行緒的原始結果。
/// - `query: &str`，結果面板中按 `f` 後輸入的模糊查詢。
///
/// 回傳：`Vec<GlobalSearchEntry>`，依模糊分數排序的可見結果副本；原始串流順序不會被修改。
pub(crate) fn filtered_global_search_entries(
    entries: &[GlobalSearchEntry],
    query: &str,
) -> Vec<GlobalSearchEntry> {
    fuzzy_matched_indices(entries, query, |entry| entry.relative_path.clone().into())
        .into_iter()
        .map(|index| entries[index].clone())
        .collect()
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
