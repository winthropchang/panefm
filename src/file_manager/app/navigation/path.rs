//! 目錄跳轉、歷史路徑、Zoxide 記錄、SMB/網路掛載與重整。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use crate::file_manager::smb;

use super::super::*;

impl App {
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
            let cwd = pane.cwd.clone();
            let entries = pane.entries.clone();
            self.store_directory_cache(cwd.clone(), entries);
            cwd
        };
        self.restart_directory_size_scan_after_navigation(pane_id, &previous_cwd);
        self.zoxide_tracker.track(&current_cwd);
        if self.config.ui.vcs.enabled {
            self.vcs_manager.request_query(pane_id, current_cwd);
        }
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
            let cwd = pane.cwd.clone();
            let entries = pane.entries.clone();
            self.store_directory_cache(cwd.clone(), entries);
            cwd
        };
        self.restart_directory_size_scan_after_navigation(pane_id, &previous_cwd);
        self.zoxide_tracker.track(&current_cwd);
        if self.config.ui.vcs.enabled {
            self.vcs_manager.request_query(pane_id, current_cwd);
        }
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
        let trimmed = smb::strip_quotes(target);
        if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("smb://") {
            return self.goto_smb_location(trimmed);
        }

        #[cfg(not(target_os = "windows"))]
        if is_unc_path(trimmed) {
            return self.goto_smb_location(trimmed);
        }

        let Some(target_path) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: goto <path>");
            return Ok(());
        };

        if is_unc_path(trimmed) {
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

    #[cfg(test)]
    /// 在測試環境下模擬 macOS 的 command 跳轉行為，驗證 UNC 路徑會被自動導向 SMB 掛載機制。
    pub(crate) fn change_directory_from_command_as_macos(
        &mut self,
        target: &str,
        mount_root: &std::path::Path,
    ) -> io::Result<()> {
        let trimmed = smb::strip_quotes(target);
        if (trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("smb://"))
            || is_unc_path(trimmed)
        {
            return self.goto_smb_location_with_mount_root(trimmed, mount_root);
        }
        self.change_directory_from_command(target)
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
                let target_path = if path.exists() {
                    path
                } else {
                    let mut fallback = path.parent();
                    while let Some(parent) = fallback {
                        if parent.exists()
                            && parent != Path::new("/Volumes")
                            && parent != Path::new("/")
                        {
                            break;
                        }
                        fallback = parent.parent();
                    }
                    if let Some(existing_ancestor) = fallback.filter(|p| p.exists()) {
                        self.status = format!(
                            "SMB 子路徑不存在: {}；已切換至 {}",
                            path.display(),
                            existing_ancestor.display()
                        );
                        existing_ancestor.to_path_buf()
                    } else {
                        self.status = format!("smb path missing: {}", path.display());
                        return Ok(());
                    }
                };
                if !self.panes.contains_key(&self.focused_pane) {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
                self.go_to_path_and_track(self.focused_pane, &target_path)?;
                let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                };
                pane.set_bookmark_target(BookmarkTarget::SmbLocation(location.url.clone()));
                self.full_redraw_requested = true;
                if !self.status.starts_with("SMB 子路徑不存在") {
                    self.status = format!("jumped to smb: {}", location.url);
                }
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
        let trimmed = smb::strip_quotes(target);
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
        let entries_to_cache: Vec<_> = self
            .panes
            .values()
            .map(|p| (p.cwd.clone(), p.entries.clone()))
            .collect();
        for (cwd, entries) in entries_to_cache {
            self.store_directory_cache(cwd, entries);
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
        self.vcs_manager.invalidate(None);
        if self.config.ui.vcs.enabled {
            for (pane_id, pane) in &self.panes {
                self.vcs_manager.request_query(*pane_id, pane.cwd.clone());
            }
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
        let entries_to_cache: Vec<_> = self
            .panes
            .values()
            .filter(|pane| pane.cwd == directory || pane.cwd.starts_with(directory))
            .map(|p| (p.cwd.clone(), p.entries.clone()))
            .collect();
        for (cwd, entries) in entries_to_cache {
            self.store_directory_cache(cwd, entries);
        }
        let active_cwds: BTreeSet<_> = self.panes.values().map(|p| p.cwd.clone()).collect();
        // 清理不在目前開啟 panel 中的陳舊子目錄快取
        self.retain_directory_cache(|cached_path| {
            !cached_path.starts_with(directory) || active_cwds.contains(cached_path)
        });
        self.vcs_manager.invalidate(Some(directory.to_path_buf()));
        if self.config.ui.vcs.enabled {
            for (pane_id, pane) in &self.panes {
                if pane.cwd == directory || pane.cwd.starts_with(directory) {
                    self.vcs_manager.request_query(*pane_id, pane.cwd.clone());
                }
            }
        }
        Ok(())
    }
}
