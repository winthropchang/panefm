#![allow(unused_imports)]

use super::*;

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

    /// 打開 `Copy` 面板，讓使用者把不同格式的文字直接複製到系統剪貼簿。
    pub(crate) fn open_copy_picker(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to copy");
            return Ok(());
        };

        self.pending_action = Some(PendingAction::CopyPicker {
            pane_id: self.focused_pane,
            target: target.clone(),
            selected: 0,
        });
        self.status = format!("copy to clipboard: {}", target.display_name);
        Ok(())
    }

    /// 根據選擇的複製動作，把文字寫進系統剪貼簿。
    pub(crate) fn copy_target_to_system_clipboard(
        &mut self,
        target: OpenTarget,
        action: CopyAction,
    ) -> io::Result<()> {
        let text = build_copy_text(&target, action)
            .map_err(|error| io::Error::other(error.to_string()))?;
        write_text_to_system_clipboard(&text)?;
        self.status = format!(
            "{}: {}",
            copy_action_status_label(action),
            target.display_name
        );
        Ok(())
    }

    /// 檢查指定路徑是否目前正由背景檔案工作（複製、貼上、移動、刪除、壓縮、解壓）寫入或修改中。
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

    /// 開始重新命名流程，建立一個待輸入的新名稱互動。
    pub(crate) fn start_rename(&mut self) {
        let Some(entry) = self
            .panes
            .get(&self.focused_pane)
            .and_then(PaneState::selected_entry)
            .cloned()
        else {
            self.status = String::from("nothing selected to rename");
            return;
        };

        self.pending_action = Some(PendingAction::Rename {
            pane_id: self.focused_pane,
            original_name: entry.display_name(),
            cursor: rename_basename_cursor(&entry.name),
            buffer: entry.name,
            mode: RenameMode::Insert,
        });
        self.status = String::from("rename: insert");
    }

    /// 真正執行重新命名目前待確認項目的檔案系統操作。
    pub(crate) fn confirm_rename(
        &mut self,
        pane_id: usize,
        original_name: &str,
        new_name: &str,
    ) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let rename_result = {
            let pane = self
                .panes
                .get_mut(&pane_id)
                .expect("checked pane existence before rename");
            pane.rename_selected(new_name)
        };

        match rename_result {
            Ok(Some(renamed_name)) => {
                self.reload_all_panes()?;
                self.status = format!("renamed {original_name} -> {renamed_name}");
            }
            Ok(None) => {
                self.status = String::from("nothing selected to rename");
            }
            Err(error) => {
                self.status = format!("failed to rename {original_name}: {error}");
            }
        }

        Ok(())
    }

    /// 開始建立新檔案流程，打開一個可直接輸入名稱的 inline 編輯器。
    ///
    /// 參數：無。
    /// 回傳：`()`
    /// 打開一個可直接輸入建立路徑的 inline 編輯器。
    ///
    /// 回傳：`()`
    pub(crate) fn start_create_entry(&mut self) {
        self.pending_action = Some(PendingAction::CreateEntry {
            pane_id: self.focused_pane,
            buffer: String::new(),
            cursor: 0,
            mode: RenameMode::Insert,
        });
        self.status = create_status_label("insert");
    }

    /// 真正執行建立新項目的檔案系統操作。
    ///
    /// 參數：
    /// - `pane_id: usize`，要建立項目的目標 pane。
    /// - `path: &str`，新項目的相對路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn confirm_create_entry(&mut self, pane_id: usize, path: &str) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let create_result = {
            let pane = self
                .panes
                .get_mut(&pane_id)
                .expect("checked pane existence before create");
            pane.create_entry(path)
        };

        match create_result {
            Ok(created_name) => {
                self.reload_all_panes()?;
                let item_type = if created_name.ends_with('/') {
                    "directory"
                } else {
                    "file"
                };
                self.status = format!("created {item_type}: {created_name}");
            }
            Err(error) => {
                self.status = format!("failed to create entry: {error}");
            }
        }

        Ok(())
    }

    /// 讓命令模式可以直接建立新項目，而不必再開啟 inline 輸入框。
    ///
    /// 參數：
    /// - `path: &str`，命令列中指定的新路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn create_entry_from_command(&mut self, path: &str) -> io::Result<()> {
        self.confirm_create_entry(self.focused_pane, path)
    }

    /// 解析 `:rename-regex` 指令參數，並建立批次改名預覽。
    ///
    /// 參數：
    /// - `args: &str`，使用者輸入在 `rename-regex` 後面的原始參數字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn start_regex_rename_from_command(&mut self, args: &str) -> io::Result<()> {
        let parsed = shlex::split(args).unwrap_or_default();
        if parsed.len() != 2 {
            self.status = String::from("usage: rename-regex <pattern> <replace>");
            return Ok(());
        }
        self.open_regex_rename_preview(&parsed[0], &parsed[1])
    }

    /// 依照 regex 規則建立目前選取項目的批次改名預覽。
    ///
    /// 參數：
    /// - `pattern: &str`，要用來匹配檔名的 regex。
    /// - `replacement: &str`，匹配後要替換成的新文字，支援 `$1` 這類群組語法。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn open_regex_rename_preview(
        &mut self,
        pattern: &str,
        replacement: &str,
    ) -> io::Result<()> {
        let regex = match Regex::new(pattern) {
            Ok(regex) => regex,
            Err(error) => {
                self.status = format!("invalid regex: {error}");
                return Ok(());
            }
        };
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to rename");
            return Ok(());
        }

        let selected_paths = entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>();
        let existing_names = pane
            .entries
            .iter()
            .filter(|entry| !selected_paths.contains(&entry.path))
            .map(|entry| entry.name.clone())
            .collect::<BTreeSet<_>>();

        let mut previews = entries
            .into_iter()
            .map(|entry| {
                let new_name = regex.replace_all(&entry.name, replacement).into_owned();
                let outcome = classify_regex_rename_preview(&entry.name, &new_name);
                RegexRenamePreview {
                    source_path: entry.path,
                    original_name: entry.name,
                    new_name,
                    outcome,
                }
            })
            .collect::<Vec<_>>();

        let mut target_counts = BTreeMap::new();
        for preview in previews
            .iter()
            .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
        {
            *target_counts
                .entry(preview.new_name.clone())
                .or_insert(0usize) += 1;
        }

        for preview in &mut previews {
            if !matches!(preview.outcome, RegexRenameOutcome::Ready) {
                continue;
            }
            if target_counts.get(&preview.new_name).copied().unwrap_or(0) > 1
                || existing_names.contains(&preview.new_name)
            {
                preview.outcome = RegexRenameOutcome::Conflict;
            }
        }

        self.pending_action = Some(PendingAction::RegexRename {
            pane_id: self.focused_pane,
            pattern: pattern.to_string(),
            replacement: replacement.to_string(),
            selected: 0,
            previews,
        });
        if let Some(action) = self.pending_action.as_ref() {
            self.status = self.status_for_pending_action(action)?;
        }
        Ok(())
    }

    /// 真正執行 regex 批次改名預覽中所有可套用的項目。
    ///
    /// 參數：
    /// - `pane_id: usize`，發起這次批次改名的 pane。
    /// - `previews: &[RegexRenamePreview]`，先前建立好的預覽結果。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn apply_regex_rename_preview(
        &mut self,
        pane_id: usize,
        previews: &[RegexRenamePreview],
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let cwd = pane.cwd.clone();
        let ready = previews
            .iter()
            .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
            .cloned()
            .collect::<Vec<_>>();

        if previews.iter().any(|preview| {
            matches!(
                preview.outcome,
                RegexRenameOutcome::Conflict | RegexRenameOutcome::Invalid
            )
        }) {
            self.status = String::from("rename-regex: resolve conflicts before apply");
            return Ok(());
        }
        if ready.is_empty() {
            self.status = String::from("rename-regex: nothing to apply");
            return Ok(());
        }

        let staged = ready
            .iter()
            .enumerate()
            .map(|(index, preview)| {
                let temp_path =
                    unique_regex_rename_temp_path(&cwd, &preview.original_name, index, &ready);
                (
                    preview.source_path.clone(),
                    temp_path,
                    cwd.join(&preview.new_name),
                )
            })
            .collect::<Vec<_>>();

        for (source_path, temp_path, _) in &staged {
            fs::rename(source_path, temp_path)?;
        }

        for (_, temp_path, final_path) in &staged {
            if let Err(error) = fs::rename(temp_path, final_path) {
                self.status = format!("rename-regex failed: {error}");
                return Ok(());
            }
        }

        self.reload_all_panes()?;
        self.status = if ready.len() == 1 {
            String::from("rename-regex: renamed 1 item")
        } else {
            format!("rename-regex: renamed {} items", ready.len())
        };
        Ok(())
    }

    /// 將目前選取或標記的項目壓成單一 zip 檔，並在完成後刷新所有 pane。
    pub(crate) fn compress_selected_entries(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("nothing selected to compress");
            return Ok(());
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to compress");
            return Ok(());
        }

        let target_dir = pane.cwd.clone();
        if entries_should_run_in_background(&entries) {
            return self.start_background_compress(self.focused_pane, target_dir, entries);
        }
        let archive_path = compress_entries_to_zip(&target_dir, &entries)?;
        self.reload_all_panes()?;
        let _ = self.reveal_path_and_track(self.focused_pane, &archive_path);

        self.status = if entries.len() == 1 {
            format!(
                "compressed {} -> {}",
                entries[0].display_name(),
                archive_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive.zip")
            )
        } else {
            format!(
                "compressed {} items -> {}",
                entries.len(),
                archive_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive.zip")
            )
        };
        Ok(())
    }

    /// 解開目前選取或標記的壓縮檔，並盡量把游標帶到第一個輸出結果。
    pub(crate) fn extract_selected_archives(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("nothing selected to extract");
            return Ok(());
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to extract");
            return Ok(());
        }

        let target_dir = pane.cwd.clone();
        if entries_should_run_in_background(&entries) {
            return self.start_background_extract(self.focused_pane, target_dir, entries);
        }
        let (extracted, skipped) = extract_entries(&target_dir, &entries)?;
        if extracted.is_empty() {
            self.status = if skipped == 0 {
                String::from("nothing selected to extract")
            } else {
                format!("no supported archives selected (skipped {skipped})")
            };
            return Ok(());
        }

        self.reload_all_panes()?;
        self.reveal_first_extracted_output(&extracted)?;

        self.status = extraction_status_label(&extracted, skipped);
        Ok(())
    }

    /// 把大型壓縮工作排入背景執行，避免 ZIP deflate 長時間占住 TUI 主執行緒。
    ///
    /// 參數：`pane_id: usize` 為來源 panel；`target_dir: PathBuf` 為輸出目錄；
    /// `entries: Vec<FileEntry>` 為本次要壓縮的項目。
    /// 回傳：`io::Result<()>`；成功代表工作已排入 task manager。
    pub(crate) fn start_background_compress(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        entries: Vec<crate::file_manager::entry::FileEntry>,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let first_name = entries
            .first()
            .map(|entry| entry.display_name())
            .unwrap_or_else(|| String::from("item"));
        let task_id = self.push_task(
            pane_id,
            "compress",
            format!("compress {entry_count} item(s)"),
            format!("output: {}", target_dir.display()),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let busy_paths = entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        self.active_file_job_busy_paths.insert(task_id, busy_paths);
        let (sender, receiver) = std::sync::mpsc::channel();
        let total_bytes = entries
            .iter()
            .map(|entry| entry.size)
            .fold(0u64, u64::saturating_add);
        self.update_task_progress(task_id, 0, total_bytes);
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result =
                compress_entries_to_zip_with_progress(&target_dir, &entries, &mut progress);
            let _ = sender.send(FileJobEvent::Compress {
                task_id,
                pane_id,
                entry_count,
                first_name,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("compressing {entry_count} item(s) in background [task {task_id}]");
        Ok(())
    }

    /// 把大型解壓工作排入背景執行，讓使用者可在其他 panel 繼續操作。
    ///
    /// 參數：`pane_id: usize` 為來源 panel；`target_dir: PathBuf` 為輸出目錄；
    /// `entries: Vec<FileEntry>` 為選取的壓縮檔。
    /// 回傳：`io::Result<()>`；成功代表工作已排入 task manager。
    pub(crate) fn start_background_extract(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        entries: Vec<crate::file_manager::entry::FileEntry>,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let task_id = self.push_task(
            pane_id,
            "extract",
            format!("extract {entry_count} item(s)"),
            format!("output: {}", target_dir.display()),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let mut busy_paths = entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        for entry in &entries {
            if let Some(format) = detect_archive_format(&entry.path) {
                busy_paths.push(default_extract_output_path(
                    &target_dir,
                    &entry.path,
                    format,
                ));
            }
        }
        self.active_file_job_busy_paths.insert(task_id, busy_paths);
        let (sender, receiver) = mpsc::channel();
        // 解壓後資料通常比壓縮檔大，因此這是估算分母；完成事件會校正成最終 byte。
        let total_bytes = entries
            .iter()
            .map(|entry| entry.size)
            .fold(0u64, u64::saturating_add);
        self.update_task_progress(task_id, 0, total_bytes);
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result = extract_entries_with_progress(&target_dir, &entries, &mut progress);
            let _ = sender.send(FileJobEvent::Extract {
                task_id,
                pane_id,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("extracting {entry_count} item(s) in background [task {task_id}]");
        Ok(())
    }

    /// 將目前焦點 pane 的游標帶到第一個解壓結果，方便使用者立刻繼續操作。
    pub(crate) fn reveal_first_extracted_output(
        &mut self,
        extracted: &[ExtractedArchive],
    ) -> io::Result<()> {
        let Some(first) = extracted.first() else {
            return Ok(());
        };
        let _ = self.reveal_path_and_track(self.focused_pane, &first.output_path);
        Ok(())
    }

    /// 開始刪除確認流程，建立一個待確認的刪除互動。
    pub(crate) fn start_delete_confirmation(&mut self, permanent: bool) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = if permanent {
                String::from("nothing selected to delete")
            } else {
                String::from("nothing selected to trash")
            };
            return;
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = if permanent {
                String::from("nothing selected to delete")
            } else {
                String::from("nothing selected to trash")
            };
            return;
        }

        let target_name = if entries.len() == 1 {
            entries[0].display_name()
        } else {
            format!("{} items", entries.len())
        };

        let total_bytes = entries
            .iter()
            .map(|e| e.directory_size.unwrap_or(e.size))
            .fold(0u64, u64::saturating_add);

        let warning_message = if permanent {
            None
        } else {
            let is_cross = entries
                .iter()
                .any(|e| self.trash_store.is_cross_device(&e.path));
            let is_large = total_bytes > 10 * 1024 * 1024
                || entries
                    .iter()
                    .any(|e| e.size > 10 * 1024 * 1024 || (e.is_dir && is_cross));

            if is_cross {
                if total_bytes > 0 {
                    Some(format!(
                        "[!] Cross-device move ({}) will copy data and take time.",
                        format_task_bytes(total_bytes)
                    ))
                } else {
                    Some(String::from(
                        "[!] Cross-device move will copy data and take time.",
                    ))
                }
            } else if is_large {
                Some(format!(
                    "[!] Large item ({}) detected.",
                    format_task_bytes(total_bytes)
                ))
            } else {
                None
            }
        };

        self.pending_action = Some(PendingAction::ConfirmDelete {
            pane_id: self.focused_pane,
            target_name: target_name.clone(),
            permanent,
            warning_message,
        });
        self.status = if permanent {
            format!("confirm delete {target_name}: y/n")
        } else {
            format!("confirm trash {target_name}: y/n")
        };
    }

    /// 把永久刪除工作排入背景 task 執行，避免刪除大型目錄（數萬筆檔案）時凍結主 UI 執行緒。
    pub(crate) fn start_background_delete(
        &mut self,
        pane_id: usize,
        entries: Vec<crate::file_manager::entry::FileEntry>,
        target_name: String,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let task_id = self.push_task(
            pane_id,
            "delete",
            format!("delete {entry_count} item(s)"),
            format!("target: {target_name}"),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            None,
        );
        let mut busy_paths = Vec::new();
        let mut delete_targets = Vec::new();
        let deleted_names = entries.iter().map(|e| e.display_name()).collect::<Vec<_>>();

        for entry in &entries {
            busy_paths.push(entry.path.clone());
            delete_targets.push((entry.path.clone(), entry.is_dir));
        }
        self.active_file_job_busy_paths.insert(task_id, busy_paths);

        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.marked_paths.clear();
        }

        let total_bytes = entries
            .iter()
            .map(|e| e.directory_size.unwrap_or(e.size))
            .fold(0u64, u64::saturating_add);
        self.update_task_progress(task_id, 0, total_bytes.max(1));
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(200) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes.max(completed_bytes),
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };

            let mut failed_error = None;
            for (path, is_dir) in delete_targets {
                let res = if is_dir {
                    remove_dir_all_parallel_with_progress(&path, &mut progress)
                } else {
                    match remove_file_or_symlink_with_retry(&path) {
                        Ok(size) => {
                            progress(size);
                            Ok(())
                        }
                        Err(err) => Err(err),
                    }
                };
                if let Err(error) = res {
                    failed_error = Some(error);
                    break;
                }
            }
            send_progress_if_changed(
                &progress_sender,
                task_id,
                completed_bytes,
                total_bytes.max(completed_bytes),
                &mut last_progress,
            );
            let result = match failed_error {
                Some(error) => Err(error),
                None => Ok(deleted_names),
            };
            let _ = sender.send(FileJobEvent::Delete {
                task_id,
                target_name,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("deleting {entry_count} item(s) in background [task #{task_id}]");
        Ok(())
    }

    /// 真正執行將目前待確認項目移到 trash 或永久刪除的檔案系統操作。
    pub(crate) fn confirm_delete(
        &mut self,
        pane_id: usize,
        target_name: &str,
        permanent: bool,
    ) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        if permanent {
            let entries = {
                let pane = self
                    .panes
                    .get_mut(&pane_id)
                    .expect("checked pane existence before delete");
                pane.selected_or_marked_entries()
            };
            if entries.is_empty() {
                self.status = String::from("nothing selected to delete");
                return Ok(());
            }
            self.start_background_delete(pane_id, entries, target_name.to_string())?;
        } else {
            let trash_store = self.trash_store.clone();
            let trash_result = {
                let pane = self
                    .panes
                    .get_mut(&pane_id)
                    .expect("checked pane existence before trash");
                pane.trash_selected_or_marked(&trash_store)
            };
            match trash_result {
                Ok(trashed_names) if trashed_names.is_empty() => {
                    self.status = String::from("nothing selected to trash");
                }
                Ok(trashed_names) if trashed_names.len() == 1 => {
                    self.reload_all_panes()?;
                    self.status = format!("trashed {}", trashed_names[0]);
                }
                Ok(trashed_names) => {
                    self.reload_all_panes()?;
                    self.status = format!("trashed {} items", trashed_names.len());
                }
                Err(error) => self.status = format!("failed to trash {target_name}: {error}"),
            }
        }

        Ok(())
    }

    /// 還原最近一次放進 trash 的項目，並盡量在目前 pane 對焦到還原結果。
    pub(crate) fn restore_latest_from_trash(&mut self) -> io::Result<()> {
        match self.trash_store.restore_latest()? {
            Some(result) => {
                self.reload_all_panes()?;
                let _ = self.reveal_path_and_track(self.focused_pane, &result.restored_path);
                self.status = format!("restored {}", result.display_name);
            }
            None => {
                self.status = String::from("trash is empty");
            }
        }
        Ok(())
    }

    /// 執行使用者在確認視窗中同意的 trash 操作。
    pub(crate) fn confirm_trash_action(
        &mut self,
        action: TrashConfirmAction,
        target_name: String,
        entry_count: usize,
    ) -> io::Result<()> {
        match action {
            TrashConfirmAction::RestoreFromPanel {
                pane_id,
                target_ids,
                search,
                selected,
            } => self.restore_trash_ids_in_panel(
                pane_id,
                &target_ids,
                search,
                selected,
                &target_name,
                entry_count,
            ),
            TrashConfirmAction::DeleteFromPanel {
                pane_id,
                target_ids,
                search,
                selected,
            } => self.delete_trash_ids_in_panel(
                pane_id,
                &target_ids,
                search,
                selected,
                &target_name,
                entry_count,
            ),
        }
    }

    /// 在 trash 面板中批次還原指定 id 清單，並盡量保留原本搜尋上下文。
    pub(crate) fn restore_trash_ids_in_panel(
        &mut self,
        pane_id: usize,
        target_ids: &[String],
        search: PanelSearchState,
        selected: usize,
        target_name: &str,
        entry_count: usize,
    ) -> io::Result<()> {
        if target_ids.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }

        match self.trash_store.restore_many_by_ids(target_ids) {
            Ok(results) => {
                let _ = self.reload_all_panes();
                if let Some(first) = results.first() {
                    let _ = self.reveal_path_and_track(pane_id, &first.restored_path);
                }
                let _ = self.reopen_trash_panel_after_mutation(pane_id, search, selected);
                if results.is_empty() {
                    self.status = format!("trash item no longer exists: {target_name}");
                } else if entry_count <= 1 {
                    self.status = format!("restored {target_name}");
                } else {
                    self.status = format!("restored {} items", results.len());
                }
            }
            Err(error) => {
                let _ = self.reopen_trash_panel_after_mutation(pane_id, search, selected);
                self.status = format!("failed to restore from trash: {error}");
            }
        }
        Ok(())
    }

    /// 在 trash 面板中永久刪除指定 id 清單，並保留目前搜尋上下文。
    pub(crate) fn delete_trash_ids_in_panel(
        &mut self,
        pane_id: usize,
        target_ids: &[String],
        search: PanelSearchState,
        selected: usize,
        target_name: &str,
        entry_count: usize,
    ) -> io::Result<()> {
        if target_ids.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }

        match self.trash_store.delete_many_by_ids(target_ids) {
            Ok(deleted_names) => {
                let _ = self.reopen_trash_panel_after_mutation(pane_id, search, selected);
                let remaining_trash = self
                    .trash_store
                    .list_entries()
                    .map(|e| e.len())
                    .unwrap_or(0);
                if remaining_trash == 0 {
                    let _ = crate::file_manager::undo_backup::clear_undo_backup_dir();
                } else {
                    let _ = crate::file_manager::undo_backup::sync_delete_from_undo_backup(
                        &deleted_names,
                    );
                }

                if deleted_names.is_empty() {
                    self.status = format!("trash item no longer exists: {target_name}");
                } else if entry_count <= 1 {
                    self.status = format!("deleted permanently {target_name}");
                } else {
                    self.status = format!("deleted permanently {} items", deleted_names.len());
                }
            }
            Err(error) => {
                let _ = self.reopen_trash_panel_after_mutation(pane_id, search, selected);
                self.status = format!("failed to delete from trash: {error}");
            }
        }
        Ok(())
    }

    /// 在 trash 異動完成後重建面板狀態，避免游標跳到錯誤位置。
    pub(crate) fn reopen_trash_panel_after_mutation(
        &mut self,
        pane_id: usize,
        search: PanelSearchState,
        selected: usize,
    ) -> io::Result<()> {
        let visible_count = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
        let next_selected = if visible_count == 0 {
            0
        } else {
            selected.min(visible_count.saturating_sub(1))
        };
        self.pending_action = Some(PendingAction::TrashPanel {
            pane_id,
            selected: next_selected,
            search,
            marked_ids: Vec::new(),
            visual_anchor: None,
        });
        Ok(())
    }

    /// 從目前可見 trash 項目中，挑出批次操作真正要套用的目標清單。
    ///
    /// 規則：
    /// - 若已經有 `V` 選到的標記，就只處理那些標記。
    /// - 若目前沒有標記，則直接以搜尋結果中的全部項目當作目標。
    pub(crate) fn trash_panel_batch_entries<'a>(
        &self,
        entries: &'a [TrashListEntry],
        marked_ids: &[String],
    ) -> Vec<&'a TrashListEntry> {
        if marked_ids.is_empty() {
            return entries.iter().collect();
        }

        entries
            .iter()
            .filter(|entry| marked_ids.iter().any(|id| id == &entry.id))
            .collect()
    }

    /// 為一批 trash 項目產生確認視窗要顯示的名稱摘要。
    pub(crate) fn trash_confirm_target_name(entries: &[&TrashListEntry]) -> String {
        if entries.len() == 1 {
            entries[0].display_name.clone()
        } else {
            format!("{} items", entries.len())
        }
    }

    /// 回傳 trash 視覺選取狀態列文字。
    pub(crate) fn trash_visual_status_label(
        &self,
        selected: usize,
        anchor: usize,
        marked_count: usize,
    ) -> String {
        let range_count = anchor.abs_diff(selected) + 1;
        if marked_count == 0 {
            format!("trash visual: selecting {range_count} items")
        } else {
            format!("trash visual: selecting {range_count} items ({marked_count} marked)")
        }
    }

    /// 為目前 `trash` 面板挑出還原目標，並進入確認視窗。
    ///
    /// 規則：
    /// - 若已有 `V` 標記，`u` 會直接還原全部標記項目。
    /// - 若沒有標記，`u` 才會只還原游標所在的單筆項目。
    pub(crate) fn start_trash_panel_restore_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if !marked_ids.is_empty() {
            if selected_entries.is_empty() {
                self.status = String::from("trash is empty");
                return Ok(());
            }
            let target_name = Self::trash_confirm_target_name(&selected_entries);
            let entry_count = selected_entries.len();
            let action = TrashConfirmAction::RestoreFromPanel {
                pane_id,
                target_ids: selected_entries
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect(),
                search,
                selected,
            };
            self.pending_action = Some(PendingAction::ConfirmTrashAction {
                action: action.clone(),
                target_name: target_name.clone(),
                entry_count,
                marked_ids: marked_ids.to_vec(),
                visual_anchor,
            });
            self.status = trash_confirm_status(&action, &target_name, entry_count);
            return Ok(());
        }

        let Some(entry) = entries.get(selected) else {
            self.status = String::from("trash is empty");
            return Ok(());
        };
        let action = TrashConfirmAction::RestoreFromPanel {
            pane_id,
            target_ids: vec![entry.id.clone()],
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: entry.display_name.clone(),
            entry_count: 1,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &entry.display_name, 1);
        Ok(())
    }

    /// 為目前 `trash` 面板挑出批次還原目標，並進入確認視窗。
    ///
    /// 若已有 `V` 標記，會只操作標記的項目；否則會操作目前可見結果的全部項目。
    pub(crate) fn start_trash_panel_restore_all_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if selected_entries.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }
        let target_name = Self::trash_confirm_target_name(&selected_entries);
        let entry_count = selected_entries.len();
        let action = TrashConfirmAction::RestoreFromPanel {
            pane_id,
            target_ids: selected_entries
                .iter()
                .map(|entry| entry.id.clone())
                .collect(),
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: target_name.clone(),
            entry_count,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &target_name, entry_count);
        Ok(())
    }

    /// 為目前 `trash` 面板挑出永久刪除目標，並進入確認視窗。
    ///
    /// 規則：
    /// - 若已有 `V` 標記，`d` 會直接刪除全部標記項目。
    /// - 若沒有標記，`d` 才會只刪除游標所在的單筆項目。
    pub(crate) fn start_trash_panel_delete_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if !marked_ids.is_empty() {
            if selected_entries.is_empty() {
                self.status = String::from("trash is empty");
                return Ok(());
            }
            let target_name = Self::trash_confirm_target_name(&selected_entries);
            let entry_count = selected_entries.len();
            let action = TrashConfirmAction::DeleteFromPanel {
                pane_id,
                target_ids: selected_entries
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect(),
                search,
                selected,
            };
            self.pending_action = Some(PendingAction::ConfirmTrashAction {
                action: action.clone(),
                target_name: target_name.clone(),
                entry_count,
                marked_ids: marked_ids.to_vec(),
                visual_anchor,
            });
            self.status = trash_confirm_status(&action, &target_name, entry_count);
            return Ok(());
        }

        let Some(entry) = entries.get(selected) else {
            self.status = String::from("trash is empty");
            return Ok(());
        };
        let action = TrashConfirmAction::DeleteFromPanel {
            pane_id,
            target_ids: vec![entry.id.clone()],
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: entry.display_name.clone(),
            entry_count: 1,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &entry.display_name, 1);
        Ok(())
    }

    /// 為目前 `trash` 面板挑出批次永久刪除目標，並進入確認視窗。
    ///
    /// 若已有 `V` 標記，會只刪除標記項目；否則會刪除目前搜尋結果中的全部項目。
    pub(crate) fn start_trash_panel_delete_all_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if selected_entries.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }
        let target_name = Self::trash_confirm_target_name(&selected_entries);
        let entry_count = selected_entries.len();
        let action = TrashConfirmAction::DeleteFromPanel {
            pane_id,
            target_ids: selected_entries
                .iter()
                .map(|entry| entry.id.clone())
                .collect(),
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: target_name.clone(),
            entry_count,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &target_name, entry_count);
        Ok(())
    }

    /// 將目前選取項目放進內部剪貼簿，模式為複製。
    ///
    /// 參數：無。
    /// 回傳：`()`
    pub(crate) fn copy_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Copy);
    }

    /// 將目前選取項目放進內部剪貼簿，模式為剪下。
    ///
    /// 參數：無。
    /// 回傳：`()`
    pub(crate) fn cut_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Cut);
    }

    /// 清除目前內部剪貼簿中指定類型的 yank 狀態。
    ///
    /// 規則：
    /// - 若目前剪貼簿剛好就是指定操作類型，就清掉它。
    /// - 若目前不是該類型，則只更新狀態列，不動既有內容。
    pub(crate) fn clear_clipboard(&mut self, operation: ClipboardOperation) {
        match self.clipboard.as_ref() {
            Some(clipboard) if clipboard.operation == operation => {
                self.clipboard = None;
                self.status = match operation {
                    ClipboardOperation::Copy => String::from("cleared copied items"),
                    ClipboardOperation::Cut => String::from("cleared cut items"),
                };
            }
            _ => {
                self.status = match operation {
                    ClipboardOperation::Copy => String::from("no copied items to clear"),
                    ClipboardOperation::Cut => String::from("no cut items to clear"),
                };
            }
        }
    }

    /// 把目前焦點 pane 的選取項目寫入剪貼簿。
    ///
    /// 參數：
    /// - `operation: ClipboardOperation`，要記錄成複製或剪下。
    ///
    /// 回傳：`()`
    pub(crate) fn store_selected_in_clipboard(&mut self, operation: ClipboardOperation) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        };

        let entries: Vec<ClipboardEntry> = pane
            .selected_or_marked_entries()
            .into_iter()
            .map(|entry| {
                let display_name = entry.display_name();
                ClipboardEntry {
                    source_path: entry.path,
                    display_name,
                }
            })
            .collect();

        if entries.is_empty() {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        }

        let count = entries.len();
        self.clipboard = Some(ClipboardState { entries, operation });

        self.status = match operation {
            ClipboardOperation::Copy if count == 1 => String::from("copied 1 item"),
            ClipboardOperation::Copy => format!("copied {count} items"),
            ClipboardOperation::Cut if count == 1 => String::from("cut 1 item"),
            ClipboardOperation::Cut => format!("cut {count} items"),
        };
    }

    /// 將內部剪貼簿中的項目貼到目前有焦點的 pane 目錄。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表貼上流程已完成，並同步更新所有 pane。
    /// - 失敗時代表目標目錄或檔案系統操作失敗。
    pub(crate) fn paste_into_focused_pane(&mut self) -> io::Result<()> {
        self.paste_into_focused_pane_with_confirmation()
    }

    /// 將內部剪貼簿中的項目以覆蓋模式貼到目前有焦點的 pane 目錄。
    ///
    /// 規則：
    /// - 若目標名稱已存在，會直接覆蓋目標。
    /// - 若來源與目標本來就在同一路徑，會退回一般不覆蓋的行為避免覆寫自己。
    pub(crate) fn paste_into_focused_pane_with_overwrite(&mut self) -> io::Result<()> {
        self.paste_into_focused_pane_impl(true)
    }

    /// 先檢查目前貼上目標是否會和既有項目同名，必要時再打開覆蓋確認視窗。
    ///
    /// 規則：
    /// - 若沒有任何同名衝突，直接沿用一般貼上流程。
    /// - 若有同名衝突，先詢問是否要整批覆蓋。
    /// - 若來源與目標本來就是同一路徑，仍維持原本的 duplicate 命名策略，不視為衝突。
    ///
    /// 參數：
    /// - `self: &mut App`，目前應用程式狀態。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已直接貼上，或已打開確認視窗等待使用者決定。
    /// - 失敗時代表在檢查目標目錄是否衝突時發生 I/O 錯誤。
    pub(crate) fn paste_into_focused_pane_with_confirmation(&mut self) -> io::Result<()> {
        let Some(clipboard) = self.clipboard.as_ref() else {
            self.status = String::from("clipboard is empty");
            return Ok(());
        };
        let conflicts = self.paste_conflict_names()?;
        if conflicts.is_empty() {
            return self.paste_into_focused_pane_impl(false);
        }

        let target_name = if conflicts.len() == 1 {
            conflicts[0].clone()
        } else {
            format!("{} items", conflicts.len())
        };
        self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
            pane_id: self.focused_pane,
            target_name: target_name.clone(),
            entry_count: clipboard.entries.len(),
            operation: clipboard.operation,
        });
        self.status = paste_overwrite_confirm_status(&target_name, clipboard.entries.len());
        Ok(())
    }

    /// 負責實作一般貼上與覆蓋貼上的共用流程。
    pub(crate) fn paste_into_focused_pane_impl(&mut self, overwrite: bool) -> io::Result<()> {
        let Some(clipboard) = self.clipboard.clone() else {
            self.status = String::from("clipboard is empty");
            return Ok(());
        };

        let target_dir = match self.panes.get(&self.focused_pane) {
            Some(pane) => pane.cwd.clone(),
            None => {
                self.status = String::from("panel no longer exists");
                return Ok(());
            }
        };

        if paste_should_run_in_background(&clipboard, &target_dir) {
            return self.start_background_paste(
                self.focused_pane,
                target_dir,
                clipboard,
                overwrite,
            );
        }

        let mut pasted_count = 0usize;
        let mut history_items = Vec::new();
        for entry in &clipboard.entries {
            if entry.source_path.parent() == Some(target_dir.as_path())
                && clipboard.operation == ClipboardOperation::Cut
            {
                continue;
            }

            // 在真正執行前先保存目標名稱；失敗後檔案可能已被清理，不能再靠目錄內容
            // 推測目的地。這也確保同名複製時能顯示實際的 `copy` 名稱。
            let planned_target = self
                .panes
                .get(&self.focused_pane)
                .and_then(|pane| {
                    pane.planned_paste_target(&entry.source_path, overwrite)
                        .ok()
                })
                .unwrap_or_else(|| target_dir.join(&entry.display_name));

            let paste_result = match self.panes.get_mut(&self.focused_pane) {
                Some(pane) => match clipboard.operation {
                    ClipboardOperation::Copy => {
                        pane.copy_entry_with_history(&entry.source_path, overwrite)
                    }
                    ClipboardOperation::Cut => {
                        pane.move_entry_with_history(&entry.source_path, overwrite)
                    }
                },
                None => {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
            };

            let outcome = match paste_result {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.record_file_operation(clipboard.operation, history_items);
                    self.status =
                        paste_failure_status(&entry.display_name, &planned_target, &error);
                    return Ok(());
                }
            };

            pasted_count += 1;
            history_items.push(OperationItem {
                source_path: entry.source_path.clone(),
                destination_path: outcome.target_path,
                replaced_backup: outcome.backup_path,
            });
        }

        if pasted_count == 0 {
            self.status = String::from("nothing to paste into this directory");
            return Ok(());
        }

        self.reload_all_panes()?;
        self.status = paste_success_status(clipboard.operation, overwrite, pasted_count);

        if clipboard.operation == ClipboardOperation::Cut {
            self.clipboard = None;
        }
        self.record_file_operation(clipboard.operation, history_items);

        Ok(())
    }

    /// 把大型或網路目的地 paste 排入背景 task，避免傳輸期間凍結 TUI。
    ///
    /// 參數：
    /// - `pane_id: usize`，啟動貼上的目標 panel。
    /// - `target_dir: PathBuf`，實際目的目錄，可能是 UNC 或 macOS `/Volumes`。
    /// - `clipboard: ClipboardState`，本次固定使用的來源批次與 copy/cut 模式。
    /// - `overwrite: bool`，是否允許覆蓋同名項目。
    ///
    /// 回傳：`io::Result<()>`；成功表示工作已排入背景，完成結果稍後由主迴圈套用。
    pub(crate) fn start_background_paste(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        clipboard: ClipboardState,
        overwrite: bool,
    ) -> io::Result<()> {
        let entry_count = clipboard.entries.len();
        let operation = clipboard.operation;
        let operation_label = match operation {
            ClipboardOperation::Copy => "copy",
            ClipboardOperation::Cut => "move",
        };
        let task_id = self.push_task(
            pane_id,
            "paste",
            format!("{operation_label} {entry_count} item(s)"),
            format!("destination: {}", target_dir.display()),
            clipboard
                .entries
                .iter()
                .map(|entry| entry.source_path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let mut busy = Vec::new();
        for entry in &clipboard.entries {
            let item_name = entry
                .source_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| entry.display_name.trim_end_matches('/').to_string());
            busy.push(target_dir.join(&item_name));
            if clipboard.operation == ClipboardOperation::Cut {
                busy.push(entry.source_path.clone());
            }
        }
        self.active_file_job_busy_paths.insert(task_id, busy);
        // 背景 paste 一排入就顯示初始 byte，避免總大小尚未發現時 task 面板只有
        // RUNNING 而沒有任何進度資訊。後續事件會逐步修正已完成量與總量。
        self.update_task_progress(task_id, 0, 1);
        let (sender, receiver) = mpsc::channel();
        let worker_clipboard = clipboard.clone();
        let worker_target_dir = target_dir.clone();
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let progress_target_dir = worker_target_dir.clone();
            let mut completed_bytes = 0u64;
            let mut total_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now()
                .checked_sub(Duration::from_millis(500))
                .unwrap_or_else(Instant::now);
            let mut progress = |event: TransferProgress| {
                if event == TransferProgress::TargetVisible {
                    let _ = progress_sender.send(FileJobEvent::DestinationVisible {
                        target_dir: progress_target_dir.clone(),
                    });
                    return;
                }

                match event {
                    TransferProgress::BytesDiscovered(increment) => {
                        total_bytes = total_bytes.saturating_add(increment);
                    }
                    TransferProgress::BytesCopied(increment) => {
                        completed_bytes = completed_bytes.saturating_add(increment);
                    }
                    TransferProgress::TargetVisible => unreachable!(),
                }
                // 動態總量可能在相鄰檔案間持續增加；若每次都送到 App，task
                // history 會持續寫檔並反過來拖慢數十萬個小檔案的 copy。固定節流為
                // 每 500ms 最多一次，完成事件前仍會再送最後的精確值。
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result = perform_paste_job(
                &worker_clipboard,
                &worker_target_dir,
                overwrite,
                &mut progress,
            );

            let _ = sender.send(FileJobEvent::Progress {
                task_id,
                completed_bytes,
                total_bytes: total_bytes.max(completed_bytes),
            });
            let _ = sender.send(FileJobEvent::Paste {
                task_id,
                clipboard: worker_clipboard,
                overwrite,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!(
            "pasting {entry_count} item(s) in background to {} [task {task_id}]",
            target_dir.display()
        );
        Ok(())
    }

    /// 將已成功的貼上項目整理成一筆批次 Undo 歷史。
    ///
    /// 參數：
    /// - `operation: ClipboardOperation`，原操作是 Copy 或 Cut/Move。
    /// - `items: Vec<OperationItem>`，本批次真正成功的項目；部分失敗時可能少於剪貼簿。
    ///
    /// 回傳：`() `；空清單不會建立歷史。
    pub(crate) fn record_file_operation(
        &mut self,
        operation: ClipboardOperation,
        items: Vec<OperationItem>,
    ) {
        let kind = match operation {
            ClipboardOperation::Copy => FileOperationKind::Copy,
            ClipboardOperation::Cut => FileOperationKind::Move,
        };
        self.operation_history.push(FileOperation { kind, items });
    }

    /// 復原最近一次成功的 Copy 或 Move 批次，並同步刷新所有 panel。
    ///
    /// Copy 建立物會移到 PaneFM Trash；Move 會搬回原來源。可連續呼叫以依序復原最多
    /// 20 筆記憶體內歷史，重開程式後歷史會清空。
    ///
    /// 參數：`self: &mut App`，目前應用程式狀態。
    /// 回傳：`io::Result<()>`；檔案系統拒絕復原時保留失敗項目供下次重試。
    pub(crate) fn undo_latest_file_operation(&mut self) -> io::Result<()> {
        match self.operation_history.undo_latest(&self.trash_store) {
            Ok(None) => self.status = String::from("nothing to undo"),
            Ok(Some(result)) => {
                self.reload_all_panes()?;
                let action = match result.kind {
                    FileOperationKind::Copy => "copy",
                    FileOperationKind::Move => "move",
                };
                self.status = if result.failed == 0 {
                    format!("undid {action}: {} items", result.restored)
                } else {
                    format!(
                        "undo {action}: {} restored, {} failed; press u to retry",
                        result.restored, result.failed
                    )
                };
            }
            Err(error) => {
                self.status = format!("undo failed: {error}");
            }
        }
        Ok(())
    }

    /// 掃描這次貼上是否會和目前目標目錄中的既有項目同名。
    ///
    /// 規則：
    /// - 只有「直接同名」的目標才算衝突。
    /// - 若來源本身就位於同一個目錄，視為 duplicate copy / move 情境，不算衝突。
    ///
    /// 參數：
    /// - `self: &App`，目前應用程式狀態。
    ///
    /// 回傳：`io::Result<Vec<String>>`。
    /// - 成功時回傳所有會衝突的名稱清單。
    /// - 失敗時回傳取得目標 pane 或檢查檔案資訊時的錯誤。
    pub(crate) fn paste_conflict_names(&self) -> io::Result<Vec<String>> {
        let Some(clipboard) = self.clipboard.as_ref() else {
            return Ok(Vec::new());
        };
        let target_dir = self
            .panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "panel no longer exists"))?;

        let mut conflicts = Vec::new();
        for entry in &clipboard.entries {
            let Some(file_name) = entry.source_path.file_name() else {
                continue;
            };
            let direct_target = target_dir.join(file_name);
            let same_location = entry.source_path.parent() == Some(target_dir.as_path())
                && direct_target == entry.source_path;
            if !same_location && direct_target.exists() {
                conflicts.push(entry.display_name.clone());
            }
        }

        Ok(conflicts)
    }

    /// 在使用者確認後，以覆蓋模式完成這次貼上。
    ///
    /// 參數：
    /// - `self: &mut App`，目前應用程式狀態。
    /// - `pane_id: usize`，當初打開確認視窗時的目標 pane。
    /// - `target_name: String`，顯示在確認視窗中的名稱摘要。
    /// - `entry_count: usize`，這次整批貼上的項目數。
    /// - `operation: ClipboardOperation`，這批項目原本是 copy 還是 cut。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表覆蓋貼上已完成。
    /// - 失敗時代表貼上過程發生 I/O 錯誤。
    pub(crate) fn confirm_paste_overwrite(
        &mut self,
        pane_id: usize,
        target_name: String,
        entry_count: usize,
        operation: ClipboardOperation,
    ) -> io::Result<()> {
        if self.focused_pane != pane_id {
            self.focused_pane = pane_id;
        }
        if self
            .clipboard
            .as_ref()
            .is_none_or(|clipboard| clipboard.operation != operation)
        {
            self.status = format!("paste changed before overwrite: {target_name}");
            return Ok(());
        }
        let _ = entry_count;
        self.paste_into_focused_pane_impl(true)
    }

    /// 將目前選取或已標記的項目直接移動到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，目標目錄，可以是絕對路徑或相對於目前 pane 的路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_path(&mut self, target: &str) -> io::Result<()> {
        let Some(target_dir) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: move <target-dir>");
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 將目前選取或已標記的項目移到指定 pane 目前所在的目錄。
    ///
    /// 參數：
    /// - `target: &str`，目標 pane 編號字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_pane_id(&mut self, target: &str) -> io::Result<()> {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: move-panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        let Some(target_dir) = self.panes.get(&target_pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 將目前焦點 pane 的選取項目批次移動到目標目錄。
    pub(crate) fn move_selected_entries_into_dir(
        &mut self,
        target_dir: &std::path::Path,
    ) -> io::Result<()> {
        let Some(source_pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let source_dir = source_pane.cwd.clone();
        let entries = source_pane.selected_or_marked_entries();

        if entries.is_empty() {
            self.status = String::from("nothing selected to move");
            return Ok(());
        }

        if !target_dir.exists() {
            self.status = format!("move target does not exist: {}", target_dir.display());
            return Ok(());
        }
        if !target_dir.is_dir() {
            self.status = format!("move target is not a directory: {}", target_dir.display());
            return Ok(());
        }
        if source_dir == target_dir {
            self.status = String::from("move target is the current directory");
            return Ok(());
        }

        let clipboard = ClipboardState {
            operation: ClipboardOperation::Cut,
            entries: entries
                .iter()
                .map(|entry| ClipboardEntry {
                    source_path: entry.path.clone(),
                    display_name: entry.display_name(),
                })
                .collect(),
        };

        if paste_should_run_in_background(&clipboard, target_dir) {
            return self.start_background_paste(
                self.focused_pane,
                target_dir.to_path_buf(),
                clipboard,
                false,
            );
        }

        let mut moved_count = 0usize;
        let mut history_items = Vec::new();
        for entry in &entries {
            let outcome = match PaneState::move_path_to_dir_with_history(&entry.path, target_dir) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.operation_history.push(FileOperation {
                        kind: FileOperationKind::Move,
                        items: history_items,
                    });
                    self.status = format!("move failed for {}: {error}", entry.display_name());
                    return Ok(());
                }
            };
            moved_count += 1;
            history_items.push(OperationItem {
                source_path: entry.path.clone(),
                destination_path: outcome.target_path,
                replaced_backup: outcome.backup_path,
            });
        }

        self.operation_history.push(FileOperation {
            kind: FileOperationKind::Move,
            items: history_items,
        });
        self.reload_all_panes()?;
        self.status = if moved_count == 1 {
            format!("moved 1 item -> {}", target_dir.display())
        } else {
            format!("moved {moved_count} items -> {}", target_dir.display())
        };
        Ok(())
    }

    /// 將目前選取或已標記的項目複製到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，目標目錄，可以是絕對路徑或相對於目前 pane 的路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn copy_selected_to_path(&mut self, target: &str) -> io::Result<()> {
        let Some(target_dir) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: copy <target-dir>");
            return Ok(());
        };
        self.copy_selected_entries_into_dir(&target_dir)
    }

    /// 將目前選取或已標記的項目複製到指定 pane 目前所在的目錄。
    ///
    /// 參數：
    /// - `target: &str`，目標 pane 編號字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn copy_selected_to_pane_id(&mut self, target: &str) -> io::Result<()> {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: copy-panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        let Some(target_dir) = self.panes.get(&target_pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        self.copy_selected_entries_into_dir(&target_dir)
    }

    /// 將目前焦點 pane 的選取項目批次複製到目標目錄。
    pub(crate) fn copy_selected_entries_into_dir(
        &mut self,
        target_dir: &std::path::Path,
    ) -> io::Result<()> {
        let Some(source_pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let entries = source_pane.selected_or_marked_entries();

        if entries.is_empty() {
            self.status = String::from("nothing selected to copy");
            return Ok(());
        }

        if !target_dir.exists() {
            self.status = format!("copy target does not exist: {}", target_dir.display());
            return Ok(());
        }
        if !target_dir.is_dir() {
            self.status = format!("copy target is not a directory: {}", target_dir.display());
            return Ok(());
        }

        let clipboard = ClipboardState {
            operation: ClipboardOperation::Copy,
            entries: entries
                .iter()
                .map(|entry| ClipboardEntry {
                    source_path: entry.path.clone(),
                    display_name: entry.display_name(),
                })
                .collect(),
        };

        if paste_should_run_in_background(&clipboard, target_dir) {
            return self.start_background_paste(
                self.focused_pane,
                target_dir.to_path_buf(),
                clipboard,
                false,
            );
        }

        let mut copied_count = 0usize;
        let mut history_items = Vec::new();
        for entry in &entries {
            let outcome = match PaneState::copy_path_to_dir_with_history(&entry.path, target_dir) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.operation_history.push(FileOperation {
                        kind: FileOperationKind::Copy,
                        items: history_items,
                    });
                    self.status = format!("copy failed for {}: {error}", entry.display_name());
                    return Ok(());
                }
            };
            copied_count += 1;
            history_items.push(OperationItem {
                source_path: entry.path.clone(),
                destination_path: outcome.target_path,
                replaced_backup: outcome.backup_path,
            });
        }

        self.operation_history.push(FileOperation {
            kind: FileOperationKind::Copy,
            items: history_items,
        });
        self.reload_all_panes()?;
        self.status = if copied_count == 1 {
            format!("copied 1 item -> {}", target_dir.display())
        } else {
            format!("copied {copied_count} items -> {}", target_dir.display())
        };
        Ok(())
    }
}
