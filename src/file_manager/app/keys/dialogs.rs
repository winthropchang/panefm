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

    pub(crate) fn handle_confirm_paste_overwrite_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        target_name: String,
        entry_count: usize,
        operation: ClipboardOperation,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_letter_any_case(&key, 'y') || key.code == KeyCode::Enter => {
                self.confirm_paste_overwrite(pane_id, target_name, entry_count, operation)?;
            }
            KeyCode::Esc => {
                self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
            }
            _ if key_matches_letter_any_case(&key, 'n') || key_matches_plain_letter(&key, 'q') => {
                self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
            }
            _ => {
                self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                    pane_id,
                    target_name: target_name.clone(),
                    entry_count,
                    operation,
                });
                self.status = paste_overwrite_confirm_status(&target_name, entry_count);
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
