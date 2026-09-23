use super::*;

impl App {
    pub(crate) fn handle_confirm_delete_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        target_name: String,
        permanent: bool,
        warning_message: Option<String>,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_shifted_letter(&key, 'D') => {
                self.confirm_delete(pane_id, &target_name, true)?;
            }
            _ if key_matches_plain_letter(&key, 'd') => {
                self.status = if permanent {
                    format!("delete cancelled: {target_name}")
                } else {
                    format!("trash cancelled: {target_name}")
                };
            }
            _ if key_matches_letter_any_case(&key, 'y') => {
                self.confirm_delete(pane_id, &target_name, permanent)?;
            }
            KeyCode::Esc => {
                self.status = if permanent {
                    format!("delete cancelled: {target_name}")
                } else {
                    format!("trash cancelled: {target_name}")
                };
            }
            _ if key_matches_letter_any_case(&key, 'n') || key_matches_plain_letter(&key, 'q') => {
                self.status = if permanent {
                    format!("delete cancelled: {target_name}")
                } else {
                    format!("trash cancelled: {target_name}")
                };
            }
            _ => {
                let warning_message = warning_message.clone();
                self.pending_action = Some(PendingAction::ConfirmDelete {
                    pane_id,
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
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_confirm_paste_overwrite_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        target_name: String,
        entry_count: usize,
        operation: ClipboardOperation,
        conflicts: Vec<PasteConflictItem>,
        current_index: usize,
        selected_option: usize,
        mut decisions: Vec<(PathBuf, CollisionStrategy)>,
    ) -> Result<bool> {
        if conflicts.is_empty() {
            match key.code {
                _ if key_matches_letter_any_case(&key, 'y')
                    || key_matches_plain_letter(&key, 'o')
                    || key.code == KeyCode::Enter =>
                {
                    self.confirm_paste_overwrite(pane_id, target_name, entry_count, operation)?;
                }
                KeyCode::Esc => {
                    self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
                }
                _ if key_matches_letter_any_case(&key, 'n')
                    || key_matches_plain_letter(&key, 'q')
                    || key_matches_plain_letter(&key, 'c') =>
                {
                    self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                        pane_id,
                        target_name: target_name.clone(),
                        entry_count,
                        operation,
                        conflicts,
                        current_index,
                        selected_option,
                        decisions,
                    });
                    self.status = paste_overwrite_confirm_status(&target_name, entry_count);
                }
            }
            return Ok(true);
        }

        const SINGLE_CHOICES: [PasteConflictChoice; 4] = [
            PasteConflictChoice::Overwrite,
            PasteConflictChoice::AutoRename,
            PasteConflictChoice::Skip,
            PasteConflictChoice::Cancel,
        ];
        const MULTI_CHOICES: [PasteConflictChoice; 7] = [
            PasteConflictChoice::Overwrite,
            PasteConflictChoice::OverwriteAll,
            PasteConflictChoice::AutoRename,
            PasteConflictChoice::AutoRenameAll,
            PasteConflictChoice::Skip,
            PasteConflictChoice::SkipAll,
            PasteConflictChoice::Cancel,
        ];

        let is_multi = conflicts.len() > 1;
        let choices: &[PasteConflictChoice] = if is_multi {
            &MULTI_CHOICES[..]
        } else {
            &SINGLE_CHOICES[..]
        };

        // 1. 檢查選單游標導航（↑ / ↓ / k / j）
        if key_matches_plain_letter(&key, 'k') || key.code == KeyCode::Up {
            let next_option = selected_option.saturating_sub(1);
            self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                pane_id,
                target_name,
                entry_count,
                operation,
                conflicts,
                current_index,
                selected_option: next_option,
                decisions,
            });
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'j') || key.code == KeyCode::Down {
            let next_option = (selected_option + 1).min(choices.len() - 1);
            self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                pane_id,
                target_name,
                entry_count,
                operation,
                conflicts,
                current_index,
                selected_option: next_option,
                decisions,
            });
            return Ok(true);
        }

        // 2. 判斷使用者觸發的選擇
        let selected_choice = if key.code == KeyCode::Enter {
            Some(
                choices
                    .get(selected_option)
                    .copied()
                    .unwrap_or(PasteConflictChoice::Overwrite),
            )
        } else if key_matches_shifted_letter(&key, 'O')
            || (is_multi && key_matches_plain_letter(&key, 'a'))
            || key_matches_shifted_letter(&key, 'Y')
        {
            Some(if is_multi {
                PasteConflictChoice::OverwriteAll
            } else {
                PasteConflictChoice::Overwrite
            })
        } else if key_matches_plain_letter(&key, 'o') || key_matches_letter_any_case(&key, 'y') {
            Some(PasteConflictChoice::Overwrite)
        } else if key_matches_shifted_letter(&key, 'R') {
            Some(if is_multi {
                PasteConflictChoice::AutoRenameAll
            } else {
                PasteConflictChoice::AutoRename
            })
        } else if key_matches_plain_letter(&key, 'r') {
            Some(PasteConflictChoice::AutoRename)
        } else if key_matches_shifted_letter(&key, 'S') {
            Some(if is_multi {
                PasteConflictChoice::SkipAll
            } else {
                PasteConflictChoice::Skip
            })
        } else if key_matches_plain_letter(&key, 's') || key_matches_letter_any_case(&key, 'n') {
            Some(PasteConflictChoice::Skip)
        } else if key.code == KeyCode::Esc
            || key_matches_plain_letter(&key, 'q')
            || key_matches_plain_letter(&key, 'c')
        {
            Some(PasteConflictChoice::Cancel)
        } else {
            None
        };

        let Some(choice) = selected_choice else {
            // 未辨識的按鍵：保留目前視窗
            self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                pane_id,
                target_name: target_name.clone(),
                entry_count,
                operation,
                conflicts,
                current_index,
                selected_option,
                decisions,
            });
            self.status = paste_overwrite_confirm_status(&target_name, entry_count);
            return Ok(true);
        };

        // 3. 套用所選決策
        match choice {
            PasteConflictChoice::Cancel => {
                self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
            }
            PasteConflictChoice::OverwriteAll => {
                for item in conflicts.iter().skip(current_index) {
                    decisions.push((item.source_path.clone(), CollisionStrategy::Overwrite));
                }
                self.execute_paste_with_conflict_decisions(decisions)?;
            }
            PasteConflictChoice::AutoRenameAll => {
                for item in conflicts.iter().skip(current_index) {
                    decisions.push((item.source_path.clone(), CollisionStrategy::Rename));
                }
                self.execute_paste_with_conflict_decisions(decisions)?;
            }
            PasteConflictChoice::SkipAll => {
                for item in conflicts.iter().skip(current_index) {
                    decisions.push((item.source_path.clone(), CollisionStrategy::Skip));
                }
                self.execute_paste_with_conflict_decisions(decisions)?;
            }
            PasteConflictChoice::Overwrite
            | PasteConflictChoice::AutoRename
            | PasteConflictChoice::Skip => {
                let strategy = match choice {
                    PasteConflictChoice::Overwrite => CollisionStrategy::Overwrite,
                    PasteConflictChoice::AutoRename => CollisionStrategy::Rename,
                    PasteConflictChoice::Skip => CollisionStrategy::Skip,
                    _ => unreachable!(),
                };
                if let Some(item) = conflicts.get(current_index) {
                    decisions.push((item.source_path.clone(), strategy));
                }
                let next_index = current_index + 1;
                if next_index < conflicts.len() {
                    let next_item = &conflicts[next_index];
                    let next_display = next_item.display_name.clone();
                    let next_target_name = format!(
                        "{} ({} of {})",
                        next_display,
                        next_index + 1,
                        conflicts.len()
                    );
                    self.status = paste_overwrite_confirm_status(&next_target_name, entry_count);
                    self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                        pane_id,
                        target_name: next_display,
                        entry_count,
                        operation,
                        conflicts,
                        current_index: next_index,
                        selected_option: 0,
                        decisions,
                    });
                } else {
                    self.execute_paste_with_conflict_decisions(decisions)?;
                }
            }
        }

        Ok(true)
    }

    pub(crate) fn handle_confirm_trash_action_key(
        &mut self,
        key: KeyEvent,
        action: TrashConfirmAction,
        target_name: String,
        entry_count: usize,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 'd')
                && matches!(&action, TrashConfirmAction::DeleteFromPanel { .. }) =>
            {
                self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                    &action,
                    marked_ids,
                    visual_anchor,
                ));
                self.status = trash_confirm_cancelled_status(&action, &target_name, entry_count);
            }
            _ if key_matches_letter_any_case(&key, 'y') || key.code == KeyCode::Enter => {
                self.confirm_trash_action(action, target_name, entry_count)?;
            }
            KeyCode::Esc => {
                self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                    &action,
                    marked_ids,
                    visual_anchor,
                ));
                self.status = trash_confirm_cancelled_status(&action, &target_name, entry_count);
            }
            _ if key_matches_letter_any_case(&key, 'n') || key_matches_plain_letter(&key, 'q') => {
                self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                    &action,
                    marked_ids,
                    visual_anchor,
                ));
                self.status = trash_confirm_cancelled_status(&action, &target_name, entry_count);
            }
            _ => {
                self.pending_action = Some(PendingAction::ConfirmTrashAction {
                    action: action.clone(),
                    target_name: target_name.clone(),
                    entry_count,
                    marked_ids,
                    visual_anchor,
                });
                self.status = trash_confirm_status(&action, &target_name, entry_count);
            }
        }
        Ok(true)
    }
}
