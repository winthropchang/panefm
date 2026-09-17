use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::super::*;

impl App {
    /// 接收檔案系統 watcher 事件，去重後刷新所有顯示受影響目錄的 panel。
    ///
    /// 參數：無；監看目錄來自目前 `panes`，事件來自 [`FilesystemWatcher`] channel。
    /// 回傳：`()`；watcher 或單一目錄刷新失敗只寫入狀態列，不會結束主事件迴圈。
    pub(crate) fn poll_filesystem_watcher(&mut self) {
        let directories = self
            .panes
            .values()
            .map(|pane| pane.cwd.clone())
            .collect::<BTreeSet<_>>();
        let (changed, watch_errors) = match self.filesystem_watcher.as_mut() {
            Some(watcher) => {
                let errors = watcher.sync_directories(directories);
                (watcher.changed_directories(), errors)
            }
            None => return,
        };

        if !watch_errors.is_empty() {
            self.status = format!("filesystem watcher failed: {}", watch_errors.join(" | "));
        }
        if !changed.is_empty() {
            // 如果目前有背景檔案傳輸或寫入進行中，且變更的目錄包含正在寫入的忙碌路徑，
            // 這些事件純粹是我們自己的傳輸執行緒寫入分塊造成的檔案系統事件。
            // 不應在傳輸中途每 400ms 反覆 reload 該目錄，避免網路芳鄰（SMB）大量產生
            // read_dir 造成介面卡頓、進度跳動或畫面閃爍。傳輸完成後 FileJobEvent 會自動觸發全量 reload。
            let changed_to_process: BTreeSet<PathBuf> =
                if self.active_file_job_busy_paths.is_empty() {
                    changed
                } else {
                    changed
                        .into_iter()
                        .filter(|dir| {
                            !self.active_file_job_busy_paths.values().any(|busy_paths| {
                                busy_paths.iter().any(|busy| {
                                    busy == dir
                                        || busy.starts_with(dir)
                                        || busy.parent() == Some(dir.as_path())
                                })
                            })
                        })
                        .collect()
                };

            if !changed_to_process.is_empty() {
                self.pending_watched_directories.extend(changed_to_process);
                let debounce = if self.file_job_receivers.is_empty() {
                    self.config.watcher.debounce
                } else {
                    Duration::from_millis(400)
                };
                // 第一個事件設定 deadline，後續同批事件只合併路徑而不無限延後刷新。
                self.filesystem_refresh_deadline
                    .get_or_insert_with(|| Instant::now() + debounce);
            }
        }

        let Some(deadline) = self.filesystem_refresh_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }

        self.filesystem_refresh_deadline = None;
        let changed = std::mem::take(&mut self.pending_watched_directories);
        if let Err(error) = self.reload_watched_directories(&changed) {
            self.status = format!("automatic directory refresh failed: {error}");
        }
    }

    /// 重新載入目前工作目錄出現在 watcher 變更集合中的所有 panel。
    ///
    /// 參數：`directories: &BTreeSet<PathBuf>`，已經過 debounce 的異動目錄集合。
    /// 回傳：`io::Result<()>`；任一受影響 panel 無法重新讀取時回傳原始 I/O 錯誤。
    pub(crate) fn reload_watched_directories(
        &mut self,
        directories: &BTreeSet<PathBuf>,
    ) -> io::Result<()> {
        let affected_panes = self
            .panes
            .iter()
            .filter(|(_, pane)| directories.contains(&pane.cwd))
            .map(|(pane_id, _)| *pane_id)
            .collect::<Vec<_>>();
        for pane_id in &affected_panes {
            let pane = self.panes.get_mut(pane_id).expect("pane id came from map");
            pane.reload()?;
        }
        for pane_id in &affected_panes {
            if let Some(pane) = self.panes.get(pane_id) {
                self.directory_entry_cache
                    .insert(pane.cwd.clone(), pane.entries.clone());
            }
        }
        for pane_id in &affected_panes {
            if self.file_job_receivers.is_empty()
                && self
                    .panes
                    .get(pane_id)
                    .is_some_and(|pane| matches!(pane.active_detail_kind(), SortDetailKind::Size))
            {
                self.start_directory_size_scan(*pane_id);
            }
        }
        for dir in directories {
            self.vcs_manager.invalidate(Some(dir.clone()));
            if !self.panes.values().any(|pane| &pane.cwd == dir) {
                self.directory_entry_cache.remove(dir);
            }
        }
        for pane_id in &affected_panes {
            if let Some(pane) = self.panes.get(pane_id)
                && self.config.ui.vcs.enabled
            {
                self.vcs_manager.request_query(*pane_id, pane.cwd.clone());
            }
        }
        Ok(())
    }

    /// 非阻塞接收 UNC `goto` 的背景載入結果，完成後才替換指定 panel。
    ///
    /// 參數：無；資料來自 `network_goto_rx`。
    /// 回傳：`()`；尚未完成時立即返回，取消後晚到的結果也不會被套用。
    pub(crate) fn poll_network_goto(&mut self) {
        let Some(receiver) = self.network_goto_rx.take() else {
            return;
        };

        let event = match receiver.try_recv() {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => {
                self.network_goto_rx = Some(receiver);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(task_id) = self.active_network_goto_task_id.take() {
                    self.finish_task(
                        task_id,
                        TaskState::Failed,
                        String::from("network goto worker disconnected"),
                    );
                }
                self.status = String::from("network goto failed: worker disconnected");
                return;
            }
        };

        if self.active_network_goto_task_id != Some(event.task_id) {
            return;
        }
        self.active_network_goto_task_id = None;

        match event.result {
            Ok(pane) if self.panes.contains_key(&event.pane_id) => {
                let cwd = pane.cwd.clone();
                self.panes.insert(event.pane_id, pane);
                self.zoxide_tracker.track(&cwd);
                self.finish_task(
                    event.task_id,
                    TaskState::Done,
                    format!("opened {}", event.target.display()),
                );
                self.full_redraw_requested = true;
                self.status = format!("jumped to path: {}", event.target.display());
            }
            Ok(_) => {
                self.finish_task(
                    event.task_id,
                    TaskState::Cancelled,
                    String::from("panel no longer exists"),
                );
                self.status = String::from("network goto cancelled: panel no longer exists");
            }
            Err(error) => {
                self.finish_task(event.task_id, TaskState::Failed, error.to_string());
                self.status = format!("path jump failed: {} ({error})", event.target.display());
            }
        }
    }
}
