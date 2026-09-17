//! 檔案刪除、背景永久刪除任務與垃圾桶確認面板操作。

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::super::*;
use crate::file_manager::trash::TrashListEntry;

impl App {
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
}
