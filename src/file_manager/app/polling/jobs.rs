use std::sync::mpsc;

use super::super::*;

impl App {
    /// 非阻塞接收大型 paste/compress/extract 工作，並在主執行緒更新 UI 與 Undo。
    ///
    /// 參數：無；接收端保存於 `file_job_receivers`。
    /// 回傳：`bool`；若有任何進度更新或工作完成則回傳 `true`。
    pub(crate) fn poll_file_jobs(&mut self) -> bool {
        let mut changed = false;
        let task_ids = self.file_job_receivers.keys().copied().collect::<Vec<_>>();
        for task_id in task_ids {
            let Some(receiver) = self.file_job_receivers.remove(&task_id) else {
                continue;
            };
            let mut completed = false;
            let mut refresh_target = None;
            loop {
                match receiver.try_recv() {
                    Ok(FileJobEvent::DestinationVisible { target_dir }) => {
                        refresh_target = Some(target_dir);
                        changed = true;
                    }
                    Ok(FileJobEvent::Progress {
                        task_id,
                        completed_bytes,
                        total_bytes,
                    }) => {
                        changed = true;
                        self.update_task_progress(task_id, completed_bytes, total_bytes);
                        if let Some(task) = self.task_log.iter().find(|t| t.id == task_id) {
                            let pct = task.progress_percent.unwrap_or(0);
                            let action = match task.kind.as_str() {
                                "paste" if task.title.starts_with("move") => "moving",
                                "paste" => "copying",
                                "compress" => "compressing",
                                "extract" => "extracting",
                                _ => "processing",
                            };
                            let first_item = task
                                .source_locations
                                .first()
                                .and_then(|p| {
                                    std::path::Path::new(p).file_name().and_then(|n| n.to_str())
                                })
                                .unwrap_or("item");
                            self.status = if task.source_locations.len() <= 1 {
                                format!("{action} {first_item} [{pct}%]")
                            } else {
                                format!("{action} {} items [{pct}%]", task.source_locations.len())
                            };
                        }
                    }
                    Ok(event) => {
                        self.apply_file_job_event(event);
                        completed = true;
                        changed = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        completed = true;
                        changed = true;
                        if self
                            .task_log
                            .iter()
                            .any(|task| task.id == task_id && task.state == TaskState::Running)
                        {
                            self.finish_task(
                                task_id,
                                TaskState::Failed,
                                String::from("file worker disconnected"),
                            );
                            self.status =
                                format!("file task {task_id} failed: worker disconnected");
                        }
                        break;
                    }
                }
            }
            if let Some(target_dir) = refresh_target
                && let Err(error) = self.reload_panes_in_tree(&target_dir)
            {
                self.status =
                    format!("background paste started; destination refresh failed: {error}");
            }
            if !completed {
                self.file_job_receivers.insert(task_id, receiver);
            } else {
                self.active_file_job_busy_paths.remove(&task_id);
                if self
                    .task_log
                    .iter()
                    .any(|task| task.id == task_id && task.state == TaskState::Running)
                {
                    self.finish_task(
                        task_id,
                        TaskState::Failed,
                        String::from("file worker disconnected"),
                    );
                    self.status = format!("file task {task_id} failed: worker disconnected");
                }
            }
        }
        changed
    }

