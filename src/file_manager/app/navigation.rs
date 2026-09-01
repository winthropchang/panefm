#![allow(unused_imports)]

use super::*;

impl App {
    /// 取得目前有焦點的 pane 可變參考。
    pub(crate) fn current_pane_mut(&mut self) -> io::Result<&mut PaneState> {
        self.panes
            .get_mut(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused panel"))
    }

    /// 將目前焦點 pane 依指定方向分割成兩個 pane。
    pub(crate) fn split_current(&mut self, direction: SplitDirection) -> io::Result<()> {
        self.split_current_at(direction, SplitPlacement::After)
    }

    /// 將目前焦點 pane 依指定方向與位置分割成兩個 pane。
    pub(crate) fn split_current_at(
        &mut self,
        direction: SplitDirection,
        placement: SplitPlacement,
    ) -> io::Result<()> {
        let source_pane = self
            .panes
            .get(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused panel"))?;
        let cwd = source_pane.cwd.clone();
        let show_hidden = source_pane.show_hidden;
        let sort_mode = source_pane.sort_mode;

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let mut pane = PaneState::new(cwd)?;
        pane.set_show_hidden(show_hidden);
        pane.set_sort_mode(sort_mode);
        self.panes.insert(new_id, pane);
        self.layout =
            self.layout
                .clone()
                .split_leaf(self.focused_pane, direction, placement, new_id);
        self.focused_pane = new_id;
        self.status = match (direction, placement) {
            (SplitDirection::Horizontal, SplitPlacement::Before) => String::from("split up"),
            (SplitDirection::Horizontal, SplitPlacement::After) => String::from("split down"),
            (SplitDirection::Vertical, SplitPlacement::Before) => String::from("split left"),
            (SplitDirection::Vertical, SplitPlacement::After) => String::from("split right"),
        };
        Ok(())
    }

    /// 依照目前布局順序取得所有 pane id。
    pub(crate) fn ordered_pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.layout.pane_ids(&mut ids);
        ids
    }

    /// 將焦點直接切到指定 pane 編號。
    pub(crate) fn focus_pane_by_id(&mut self, target_pane_id: usize) {
        if self.panes.contains_key(&target_pane_id) {
            self.focused_pane = target_pane_id;
            self.status = format!("focused panel {target_pane_id}");
        } else {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
        }
    }

    /// 從 `:panel <id>` 的參數解析目標 panel 編號並切換焦點。
    pub(crate) fn focus_pane_by_id_argument(&mut self, target: &str) {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return;
        };
        self.focus_pane_by_id(target_pane_id);
    }

    /// 關閉目前有焦點的 pane。
    pub(crate) fn close_current_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if ids.len() <= 1 {
            self.status = String::from("cannot close the last panel");
            return;
        }

