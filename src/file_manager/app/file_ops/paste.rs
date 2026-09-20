//! 剪貼簿貼上、覆蓋確認、背景貼上任務與 Undo 復原歷史。

use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::super::*;

impl App {
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
            busy.push(entry.source_path.clone());
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
        let action = match operation {
            ClipboardOperation::Copy => "copying",
            ClipboardOperation::Cut => "moving",
        };
        let first_item = clipboard
            .entries
            .first()
            .and_then(|entry| {
                entry
                    .source_path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| String::from("item"));
        self.status = if entry_count <= 1 {
            format!(
                "{action} {first_item} [0%] in background to {} [task {task_id}]",
                target_dir.display()
            )
        } else {
            format!(
                "pasting {entry_count} item(s) in background to {} [task {task_id}]",
                target_dir.display()
            )
        };
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
}