    /// 套用單一背景檔案工作的完成結果。
    ///
    /// 參數：`event: FileJobEvent`，worker 回傳的 paste、compress、extract 或 delete 結果。
    /// 回傳：`()`；I/O 錯誤會完整寫入 task 與狀態列，不會中止主事件迴圈。
    pub(crate) fn apply_file_job_event(&mut self, event: FileJobEvent) {
        match event {
            FileJobEvent::DestinationVisible { .. } => {
                // 目標可見事件已在 `poll_file_jobs` 即時處理，不屬於完成事件。
            }
            FileJobEvent::Progress { .. } => {
                // Progress 已在 `poll_file_jobs` 合併處理，完成事件才會進入這個函數。
            }
            FileJobEvent::Paste {
                task_id,
                clipboard,
                overwrite,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                let operation = clipboard.operation;
                self.record_file_operation(operation, result.history_items);
                if let Some(failure) = result.failure {
                    let status = paste_failure_status(
                        &failure.display_name,
                        &failure.planned_target,
                        &failure.error,
                    );
                    self.finish_task(task_id, TaskState::Failed, status.clone());
                    self.status = status;
                } else {
                    if operation == ClipboardOperation::Cut
                        && self.clipboard.as_ref() == Some(&clipboard)
                    {
                        self.clipboard = None;
                    }
                    let status = paste_success_status(operation, overwrite, result.pasted_count);
                    self.finish_task(task_id, TaskState::Done, status.clone());
                    self.status = status;
                }
                if let Err(error) = self.reload_all_panes() {
                    self.status = format!("{}; refresh failed: {error}", self.status);
                }
                self.full_redraw_requested = true;
            }
            FileJobEvent::Compress {
                task_id,
                pane_id,
                entry_count,
                first_name,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok(archive_path) => {
                        if let Err(error) = self.reload_all_panes() {
                            self.status = format!("compress completed; refresh failed: {error}");
                            self.finish_task(task_id, TaskState::Failed, self.status.clone());
                            return;
                        }
                        let target_pane_id = if self.panes.contains_key(&pane_id) {
                            Some(pane_id)
                        } else {
                            archive_path.parent().and_then(|parent| {
                                self.panes
                                    .iter()
                                    .find(|(_, p)| p.cwd == parent)
                                    .map(|(&id, _)| id)
                            })
                        };
                        if let Some(target_id) = target_pane_id
                            && let Some(pane) = self.panes.get_mut(&target_id)
                        {
                            pane.select_path(&archive_path);
                        }
                        let archive_name = archive_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("archive.zip");
                        self.status = if entry_count == 1 {
                            format!("compressed {first_name} -> {archive_name}")
                        } else {
                            format!("compressed {entry_count} items -> {archive_name}")
                        };
                        self.finish_task(task_id, TaskState::Done, self.status.clone());
                        self.full_redraw_requested = true;
                    }
                    Err(error) => {
                        self.status = format!("compress failed: {error}");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                }
            }
            FileJobEvent::Extract {
                task_id,
                pane_id,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok((extracted, skipped)) if !extracted.is_empty() => {
                        if let Err(error) = self.reload_all_panes() {
                            self.status = format!("extract completed; refresh failed: {error}");
                            self.finish_task(task_id, TaskState::Failed, self.status.clone());
                            return;
                        }
                        if let Some(first) = extracted.first() {
                            let target_pane_id = if self.panes.contains_key(&pane_id) {
                                Some(pane_id)
                            } else {
                                first.output_path.parent().and_then(|parent| {
                                    self.panes
                                        .iter()
                                        .find(|(_, p)| p.cwd == parent)
                                        .map(|(&id, _)| id)
                                })
                            };
                            if let Some(target_id) = target_pane_id
                                && let Some(pane) = self.panes.get_mut(&target_id)
                            {
                                pane.select_path(&first.output_path);
                            }
                        }
                        self.status = extraction_status_label(&extracted, skipped);
                        self.finish_task(task_id, TaskState::Done, self.status.clone());
                        self.full_redraw_requested = true;
                    }
                    Ok((_, skipped)) => {
                        self.status = format!("no supported archives selected (skipped {skipped})");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                    Err(error) => {
                        self.status = format!("extract failed: {error}");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                }
            }
            FileJobEvent::Delete {
                task_id,
                target_name,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok(deleted_names) => {
                        let status = if deleted_names.len() == 1 {
                            format!("deleted permanently {}", deleted_names[0])
                        } else {
                            format!("deleted permanently {} items", deleted_names.len())
                        };
                        self.finish_task(task_id, TaskState::Done, status.clone());
                        self.status = status;
                    }
                    Err(error) => {
                        let status = format!("failed to delete {target_name}: {error}");
                        self.finish_task(task_id, TaskState::Failed, status.clone());
                        self.status = status;
                    }
                }
                if let Err(error) = self.reload_all_panes() {
                    self.status = format!("{}; refresh failed: {error}", self.status);
                }
                self.full_redraw_requested = true;
            }
        }
    }
}
