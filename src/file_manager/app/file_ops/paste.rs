//! 剪貼簿貼上、覆蓋確認、背景貼上任務與 Undo 復原歷史。

use std::collections::HashMap;
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
    /// - 若有同名衝突，彈出衝突選單詢問使用者處理策略。
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
        let conflicts = self.paste_conflict_items()?;
        if conflicts.is_empty() {
            return self.paste_into_focused_pane_impl(false);
        }

        let first_name = conflicts[0].display_name.clone();
        let target_name = if conflicts.len() == 1 {
            first_name.clone()
        } else {
            format!("{} items", conflicts.len())
        };
        self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
            pane_id: self.focused_pane,
            target_name: target_name.clone(),
            entry_count: clipboard.entries.len(),
            operation: clipboard.operation,
            conflicts,
            current_index: 0,
            selected_option: 0,
            decisions: Vec::new(),
        });
        self.status = paste_overwrite_confirm_status(&target_name, clipboard.entries.len());
        Ok(())
    }

    /// 負責實作一般貼上與覆蓋貼上的共用流程。
    pub(crate) fn paste_into_focused_pane_impl(&mut self, overwrite: bool) -> io::Result<()> {
        let default_strategy = if overwrite {
            CollisionStrategy::Overwrite
        } else {
            CollisionStrategy::Rename
        };
        let mut strategies = HashMap::new();
        if let Some(clipboard) = &self.clipboard {
            for entry in &clipboard.entries {
                strategies.insert(entry.source_path.clone(), default_strategy);
            }
        }
        self.paste_into_focused_pane_with_strategies(&strategies)
    }

    /// 依據對個別衝突項目所決定的碰撞策略執行貼上流程。
    pub(crate) fn execute_paste_with_conflict_decisions(
        &mut self,
        decisions: Vec<(PathBuf, CollisionStrategy)>,
    ) -> io::Result<()> {
        let mut strategies: HashMap<PathBuf, CollisionStrategy> = decisions.into_iter().collect();
        if let Some(clipboard) = &self.clipboard {
            for entry in &clipboard.entries {
                strategies
                    .entry(entry.source_path.clone())
                    .or_insert(CollisionStrategy::Overwrite);
            }
        }
        self.paste_into_focused_pane_with_strategies(&strategies)
    }

    /// 依據指定的檔案碰撞策略表貼上內部剪貼簿中的項目。
    pub(crate) fn paste_into_focused_pane_with_strategies(
        &mut self,
        strategies: &HashMap<PathBuf, CollisionStrategy>,
    ) -> io::Result<()> {
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
            return self.start_background_paste_with_strategies(
                self.focused_pane,
                target_dir,
                clipboard,
                strategies.clone(),
            );
        }

        let mut pasted_count = 0usize;
        let mut skipped_count = 0usize;
        let mut renamed_count = 0usize;
        let mut history_items = Vec::new();
        let mut remaining_cut_entries = Vec::new();

        for entry in &clipboard.entries {
            if entry.source_path.parent() == Some(target_dir.as_path())
                && clipboard.operation == ClipboardOperation::Cut
            {
                continue;
            }

            let strategy = strategies
                .get(&entry.source_path)
                .copied()
                .unwrap_or(CollisionStrategy::Overwrite);

            if strategy == CollisionStrategy::Skip {
                skipped_count += 1;
                if clipboard.operation == ClipboardOperation::Cut {
                    remaining_cut_entries.push(entry.clone());
                }
                continue;
            }

            let overwrite = match strategy {
                CollisionStrategy::Overwrite => true,
                CollisionStrategy::Rename => {
                    renamed_count += 1;
                    false
                }
                CollisionStrategy::Skip => unreachable!(),
            };

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

        if pasted_count == 0 && skipped_count > 0 {
            self.status = String::from("all conflicting items skipped; nothing pasted");
            return Ok(());
        }

        if pasted_count == 0 {
            self.status = String::from("nothing to paste into this directory");
            return Ok(());
        }

        self.reload_all_panes()?;
        if skipped_count > 0 {
            let base = match clipboard.operation {
                ClipboardOperation::Copy => format!("pasted copy: {pasted_count} item(s)"),
                ClipboardOperation::Cut => format!("moved: {pasted_count} item(s)"),
            };
            self.status = format!("{base} (skipped {skipped_count} item(s))");
        } else {
            let has_overwrites = strategies
                .values()
                .any(|s| *s == CollisionStrategy::Overwrite);
            self.status = paste_success_status(
                clipboard.operation,
                has_overwrites && renamed_count == 0,
                pasted_count,
            );
        }

        if clipboard.operation == ClipboardOperation::Cut {
            if remaining_cut_entries.is_empty() {
                self.clipboard = None;
            } else {
                self.clipboard = Some(ClipboardState {
                    operation: ClipboardOperation::Cut,
                    entries: remaining_cut_entries,
                });
            }
        }
        self.record_file_operation(clipboard.operation, history_items);

        Ok(())
    }

    /// 把大型或網路目的地 paste 排入背景 task，避免傳輸期間凍結 TUI。
    pub(crate) fn start_background_paste(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        clipboard: ClipboardState,
        overwrite: bool,
    ) -> io::Result<()> {
        let default_strategy = if overwrite {
            CollisionStrategy::Overwrite
        } else {
            CollisionStrategy::Rename
        };
        let mut strategies = HashMap::new();
        for entry in &clipboard.entries {
            strategies.insert(entry.source_path.clone(), default_strategy);
        }
        self.start_background_paste_with_strategies(pane_id, target_dir, clipboard, strategies)
    }

    /// 把大型或網路目的地 paste 依指定碰撞策略排入背景 task。
    pub(crate) fn start_background_paste_with_strategies(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        clipboard: ClipboardState,
        strategies: HashMap<PathBuf, CollisionStrategy>,
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
        self.update_task_progress(task_id, 0, 1);
        let (sender, receiver) = mpsc::channel();
        let worker_clipboard = clipboard.clone();
        let worker_target_dir = target_dir.clone();
        let worker_strategies = strategies.clone();
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
            let result = perform_paste_job_with_strategies(
                &worker_clipboard,
                &worker_target_dir,
                &worker_strategies,
                &mut progress,
            );

            let _ = sender.send(FileJobEvent::Progress {
                task_id,
                completed_bytes,
                total_bytes: total_bytes.max(completed_bytes),
            });
            let effective_overwrite = worker_strategies
                .values()
                .any(|s| *s == CollisionStrategy::Overwrite);
            let _ = sender.send(FileJobEvent::Paste {
                task_id,
                clipboard: worker_clipboard,
                overwrite: effective_overwrite,
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

    /// 掃描這次貼上是否會和目前目標目錄中的既有項目同名，並收集衝突項目詳細資訊。
    pub(crate) fn paste_conflict_items(&self) -> io::Result<Vec<PasteConflictItem>> {
        let Some(clipboard) = self.clipboard.as_ref() else {
            return Ok(Vec::new());
        };
        let target_dir = self
            .panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "panel no longer exists"))?;

        let mut conflicts = Vec::new();
        for (entry_index, entry) in clipboard.entries.iter().enumerate() {
            let Some(file_name) = entry.source_path.file_name() else {
                continue;
            };
            let direct_target = target_dir.join(file_name);
            let same_location = entry.source_path.parent() == Some(target_dir.as_path())
                && direct_target == entry.source_path;
            if !same_location && direct_target.exists() {
                conflicts.push(PasteConflictItem {
                    entry_index,
                    display_name: entry.display_name.clone(),
                    source_path: entry.source_path.clone(),
                    target_path: direct_target,
                    is_dir: entry.source_path.is_dir(),
                });
            }
        }

        Ok(conflicts)
    }

    /// 掃描這次貼上是否會和目前目標目錄中的既有項目同名。
    ///
    /// 規則：
    /// - 只有「直接同名」的目標才算衝突。
    /// - 若來源本身就位於同一個目錄，視為 duplicate copy / move 情境，不算衝突。
    #[allow(dead_code)]
    pub(crate) fn paste_conflict_names(&self) -> io::Result<Vec<String>> {
        self.paste_conflict_items()
            .map(|items| items.into_iter().map(|item| item.display_name).collect())
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
