//! 檔案外部開啟、Open with 選單、終端機啟動與任務狀態徽章管理。

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use super::super::*;

impl App {
    /// 取得目前選取項目的外部開啟目標資訊。
    pub(crate) fn selected_open_target(&self) -> Option<OpenTarget> {
        let pane = self.panes.get(&self.focused_pane)?;
        let entry = pane.selected_entry()?;
        Some(OpenTarget {
            path: entry.path.clone(),
            display_name: entry.display_name(),
            is_dir: entry.is_dir,
        })
    }

    /// 使用預設動作開啟目前選取項目。
    pub(crate) fn open_selected_with_default(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to open");
            return Ok(());
        };
        self.queue_open_action(target, default_open_action())
    }

    /// 打開 `Open with` 面板，讓使用者選擇外部開啟方式。
    pub(crate) fn open_selected_with_picker(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to open");
            return Ok(());
        };
        let options = self.open_picker_options_for_target(&target);

        self.pending_action = Some(PendingAction::OpenPicker {
            pane_id: self.focused_pane,
            target: target.clone(),
            selected: 0,
            options: options.clone(),
        });
        self.status = format!("open with: {}", target.display_name);
        Ok(())
    }

    /// 若正在處理，回傳該工作的資訊（工作編號、標題、完成百分比）。
    pub(crate) fn active_file_job_for_path(
        &self,
        path: &Path,
    ) -> Option<(usize, String, Option<u8>)> {
        if self.active_file_job_busy_paths.is_empty() {
            return None;
        }
        let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        for (task_id, busy_paths) in &self.active_file_job_busy_paths {
            if busy_paths.iter().any(|busy| {
                let canon_busy = busy.canonicalize().unwrap_or_else(|_| busy.to_path_buf());
                canon_path == canon_busy
                    || canon_path.starts_with(&canon_busy)
                    || path == busy
                    || path.starts_with(busy)
            }) {
                let task = self.task_log.iter().find(|t| t.id == *task_id);
                let title = task
                    .map(|t| t.title.clone())
                    .unwrap_or_else(|| String::from("task in progress"));
                let progress = task.and_then(|t| match (t.completed_bytes, t.total_bytes) {
                    (Some(c), Some(tot)) if tot > 0 => {
                        Some(((c as f64 / tot as f64) * 100.0).min(100.0) as u8)
                    }
                    _ => t.progress_percent,
                });
                return Some((*task_id, title, progress));
            }
        }
        None
    }

    /// 回傳指定路徑目前正處於背景工作中的狀態標籤（例如 `[copying 99%]` 或 `[deleting...]`），若無進行中工作則回傳 `None`。
    pub(crate) fn active_job_badge_for_path(&self, path: &Path) -> Option<String> {
        // 一般瀏覽狀態沒有背景工作時是最常見路徑，必須在 canonicalize 前返回。
        // 否則大型目錄即使完全沒有 task，畫一幀仍會對每個可見路徑做同步磁碟查詢。
        if self.active_file_job_busy_paths.is_empty() {
            return None;
        }
        for (task_id, busy_paths) in &self.active_file_job_busy_paths {
            // UI badge 的 busy path 與 pane entry 都由 PaneFM 內部產生，已使用同一套完整
            // 路徑；render 熱路徑只做字面前綴比較，不能再觸發同步檔案系統 I/O。
            if busy_paths
                .iter()
                .any(|busy| path == busy || path.starts_with(busy))
            {
                let task = self.task_log.iter().find(|t| t.id == *task_id)?;
                let action = match task.kind.as_str() {
                    "paste" => {
                        if task.title.starts_with("move") {
                            "moving"
                        } else {
                            "copying"
                        }
                    }
                    "extract" => "extracting",
                    "compress" => "compressing",
                    "delete" => "deleting",
                    _ => "busy",
                };
                let progress = match (task.completed_bytes, task.total_bytes) {
                    (Some(c), Some(tot)) if tot > c && tot > 0 => {
                        Some(((c as f64 / tot as f64) * 100.0).min(100.0) as u8)
                    }
                    _ => task.progress_percent,
                };
                return match progress {
                    Some(pct) => Some(format!("[{action} {pct}%]")),
                    None => match task.completed_bytes {
                        Some(c) if c > 0 => Some(format!("[{action} {}]", format_task_bytes(c))),
                        _ => Some(format!("[{action}...]")),
                    },
                };
            }
        }
        None
    }

    /// 將外部開啟動作排入待執行佇列。
    pub(crate) fn queue_open_action(
        &mut self,
        target: OpenTarget,
        action: OpenAction,
    ) -> io::Result<()> {
        if target.is_dir
            && let Some((task_id, title, progress)) = self.active_file_job_for_path(&target.path)
        {
            let pct_str = progress.map(|p| format!(" ({p}%)")).unwrap_or_default();
            self.status = format!(
                "cannot open '{}': transfer in progress [task #{task_id}: {title}{pct_str}]",
                target.display_name
            );
            return Ok(());
        }
        let launch = build_launch_spec(&target, action)?;
        let title = match action {
            OpenAction::Editor => format!("open {} with editor", target.display_name),
            OpenAction::Vim => format!("open {} with vim", target.display_name),
            OpenAction::Open => format!("open {}", target.display_name),
            OpenAction::Reveal => format!("reveal {}", target.display_name),
        };
        let detail = format!("{} {}", launch.program, launch.args.join(" "));
        let task_id = self.push_task(
            self.focused_pane,
            "open",
            title,
            detail,
            vec![target.path.display().to_string()],
            None,
        );
        self.pending_launch = Some(QueuedLaunch { task_id, launch });
        self.status = match action {
            OpenAction::Editor => format!("opening {} with editor", target.display_name),
            OpenAction::Vim => format!("opening {} with vim", target.display_name),
            OpenAction::Open => format!("opening {}", target.display_name),
            OpenAction::Reveal => format!("revealing {}", target.display_name),
        };
        Ok(())
    }

    /// 以目前 active panel 的目錄開啟新終端，並排入統一外部程序佇列。
    ///
    /// 多 panel 時只讀取 `focused_pane` 的 cwd。若 plugins.toml 有 `[terminal]`，會使用
    /// 公司環境指定的 TrustView 等入口；否則 Windows 直接建立繼承權杖的新 console，
    /// macOS 則優先延續目前終端 App，無法辨識時才交給 Terminal.app。
    ///
    /// 參數：`self: &mut App`，目前應用程式狀態。
    /// 回傳：`io::Result<()>`；active panel 不存在或命令無法建立時回傳錯誤。
    pub(crate) fn open_terminal_in_active_panel(&mut self) -> io::Result<()> {
        let cwd = self
            .panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "panel no longer exists"))?;
        let launch = build_terminal_launch_spec(
            &cwd,
            self.config.actions.terminal.as_ref(),
            &self.config.actions.terminals,
        )?;
        let detail = format!("{} {}", launch.program, launch.args.join(" "));
        let task_id = self.push_task(
            self.focused_pane,
            "terminal",
            format!("open terminal: {}", cwd.display()),
            detail,
            vec![cwd.display().to_string()],
            None,
        );
        self.pending_launch = Some(QueuedLaunch { task_id, launch });
        self.status = format!("opening terminal: {}", cwd.display());
        Ok(())
    }

    /// 根據目前選取目標與設定檔，組出 Open with 面板應顯示的完整選項。
    ///
    /// `plugins.toml` 是使用者客製化層；若自訂動作與內建選項同名（例如 `Vim`
    /// 或 `Reveal`），自訂動作會在原本的位置覆寫內建動作。不同名的外掛才追加到
    /// 選單尾端。名稱比較不區分大小寫，也會移除前後空白。
    ///
    /// 參數：`target: &OpenTarget`，目前 active panel 選取的檔案或目錄。
    /// 回傳：`Vec<OpenPickerOption>`，已套用外掛覆寫且名稱不重複的選項。
    pub(crate) fn open_picker_options_for_target(
        &self,
        target: &OpenTarget,
    ) -> Vec<OpenPickerOption> {
        let mut options = open_picker_options(target);
        let mut label_positions = options
            .iter()
            .enumerate()
            .map(|(index, option)| (option.label.trim().to_lowercase(), index))
            .collect::<BTreeMap<_, _>>();

        for action in self
            .config
            .actions
            .open_with
            .iter()
            .filter(|action| custom_action_applies_to_target(action, target))
        {
            let normalized_label = action.name.trim().to_lowercase();
            if normalized_label.is_empty() {
                continue;
            }
            let option = OpenPickerOption {
                label: action.name.clone(),
                action: OpenPickerAction::Custom(action.clone()),
            };
            if let Some(index) = label_positions.get(&normalized_label).copied() {
                options[index] = option;
            } else {
                let index = options.len();
                options.push(option);
                label_positions.insert(normalized_label, index);
            }
        }
        options
    }

    /// 依照 Open with 面板中的選項類型，排入內建或自訂的外部動作。
    pub(crate) fn queue_open_picker_action(
        &mut self,
        target: OpenTarget,
        action: OpenPickerAction,
    ) -> io::Result<()> {
        match action {
            OpenPickerAction::Builtin(action) => self.queue_open_action(target, action),
            OpenPickerAction::Custom(action) => {
                let launch = build_custom_launch_spec(&target, &action)?;
                let title = format!("run {} on {}", action.name, target.display_name);
                let detail = format!("{} {}", launch.program, launch.args.join(" "));
                let task_id = self.push_task(
                    self.focused_pane,
                    "open",
                    title,
                    detail,
                    vec![target.path.display().to_string()],
                    None,
                );
                self.pending_launch = Some(QueuedLaunch { task_id, launch });
                self.status = format!("running {} on {}", action.name, target.display_name);
                Ok(())
            }
        }
    }

    /// 根據外部開啟結果，更新 task manager 中對應任務的最終狀態。
    pub(crate) fn finish_launch_task(&mut self, task_id: usize, result: io::Result<()>) {
        match result {
            Ok(()) => self.finish_task(task_id, TaskState::Done, String::from("completed")),
            Err(error) => {
                let detail = error.to_string();
                self.finish_task(task_id, TaskState::Failed, detail.clone());
                self.status = format!("open failed: {detail}");
            }
        }
    }
}