        let old_focus = self.focused_pane;
        if let Some(index) = ids.iter().position(|id| *id == old_focus) {
            let fallback = if index > 0 {
                ids[index - 1]
            } else {
                ids[index + 1]
            };
            if let Some(layout) = self.layout.clone().close_pane(old_focus) {
                self.layout = layout;
                self.cancel_directory_load(old_focus);
                self.cancel_directory_size_scan(old_focus);
                self.panes.remove(&old_focus);
                if self
                    .global_search
                    .as_ref()
                    .is_some_and(|search| search.pane_id == old_focus)
                {
                    self.global_search = None;
                }
                self.focused_pane = fallback;
                self.status = format!("closed panel {old_focus}");
            }
        }
    }

    /// 僅保留目前有焦點的 pane，其餘全部關閉。
    pub(crate) fn only_current_pane(&mut self) {
        let focused = self.focused_pane;
        self.panes.retain(|id, _| *id == focused);
        self.layout = LayoutNode::Leaf { pane_id: focused };
        if self
            .global_search
            .as_ref()
            .is_some_and(|search| search.pane_id != focused)
        {
            self.global_search = None;
        }
        self.status = String::from("kept only focused panel");
    }

    /// 讓指定 panel 切換到目標路徑，並在成功後同步把最新目錄寫進 zoxide。
    ///
    /// 參數：
    /// - `pane_id: usize`，要操作的 panel 編號。
    /// - `target_path: &Path`，要切換或定位到的目標路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn go_to_path_and_track(
        &mut self,
        pane_id: usize,
        target_path: &Path,
    ) -> io::Result<()> {
        let Some(previous_cwd) = self.panes.get(&pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        if let Some((task_id, title, progress)) = self.active_file_job_for_path(target_path) {
            let pct_str = progress.map(|p| format!(" ({p}%)")).unwrap_or_default();
            self.status = format!(
                "cannot enter '{}': transfer in progress [task #{task_id}: {title}{pct_str}]",
                target_path.display()
            );
            return Ok(());
        }
        self.cancel_directory_load(pane_id);
        let current_cwd = {
            let pane = self.panes.get_mut(&pane_id).expect("panel checked above");
            pane.go_to_path(target_path)?;
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
            pane.cwd.clone()
        };
        self.restart_directory_size_scan_after_navigation(pane_id, &previous_cwd);
        self.zoxide_tracker.track(&current_cwd);
        Ok(())
    }

    /// 讓指定 panel 定位到某個檔案或目錄，並在成功後同步把結果目錄寫進 zoxide。
    ///
    /// 參數：
    /// - `pane_id: usize`，要操作的 panel 編號。
    /// - `target_path: &Path`，要被 reveal 的目標。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn reveal_path_and_track(
        &mut self,
        pane_id: usize,
        target_path: &Path,
    ) -> io::Result<()> {
        let Some(previous_cwd) = self.panes.get(&pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        self.cancel_directory_load(pane_id);
        let current_cwd = {
            let pane = self.panes.get_mut(&pane_id).expect("panel checked above");
            pane.reveal_path(target_path)?;
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
            pane.cwd.clone()
        };
        self.restart_directory_size_scan_after_navigation(pane_id, &previous_cwd);
        self.zoxide_tracker.track(&current_cwd);
        Ok(())
    }

    /// 把目前 focus 的 panel 工作目錄寫進 zoxide，供一般瀏覽操作完成後同步學習。
    ///
    /// 這個 helper 專門給 `h/l` 與方向鍵這類直接操作 pane 的流程使用，
    /// 因為它們不會經過 `go_to_path_and_track()` 這類包裝函式。
    pub(crate) fn track_focused_pane_cwd_in_zoxide(&self) {
        if let Some(pane) = self.panes.get(&self.focused_pane) {
            self.zoxide_tracker.track(&pane.cwd);
        }
    }

    /// 讓目前焦點 pane 直接跳到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，使用者在 command mode 輸入的目標路徑。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表目前 pane 已切到指定目錄，或定位到指定檔案。
    pub(crate) fn change_directory_from_command(&mut self, target: &str) -> io::Result<()> {
        if target.trim_start().starts_with("smb://") {
            return self.goto_smb_location(target.trim());
        }

        let Some(target_path) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: goto <path>");
            return Ok(());
        };

        if is_unc_path(target.trim()) {
            return self.start_network_goto(target_path);
        }

        match self.go_to_path_and_track(self.focused_pane, &target_path) {
            Ok(()) => {
                self.status = format!("jumped to path: {}", target_path.display());
            }
            Err(error) => {
                self.status = format!("path jump failed: {} ({error})", target_path.display());
            }
        }
        Ok(())
    }

    /// 讓目前焦點 pane 依 `goto smb://...` 進入指定的 SMB share；若尚未掛載則先請求系統掛載。
    pub(crate) fn goto_smb_location(&mut self, target: &str) -> io::Result<()> {
        self.goto_smb_location_with_mount_root(target, std::path::Path::new("/Volumes"))
    }

    /// 用指定掛載根目錄測試或進入 SMB share，方便在測試中模擬 macOS 的掛載點。
    pub(crate) fn goto_smb_location_with_mount_root(
        &mut self,
        target: &str,
        mount_root: &std::path::Path,
    ) -> io::Result<()> {
        #[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
        let _ = mount_root;

        let location = match parse_smb_location(target) {
            Ok(location) => location,
            Err(error) => {
                self.status = error.to_string();
                return Ok(());
            }
        };

        #[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
        let resolved = resolve_smb_location(&location);

        #[cfg(any(all(not(target_os = "windows"), not(target_os = "macos")), test))]
        let resolved = resolve_smb_location_with_mount_root(&location, mount_root);

        match resolved {
            ResolvedSmbLocation::Ready(path) => {
                if !path.exists() {
                    self.status = format!("smb path missing: {}", path.display());
                    return Ok(());
                }
                if !self.panes.contains_key(&self.focused_pane) {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
                self.go_to_path_and_track(self.focused_pane, &path)?;
                let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                };
                pane.set_bookmark_target(BookmarkTarget::SmbLocation(location.url.clone()));
                self.full_redraw_requested = true;
                self.status = format!("jumped to smb: {}", location.url);
            }
            ResolvedSmbLocation::NeedsMount { local_path } => {
                let launch = build_smb_mount_launch(&location);
                let task_id = self.push_task(
                    self.focused_pane,
                    "smb",
                    format!("mount {}", location.url),
                    format!("expected mount path: {}", local_path.display()),
                    vec![location.url.clone()],
                    Some(local_path.display().to_string()),
                );
                self.pending_launch = Some(QueuedLaunch { task_id, launch });
                self.status = format!(
                    "已請求系統掛載 SMB：{}；若系統連線失敗，請檢查主機、share 名稱、網路與權限，成功後再重試。預期掛載位置：{}",
                    location.url,
                    local_path.display()
                );
            }
        }
        Ok(())
    }

    /// 讓 `g` 系列快捷鍵可以快速跳到常用的系統目錄。
    ///
    /// 參數：
    /// - `directory: GoSpecialDirectory`，要跳去的預設目錄種類。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已切到目標目錄。
    /// - 若系統上不存在該目錄，會在狀態列顯示原因。
    pub(crate) fn go_to_special_directory(
        &mut self,
        directory: GoSpecialDirectory,
    ) -> io::Result<()> {
        let Some(target_path) = special_directory_path(directory) else {
            self.status = format!("{} not available on this system", directory.label());
            return Ok(());
        };

        if !target_path.exists() {
            self.status = format!("{} missing: {}", directory.label(), target_path.display());
            return Ok(());
        }

        match self.go_to_path_and_track(self.focused_pane, &target_path) {
            Ok(()) => {
                self.status = format!("jumped to {}: {}", directory.label(), target_path.display());
            }
            Err(error) => {
                self.status = format!(
                    "{} jump failed: {} ({error})",
                    directory.label(),
                    target_path.display()
                );
            }
        }
        Ok(())
    }

    /// 在背景執行 UNC 目錄跳轉，避免 Windows 等待失聯 SMB 主機時凍結 TUI。
    ///
    /// 參數：
    /// - `target_path: PathBuf`，`//server/share` 或 `\\server\share` 形式的目標。
    ///
    /// 回傳：`io::Result<()>`；成功代表工作已排入背景，不代表網路目錄已載入完成。
    pub(crate) fn start_network_goto(&mut self, target_path: PathBuf) -> io::Result<()> {
        self.start_network_goto_with(target_path, |mut pane, target| {
            pane.go_to_path(&target)?;
            Ok(pane)
        })
    }

    /// 以可注入 loader 啟動 UNC 背景跳轉，讓測試能證明主執行緒不會等待網路 I/O。
    ///
    /// 參數：
    /// - `target_path: PathBuf`，要交給背景工作載入的 UNC 路徑。
    /// - `loader: F`，取得目前 panel 副本與目標路徑，回傳載入後的 panel 狀態。
    ///
    /// 回傳：`io::Result<()>`；panel 不存在時回傳正常狀態並顯示錯誤，其餘情況會
    /// 立即回傳，loader 則留在背景執行。
    pub(crate) fn start_network_goto_with<F>(
        &mut self,
        target_path: PathBuf,
        loader: F,
    ) -> io::Result<()>
    where
        F: FnOnce(PaneState, PathBuf) -> io::Result<PaneState> + Send + 'static,
    {
        if self.active_network_goto_task_id.is_some() {
            self.cancel_network_goto("replaced by new goto");
        }

        let pane_id = self.focused_pane;
        let Some(pane) = self.panes.get(&pane_id).cloned() else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let task_id = self.push_task(
            pane_id,
            "goto",
            format!("goto {}", target_path.display()),
            String::from("loading UNC path in background"),
            Vec::new(),
            Some(target_path.display().to_string()),
        );
        let (sender, receiver) = mpsc::channel();
        let worker_target = target_path.clone();
        thread::spawn(move || {
            let result = loader(pane, worker_target.clone());
            let _ = sender.send(NetworkGotoEvent {
                task_id,
                pane_id,
                target: worker_target,
                result,
            });
        });

        self.network_goto_rx = Some(receiver);
        self.active_network_goto_task_id = Some(task_id);
        self.status = format!(
            "connecting to {} in background; Esc cancels",
            target_path.display()
        );
        Ok(())
    }

    /// 取消目前 UNC 背景跳轉並捨棄晚到的結果。
    ///
    /// 參數：`reason: &str`，寫入 task log 的取消原因。
    /// 回傳：`()`；作業系統中已開始的阻塞呼叫可能稍後才結束，但不再影響 UI。
    pub(crate) fn cancel_network_goto(&mut self, reason: &str) {
        self.network_goto_rx = None;
        if let Some(task_id) = self.active_network_goto_task_id.take() {
            self.finish_task(task_id, TaskState::Cancelled, reason.to_string());
        }
        self.status = String::from("network goto cancelled");
    }

    /// 將命令列中的路徑字串解析成實際可用的目標路徑。
    pub(crate) fn resolve_path_argument(&self, target: &str) -> Option<PathBuf> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return None;
        }

        let base_dir = self.panes.get(&self.focused_pane)?.cwd.clone();
        let expanded = expand_tilde_path(trimmed).unwrap_or_else(|| trimmed.to_string());
        let path = PathBuf::from(&expanded);
        Some(
            if path.is_absolute() || is_windows_drive_path(&expanded) || is_unc_path(&expanded) {
                path
            } else {
                base_dir.join(path)
            },
        )
    }

    /// 將目前可用的 pane 編號整理成易讀字串，供錯誤訊息與提示使用。
    pub(crate) fn available_pane_ids_label(&self) -> String {
        self.ordered_pane_ids()
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// 重新整理所有 pane，讓跨目錄操作後的內容保持同步。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表所有 pane 都已重新載入。
    /// - 失敗時代表至少有一個 pane 在重新讀取時發生錯誤。
    pub(crate) fn reload_all_panes(&mut self) -> io::Result<()> {
        for pane in self.panes.values_mut() {
            pane.reload()?;
        }
        for pane in self.panes.values() {
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
        }
        let size_panes = self
            .panes
            .iter()
            .filter(|(_, pane)| matches!(pane.active_detail_kind(), SortDetailKind::Size))
            .map(|(pane_id, _)| *pane_id)
            .collect::<Vec<_>>();
        for pane_id in size_panes {
            self.start_directory_size_scan(pane_id);
        }
        Ok(())
    }

    /// 重新載入正在顯示目的目錄或其子目錄的 panel，供背景貼上逐步更新列表。
    ///
    /// 參數：`directory: &Path`，背景工作開始寫入的目的根目錄。
    /// 回傳：`io::Result<()>`；任一位於目的樹內的 panel 載入失敗時回傳原始 I/O 錯誤。
    pub(crate) fn reload_panes_in_tree(&mut self, directory: &Path) -> io::Result<()> {
        for pane in self
            .panes
            .values_mut()
            .filter(|pane| pane.cwd == directory || pane.cwd.starts_with(directory))
        {
            pane.reload()?;
        }
        for pane in self
            .panes
            .values()
            .filter(|pane| pane.cwd == directory || pane.cwd.starts_with(directory))
        {
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
        }
        // 清理不在目前開啟 panel 中的陳舊子目錄快取
        self.directory_entry_cache.retain(|cached_path, _| {
            !cached_path.starts_with(directory)
                || self.panes.values().any(|pane| &pane.cwd == cached_path)
        });
        Ok(())
    }

    /// 將主題切換到下一個內建預設值。
    pub(crate) fn cycle_theme(&mut self) {
        let next = self.theme_preset.next();
        self.apply_theme(next);
    }

    /// 打開主題選擇視窗，並將選項焦點設在目前主題。
    pub(crate) fn open_theme_picker(&mut self) {
        let original = self.theme_preset;
        let selected = ThemePreset::ALL
            .iter()
            .position(|preset| *preset == original)
            .unwrap_or(0);
        self.pending_action = Some(PendingAction::ThemePicker { selected, original });
        self.status = String::from("theme picker: use j/k preview, l apply, h cancel");
    }

    /// 即時預覽主題列表目前選到的色盤，但不會寫入設定檔。
    ///
    /// 參數：
    /// - `selected: usize`，`ThemePreset::ALL` 中目前選取的索引。
    /// - `original: ThemePreset`，開啟列表前使用的主題，供取消操作時還原。
    ///
    /// 回傳：`()`，函數會更新畫面主題並保留主題列表狀態。
    pub(crate) fn preview_theme_picker_selection(
        &mut self,
        selected: usize,
        original: ThemePreset,
    ) {
        let preset = ThemePreset::ALL[selected];
        self.theme = preset.into();
        self.pending_action = Some(PendingAction::ThemePicker { selected, original });
        self.status = format!("theme preview: {}", preset.name());
    }

    /// 打開底部排序面板，等待使用者輸入排序快捷鍵。
    pub(crate) fn open_sort_picker(&mut self) {
        self.pending_action = Some(PendingAction::SortPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("sort: choose a key from the panel");
    }

    /// 打開底部 `g` 系列命令面板，供 `gg`、`gt` 等 leader 指令共用。
    pub(crate) fn open_go_picker(&mut self) {
        self.pending_action = Some(PendingAction::GoPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("go: choose g/t/d/k from the panel");
    }

    /// 打開底部 panel 操作面板，讓使用者可視化選擇 `w` 的第二個按鍵。
    pub(crate) fn open_window_picker(&mut self) {
        self.pending_action = Some(PendingAction::WindowPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("panel: choose h/j/k/l/c/o/t/d from the panel");
    }

    /// 打開底部 linemode 面板，等待使用者輸入右側欄位顯示模式。
    pub(crate) fn open_linemode_picker(&mut self) {
        self.pending_action = Some(PendingAction::LineModePicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("linemode: choose a key from the panel");
    }

    /// 打開書籤功能面板，列出目前可用的書籤操作。
    pub(crate) fn open_bookmark_picker(&mut self) {
        self.pending_action = Some(PendingAction::BookmarkPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("bookmark: choose a/g/d/D from the panel");
    }

    /// 打開 trash 面板，列出目前可還原的項目。
    pub(crate) fn open_trash_panel(&mut self) -> io::Result<()> {
        self.pending_action = Some(PendingAction::TrashPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
            marked_ids: Vec::new(),
            visual_anchor: None,
        });
        self.help_return = None;
        self.status = trash_panel_status("", self.trash_store.list_entries()?.len(), 0, false, 0);
        Ok(())
    }

    /// 打開 `t` 系列命令面板，讓使用者選擇主題或 Trash 功能。
    ///
    /// 參數：無，功能固定作用於目前取得焦點的 panel。
    /// 回傳：`()`, 只更新目前的互動狀態與提示文字。
    pub(crate) fn open_theme_command_picker(&mut self) {
        self.pending_action = Some(PendingAction::ThemeCommandPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("theme/trash: choose l/n/t/u from the panel");
    }

    /// 打開 F1 功能說明面板，支援 Vim 式滾動與面板內搜尋。
    pub(crate) fn open_help_panel(&mut self) {
        self.help_return = None;
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 打開 task 面板，查看目前 pane 最近執行過的任務與狀態。
    pub(crate) fn open_task_panel(&mut self) {
        let count = self.tasks_for_pane(self.focused_pane).len();
        self.pending_action = Some(PendingAction::TaskPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = task_panel_status("", count, 0, false);
    }

    /// 打開全螢幕 N 路目錄與檔案差異比對工作區 (Diff Matrix)。
    pub(crate) fn open_diff_matrix(&mut self, target_pane_ids: Option<Vec<usize>>) -> Result<()> {
        let pane_ids = match target_pane_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => self.panes.keys().copied().collect::<Vec<_>>(),
        };

        if pane_ids.len() < 2 {
            self.status = String::from(
                "diff requires at least 2 open panels (e.g. use Ctrl+s / Ctrl+v to split first)",
            );
            return Ok(());
        }

        let mut valid_ids = Vec::new();
        let mut roots = Vec::new();
        let mut labels = Vec::new();

        for id in pane_ids {
            if let Some(pane) = self.panes.get(&id) {
                valid_ids.push(id);
                roots.push(pane.cwd.clone());
                let tail = pane
                    .cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| pane.cwd.display().to_string());
                labels.push(tail);
            }
        }

        if roots.len() < 2 {
            self.status = String::from("diff requires at least 2 valid panels");
            return Ok(());
        }

        // 取消任何先前的 diff background job
        if let Some(cancelled) = self.diff_job_cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        self.diff_job_cancelled = Some(cancelled.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        self.diff_job_rx = Some(rx);

        spawn_background_diff(roots.clone(), true, true, cancelled, tx);

        let diff_state = DiffMatrixState::new_loading(valid_ids, roots, labels);
        self.pending_action = Some(PendingAction::DiffMatrix(diff_state));
        self.status = format!("diff matrix: scanning {} panels...", self.panes.len());
        Ok(())
    }

    /// 輪詢背景目錄比對工作的接收端，非阻塞更新差異矩陣。
    pub(crate) fn poll_diff_job(&mut self) {
        let Some(receiver) = &self.diff_job_rx else {
            return;
        };
        let messages: Vec<DiffJobEvent> = receiver.try_iter().collect();
        if messages.is_empty() {
            return;
        }

        for message in messages {
            match message {
                DiffJobEvent::Discovered(count) => {
                    if let Some(PendingAction::DiffMatrix(state)) = &mut self.pending_action {
                        state.discovered_count = count;
                    }
                }
                DiffJobEvent::Done(rows) => {
                    let count = rows.len();
                    if let Some(PendingAction::DiffMatrix(state)) = &mut self.pending_action {
                        state.set_completed_rows(rows);
                        self.status =
                            format!("diff matrix: compared {} items (press q to exit)", count);
                    }
                    self.diff_job_rx = None;
                    self.diff_job_cancelled = None;
                    break;
                }
                DiffJobEvent::Error(err) => {
                    self.status = format!("diff error: {err}");
                    self.diff_job_rx = None;
                    self.diff_job_cancelled = None;
                    break;
                }
            }
        }
    }

    /// 打開書籤列表彈窗，讓使用者可以用列表方式跳轉既有書籤。
    pub(crate) fn open_bookmark_list(&mut self) {
        self.open_bookmark_list_with_mode(self.focused_pane, BookmarkListMode::Jump);
    }

    /// 以指定模式打開書籤列表彈窗，供跳轉或刪除流程共用。
    pub(crate) fn open_bookmark_list_with_mode(&mut self, pane_id: usize, mode: BookmarkListMode) {
        self.pending_action = Some(PendingAction::BookmarkList {
            pane_id,
            selected: 0,
            mode,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = bookmark_list_status("", self.bookmark_store.list().len(), 0, mode, false);
    }

    /// 打開 zoxide 目錄列表，讓目前 panel 可依 frecency 快速跳到常用目錄。
    ///
    /// 這個面板是 `:zoxide` 的正式入口，`Z` 也會走這裡。
    pub(crate) fn open_zoxide_list(&mut self) {
        match query_zoxide_directories() {
            Ok(entries) => {
                let count = entries.len();
                self.pending_action = Some(PendingAction::ZoxideList {
                    pane_id: self.focused_pane,
                    selected: 0,
                    entries,
                    search: PanelSearchState {
                        buffer: String::new(),
                        editing: false,
                    },
                });
                self.status = zoxide_list_status("", count, 0, false);
            }
            Err(error) => {
                self.status = format!("zoxide failed: {error}");
                self.open_tool_panel();
            }
        }
    }

    /// 在目前 focus panel 顯示外部工具安裝狀態，讓使用者知道缺少哪些依賴。
    pub(crate) fn open_tool_panel(&mut self) {
        self.pending_action = Some(PendingAction::ToolPanel {
            pane_id: self.focused_pane,
            selected: 0,
        });
        self.status = String::from("dependencies: j/k move, Esc close");
    }

    /// 將目前焦點 pane 的目錄存成書籤。
    pub(crate) fn set_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let target = pane.bookmark_target.clone();
        match &target {
            BookmarkTarget::LocalPath(path) => {
                self.bookmark_store
                    .set(key, path.clone())
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.status = format!("bookmark [{key}] = {}", path.display());
            }
            BookmarkTarget::SmbLocation(location) => {
                self.bookmark_store
                    .set_smb(key, location.clone())
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.status = format!("bookmark [{key}] = {}", target.display_text());
            }
        }
        Ok(())
    }

    /// 自動挑選下一個可用書籤代號，並把指定 pane 目前位置存成書籤。
    ///
    /// 參數：
    /// - `pane_id: usize`，要儲存位置的 pane 編號。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn add_bookmark_with_auto_key(&mut self, pane_id: usize) -> io::Result<()> {
        let Some(key) = self.bookmark_store.next_available_key() else {
            self.status = String::from("bookmark: no available auto key");
            return Ok(());
        };

        self.focused_pane = pane_id;
        self.set_bookmark(key)
    }

    /// 跳到指定書籤對應的路徑。
    pub(crate) fn jump_to_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(target) = self.bookmark_store.get(key).cloned() else {
            self.status = format!("bookmark [{key}] not found");
            return Ok(());
        };

        self.jump_to_bookmark_target(self.focused_pane, key, &target)
    }

    /// 讓 `:bookmark jump <key>` 可以直接跳到指定書籤。
    pub(crate) fn jump_to_bookmark_from_command(&mut self, args: &str) -> io::Result<()> {
        let Some(key) = parse_bookmark_argument(args) else {
            self.status = String::from("usage: bookmark jump <key>");
            return Ok(());
        };
        self.jump_to_bookmark(key)
    }

    /// 刪除指定代號的單一書籤，並同步更新狀態列。
    ///
    /// 參數：
    /// - `key: char`，要刪除的書籤代號。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn delete_bookmark(&mut self, key: char) -> io::Result<()> {
        let removed = self
            .bookmark_store
            .remove(key)
            .map_err(|error| io::Error::other(error.to_string()))?;

        if removed {
            self.status = format!("bookmark [{key}] deleted");
        } else {
            self.status = format!("bookmark [{key}] not found");
        }
        Ok(())
    }

    /// 根據目前書籤列表選取位置刪除單一書籤。
    ///
    /// 參數：
    /// - `entries: &[BookmarkEntry]`，目前列表中可見的書籤資料。
    /// - `selected: usize`，目前游標指到的列索引。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn delete_bookmark_from_list(
        &mut self,
        entries: &[BookmarkEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("bookmark delete: empty");
            return Ok(());
        };

        self.delete_bookmark(entry.key)
    }

    /// 清空全部書籤，並同步寫回 `bookmark.toml`。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn delete_all_bookmarks(&mut self) -> io::Result<()> {
        self.bookmark_store
            .clear()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.status = String::from("all bookmarks deleted");
        Ok(())
    }

    /// 讓 `:linemode <mode>` 可以直接切換目前 pane 的右側欄位顯示模式。
    ///
    /// 支援：
    /// - `size`
    /// - `permissions`
    /// - `btime`
    /// - `mtime`
    /// - `none`
    pub(crate) fn apply_line_mode_from_command(&mut self, args: &str) -> io::Result<()> {
        let line_mode = match args {
            "size" => LineMode::Size,
            "permissions" => LineMode::Permissions,
            "btime" => LineMode::Btime,
            "mtime" => LineMode::Mtime,
            "none" => LineMode::None,
            _ => {
                self.status = String::from("usage: linemode <size|permissions|btime|mtime|none>");
                return Ok(());
            }
        };

        self.apply_line_mode(self.focused_pane, line_mode)
    }

    /// 依書籤目標型別跳到本機目錄，或自動發起 SMB 掛載／進入流程。
    pub(crate) fn jump_to_bookmark_target(
        &mut self,
        pane_id: usize,
        key: char,
        target: &BookmarkTarget,
    ) -> io::Result<()> {
        self.jump_to_bookmark_target_with_mount_root(
            pane_id,
            key,
            target,
            std::path::Path::new("/Volumes"),
        )
    }

    /// 依書籤目標型別跳到本機目錄，或用指定掛載根目錄自動發起 SMB 掛載／進入流程。
    pub(crate) fn jump_to_bookmark_target_with_mount_root(
        &mut self,
        pane_id: usize,
        key: char,
        target: &BookmarkTarget,
        mount_root: &std::path::Path,
    ) -> io::Result<()> {
        self.focused_pane = pane_id;

        match target {
            BookmarkTarget::LocalPath(path) => {
                if !self.panes.contains_key(&pane_id) {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
                if !path.exists() {
                    self.status = format!("bookmark [{key}] missing: {}", path.display());
                    return Ok(());
                }
                self.go_to_path_and_track(pane_id, path)?;
                self.status = format!("jumped to bookmark [{key}]");
            }
            BookmarkTarget::SmbLocation(location) => {
                self.goto_smb_location_with_mount_root(location, mount_root)?;
                if self.status.starts_with("jumped to smb:") {
                    self.status = format!("jumped to bookmark [{key}]");
                } else if self.status.starts_with("已請求系統掛載 SMB：") {
                    self.status = format!("bookmark [{key}] 正在連線：{location}");
                }
            }
        }

        Ok(())
    }

    /// 從書籤列表彈窗中打開目前選取的書籤。
    pub(crate) fn open_bookmark_from_list(
        &mut self,
        pane_id: usize,
        entries: &[BookmarkEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("bookmark jump: empty");
            return Ok(());
        };
        let Some(target) = self.bookmark_store.get(entry.key).cloned() else {
            self.status = format!("bookmark [{}] not found", entry.key);
            return Ok(());
        };
        self.jump_to_bookmark_target(pane_id, entry.key, &target)
    }

    /// 從 zoxide 列表中打開目前選取的目錄。
    pub(crate) fn open_zoxide_from_list(
        &mut self,
        pane_id: usize,
        entries: &[PathBuf],
        selected: usize,
    ) -> io::Result<()> {
        let Some(target_path) = entries.get(selected).cloned() else {
            self.status = String::from("zoxide: empty");
            return Ok(());
        };

        self.go_to_path_and_track(pane_id, &target_path)?;
        self.focused_pane = pane_id;
        self.status = format!("jumped via zoxide: {}", target_path.display());
        Ok(())
    }

    /// 以目前正在操作的上下文為返回點，打開 help 面板。
    pub(crate) fn open_help_from_current(&mut self) {
        self.help_return = self.capture_help_return_state();
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 擷取目前互動狀態，供 help 面板關閉後回復。
    pub(crate) fn capture_help_return_state(&mut self) -> Option<HelpReturnState> {
        if let Some(action) = self.pending_action.take() {
            return Some(HelpReturnState::Pending(action));
        }
        if let Some(filter) = self.filter.take() {
            return Some(HelpReturnState::Filter(filter));
        }
        if let Some(search) = self.preview_search.take() {
            return Some(HelpReturnState::PreviewSearch(search));
        }
        if let Some(search) = self.list_find.take() {
            return Some(HelpReturnState::ListFind(search));
        }
        if let Some(search) = self.global_search.take() {
            self.cancel_global_search();
            return Some(HelpReturnState::GlobalSearch(search));
        }
        if let Some(selection) = self.visual_selection.take() {
            return Some(HelpReturnState::VisualSelection(selection));
        }
        if self.command_mode {
            self.command_mode = false;
            self.command_completion_cycle = None;
            return Some(HelpReturnState::CommandMode(std::mem::take(
                &mut self.command_buffer,
            )));
        }
        if let Some(prompt) = self.pending_bookmark.take() {
            return Some(HelpReturnState::PendingBookmark(prompt));
        }
        if let Some(pane) = self.panes.get_mut(&self.focused_pane)
            && pane.is_preview_active()
        {
            pane.set_preview_active(false);
            return Some(HelpReturnState::PreviewFocus(self.focused_pane));
        }
        None
    }

    /// 從 help 面板回到先前的互動上下文。
    pub(crate) fn restore_help_return_state(&mut self, preserve_status: bool) -> io::Result<()> {
        let previous_status = self.status.clone();
        let Some(state) = self.help_return.take() else {
            self.status = String::from("normal mode");
            return Ok(());
        };

        match state {
            HelpReturnState::Pending(action) => {
                self.status = self.status_for_pending_action(&action)?;
                self.pending_action = Some(action);
            }
            HelpReturnState::Filter(filter) => {
                self.status = if filter.editing {
                    if filter.buffer.is_empty() {
                        String::from("filter: all")
                    } else {
                        format!("filter: {}", filter.buffer)
                    }
                } else if filter.buffer.is_empty() {
                    String::from("filter active")
                } else {
                    format!("filter locked: {}", filter.buffer)
                };
                self.filter = Some(filter);
            }
            HelpReturnState::PreviewSearch(search) => {
                self.status =
                    preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
                self.preview_search = Some(search);
            }
            HelpReturnState::ListFind(search) => {
                self.status =
                    list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
                self.list_find = Some(search);
            }
            HelpReturnState::GlobalSearch(search) => {
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            HelpReturnState::VisualSelection(selection) => {
                self.visual_selection = Some(selection);
                self.status = self.visual_status_label();
            }
            HelpReturnState::CommandMode(buffer) => {
                self.open_prefilled_command(buffer);
            }
            HelpReturnState::PendingBookmark(prompt) => {
                self.pending_bookmark = Some(prompt);
                self.status = match prompt {
                    BookmarkPrompt::Jump => String::from("bookmark: press a key to jump"),
                };
            }
            HelpReturnState::PreviewFocus(pane_id) => {
                if let Some(pane) = self.panes.get_mut(&pane_id) {
                    pane.set_preview_active(true);
                    self.status = String::from("preview mode");
                } else {
                    self.status = String::from("panel no longer exists");
                }
            }
        }

        if preserve_status {
            self.status = previous_status;
        }

        Ok(())
    }

    /// 依 pending action 類型回傳適合顯示的狀態文字。
    pub(crate) fn status_for_pending_action(&self, action: &PendingAction) -> io::Result<String> {
        Ok(match action {
            PendingAction::ConfirmDelete {
                target_name,
                permanent,
                ..
            } => {
                if *permanent {
                    format!("confirm delete {target_name}: y/n")
                } else {
                    format!("confirm trash {target_name}: y/n")
                }
            }
            PendingAction::ConfirmPasteOverwrite {
                target_name,
                entry_count,
                ..
            } => paste_overwrite_confirm_status(target_name, *entry_count),
            PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                ..
            } => trash_confirm_status(action, target_name, *entry_count),
            PendingAction::GoPicker { .. } => String::from("go: choose g/t/d/k from the panel"),
            PendingAction::ThemeCommandPicker { .. } => {
                String::from("theme/trash: choose l/n/t/u from the panel")
            }
            PendingAction::SortPicker { .. } => String::from("sort: choose a key from the panel"),
            PendingAction::WindowPicker { .. } => {
                String::from("panel: choose h/j/k/l/c/o/t/d from the panel")
            }
            PendingAction::LineModePicker { .. } => {
                String::from("linemode: choose a key from the panel")
            }
            PendingAction::ThemePicker { selected, .. } => {
                format!("theme picker: {}", ThemePreset::ALL[*selected].name())
            }
            PendingAction::TaskPanel {
                pane_id,
                selected,
                search,
            } => {
                let filtered =
                    filtered_task_entries(&self.tasks_for_pane(*pane_id), &search.buffer);
                task_panel_status(&search.buffer, filtered.len(), *selected, search.editing)
            }
            PendingAction::BookmarkPicker { .. } => {
                String::from("bookmark: choose a/g/d/D from the panel")
            }
            PendingAction::TrashPanel {
                selected,
                search,
                marked_ids,
                ..
            } => {
                let visible = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
                trash_panel_status(
                    &search.buffer,
                    visible,
                    *selected,
                    search.editing,
                    marked_ids.len(),
                )
            }
            PendingAction::HelpPanel { search, .. } => help_panel_status(
                &search.buffer,
                help_entries(&search.buffer).len(),
                search.editing,
            ),
            PendingAction::ToolPanel { .. } => String::from("dependencies: j/k move, Esc close"),
            PendingAction::BookmarkList {
                selected,
                mode,
                search,
                ..
            } => {
                let filtered =
                    filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer);
                bookmark_list_status(
                    &search.buffer,
                    filtered.len(),
                    *selected,
                    *mode,
                    search.editing,
                )
            }
            PendingAction::ZoxideList {
                entries,
                selected,
                search,
                ..
            } => {
                let filtered = filtered_zoxide_entries(entries, &search.buffer);
                zoxide_list_status(&search.buffer, filtered.len(), *selected, search.editing)
            }
            PendingAction::CopyPicker { target, .. } => {
                format!("copy to clipboard: {}", target.display_name)
            }
            PendingAction::OpenPicker { target, .. } => {
                format!("open with: {}", target.display_name)
            }
            PendingAction::Rename { mode, .. } => match mode {
                RenameMode::Insert => String::from("rename: insert"),
                RenameMode::Normal => String::from("rename: normal"),
            },
            PendingAction::CreateEntry { mode, .. } => match mode {
                RenameMode::Insert => create_status_label("insert"),
                RenameMode::Normal => create_status_label("normal"),
            },
            PendingAction::RegexRename {
                previews,
                pattern,
                replacement,
                ..
            } => regex_rename_status(pattern, replacement, previews),
            PendingAction::DiffMatrix(state) => {
                format!(
                    "diff matrix: {} items (filter: {})",
                    state.filtered_indices.len(),
                    state.filter_mode.label()
                )
            }
        })
    }

    /// 依照主題名稱字串套用指定主題。
    pub(crate) fn set_theme_by_name(&mut self, name: &str) {
        match ThemePreset::from_name(name) {
            Some(preset) => self.apply_theme(preset),
            None => {
                let available = ThemePreset::ALL
                    .iter()
                    .map(|preset| preset.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.status = format!("unknown theme: {name}. available: {available}");
            }
        }
    }

    /// 直接套用指定的主題預設值。
    pub(crate) fn apply_theme(&mut self, preset: ThemePreset) {
        self.theme_preset = preset;
        self.theme = preset.into();
        self.config.ui.theme_preset = preset;
        match persist_theme(&self.config_source, preset) {
            Ok(()) => {
                self.status = format!("theme: {}", preset.name());
            }
            Err(error) => {
                self.status = format!("theme: {} (save failed: {error})", preset.name());
            }
        }
    }

    /// 打開 filter 輸入框，並以目前焦點 pane 作為過濾目標。
    pub(crate) fn open_filter_input(&mut self, mode: FilterMode) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let filter = FilterState {
            pane_id: self.focused_pane,
            buffer: String::new(),
            editing: true,
            mode,
        };
        self.apply_filter_buffer(&filter);
        self.status = format_filter_status(&filter);
        self.filter = Some(filter);
    }

    /// 打開 global search 面板，遞迴建立目前目錄下的搜尋候選資料集。
    pub(crate) fn open_global_search(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = GlobalSearchState {
            pane_id: self.focused_pane,
            root_dir: pane.cwd.clone(),
            mode: SearchMode::Path,
            buffer: String::new(),
            editing: true,
            loading: false,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        };
        self.status = global_search_status(search.mode, "", 0, true, false, false);
        self.global_search = Some(search);
        self.cancel_global_search_worker();
        Ok(())
    }

    /// 打開內容搜尋面板，遞迴搜尋目前目錄下所有檔案內容。
    pub(crate) fn open_content_search(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = GlobalSearchState {
            pane_id: self.focused_pane,
            root_dir: pane.cwd.clone(),
            mode: SearchMode::Content,
            buffer: String::new(),
            editing: true,
            loading: false,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        };
        self.status = global_search_status(search.mode, "", 0, true, false, false);
        self.global_search = Some(search);
        self.cancel_global_search_worker();
        Ok(())
    }

    /// 切換目前焦點 panel 自己的 preview mode。
    ///
    /// 參數：
    /// - `self: &mut App`，包含 panel 集合與目前焦點的應用程式狀態。
    ///
    /// 回傳：`()`；只切換 `focused_pane` 對應的 `PaneState::preview_active`，其他
    /// panel 已開啟的 preview 會保持原狀。若焦點 panel 已不存在，會在狀態列顯示錯誤。
    pub(crate) fn open_preview_focus(&mut self) {
        let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return;
        };
        let preview_active = pane.toggle_preview_active();
        self.pending_g = false;
        self.pending_y = false;
        self.status = if preview_active {
            String::from("preview mode")
        } else {
            String::from("normal mode")
        };
    }

    /// 打開 preview search 輸入框，並清空上一次的搜尋字串。
    pub(crate) fn open_preview_search_input(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = PreviewSearchState {
            pane_id: self.focused_pane,
            buffer: String::new(),
            editing: true,
        };
        self.apply_preview_search_buffer(&search);
        self.status =
            preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
        self.preview_search = Some(search);
    }

    /// 打開目前 pane 的列表內 find-next 輸入框，並沿用目前已存在的查詢字串。
    pub(crate) fn open_list_find_input(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = ListFindState {
            pane_id: self.focused_pane,
            buffer: String::new(),
        };
        self.apply_list_find_buffer(&search);
        self.status = list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
        self.list_find = Some(search);
    }

    /// 使用 `fzf` 遞迴掃描目前 pane 的目錄樹，快速挑選任意深度的目標。
    pub(crate) fn open_fzf_jump(&mut self) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("jump failed: panel not found");
            return;
        };

        if !pane.cwd.is_dir() {
            self.status = String::from("jump failed: current root is not a directory");
            return;
        }

        let root_dir = pane.cwd.clone();
        let task_id = self.push_task(
            self.focused_pane,
            "jump",
            format!("fzf jump in {}", root_dir.display()),
            String::from("waiting for fzf"),
            vec![root_dir.display().to_string()],
            None,
        );
        self.pending_fzf_jump = Some(FzfJumpRequest {
            pane_id: self.focused_pane,
            root_dir,
            show_hidden: true,
            follow_links: self.config.search.fzf_follow_links,
            task_id,
        });
        self.status = String::from("jump: fzf loading");
    }

    /// 套用 `fzf` 選取結果；取消時保留原本列表狀態。
    pub(crate) fn apply_fzf_jump_selection(
        &mut self,
        request: FzfJumpRequest,
        selected_line: Option<&str>,
    ) {
        let Some(line) = selected_line.map(str::trim).filter(|line| !line.is_empty()) else {
            self.finish_task(
                request.task_id,
                TaskState::Cancelled,
                String::from("fzf cancelled"),
            );
            self.status = String::from("jump cancelled");
            return;
        };

        if !self.panes.contains_key(&request.pane_id) {
            self.finish_task(
                request.task_id,
                TaskState::Failed,
                String::from("panel no longer exists"),
            );
            self.status = String::from("jump failed: panel no longer exists");
            return;
        }

        let target_path = jump_selection_to_path(&request.root_dir, line);
        let go_to_path_started_at = Instant::now();
        match self.go_to_path_and_track(request.pane_id, &target_path) {
            Ok(()) => {
                debug_timing_message(&format!("jump target path: {}", target_path.display()));
                debug_timing_log("jump go_to_path", go_to_path_started_at);
                self.finish_task(request.task_id, TaskState::Done, format!("opened {line}"));
                self.status = format!("jumped: {line}");
            }
            Err(error) => {
                debug_timing_log("jump go_to_path (failed)", go_to_path_started_at);
                self.finish_task(request.task_id, TaskState::Failed, error.to_string());
                self.status = format!("jump failed for {line}: {error}");
            }
        }
    }

    /// 切換目前焦點 pane 的隱藏檔顯示狀態。
    pub(crate) fn toggle_hidden_files(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        pane.toggle_hidden();
        self.status = if pane.show_hidden {
            String::from("showing hidden files")
        } else {
            String::from("hiding hidden files")
        };
        Ok(())
    }

    /// 將指定 pane 套用某一種排序模式。
    pub(crate) fn apply_sort_mode(
        &mut self,
        pane_id: usize,
        sort_mode: SortMode,
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        pane.set_sort_mode(sort_mode);
        let needs_directory_sizes = matches!(pane.active_detail_kind(), SortDetailKind::Size);
        self.status = format!("sort: {}", pane.sort_mode.label());
        if needs_directory_sizes {
            self.start_directory_size_scan(pane_id);
        } else {
            self.cancel_directory_size_scan(pane_id);
        }
        Ok(())
    }

    /// 套用指定 pane 的 linemode，只更新右側欄位顯示方式，不改動原本排序順序。
    ///
    /// 參數：
    /// - `pane_id: usize`，要被套用 linemode 的 pane 編號。
    /// - `line_mode: LineMode`，要切換成的右側欄位模式。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表 linemode 已套用完成。
    /// - 若目標 pane 已不存在，會改寫狀態列並直接結束。
    pub(crate) fn apply_line_mode(
        &mut self,
        pane_id: usize,
        line_mode: LineMode,
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        pane.set_line_mode(line_mode);
        let needs_directory_sizes = matches!(pane.active_detail_kind(), SortDetailKind::Size);
        self.status = format!("linemode: {}", line_mode.label());
        if needs_directory_sizes {
            self.start_directory_size_scan(pane_id);
        } else {
            self.cancel_directory_size_scan(pane_id);
        }
        Ok(())
    }
}
