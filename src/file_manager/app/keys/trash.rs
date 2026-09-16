use super::*;

impl App {
    pub(crate) fn handle_trash_panel_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        mut selected: usize,
        mut search: PanelSearchState,
        mut marked_ids: Vec<String>,
        mut visual_anchor: Option<usize>,
    ) -> Result<bool> {
        let entries = trash_panel_entries(&self.trash_store, &search.buffer)?;
        let len = entries.len();
        if search.editing {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    search.editing = false;
                }
                _ if key_matches_plain_letter(&key, 'q') => {
                    search.editing = false;
                }
                _ => {}
            }
            let next_len = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
            let status = trash_panel_status(
                &search.buffer,
                next_len,
                selected,
                search.editing,
                marked_ids.len(),
            );
            self.pending_action = Some(PendingAction::TrashPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            });
            self.status = status;
        } else {
            if self.capture_pending_count_digit(&key) {
                self.pending_action = Some(PendingAction::TrashPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                return Ok(true);
            }
            if key_matches_shifted_letter(&key, 'G') {
                if let Some(count) = self.take_pending_count() {
                    if len > 0 {
                        selected = count.saturating_sub(1).min(len.saturating_sub(1));
                    }
                } else if len > 0 {
                    selected = len - 1;
                }
                self.pending_g = false;
                let search_buffer = search.buffer.clone();
                let search_editing = search.editing;
                let marked_count = marked_ids.len();
                self.pending_action = Some(PendingAction::TrashPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                self.status =
                    trash_panel_status(&search_buffer, len, selected, search_editing, marked_count);
                return Ok(true);
            }
            if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V') {
                self.pending_g = false;
                if let Some(anchor) = visual_anchor.take() {
                    let added = self.commit_trash_visual_selection(
                        &entries,
                        &mut marked_ids,
                        anchor,
                        selected,
                    );
                    self.status = if added == 0 {
                        format!("trash: kept {} marked items", marked_ids.len())
                    } else {
                        format!("trash: marked {} items", marked_ids.len())
                    };
                } else if len > 0 {
                    visual_anchor = Some(selected);
                    self.status =
                        self.trash_visual_status_label(selected, selected, marked_ids.len());
                }
                self.pending_action = Some(PendingAction::TrashPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                return Ok(true);
            }
            if key_matches_plain_letter(&key, 'u') {
                self.pending_g = false;
                self.start_trash_panel_restore_confirmation(
                    pane_id,
                    &entries,
                    selected,
                    search,
                    &marked_ids,
                    visual_anchor,
                )?;
                return Ok(true);
            }
            if key_matches_shifted_letter(&key, 'U') {
                self.pending_g = false;
                self.start_trash_panel_restore_all_confirmation(
                    pane_id,
                    &entries,
                    selected,
                    search,
                    &marked_ids,
                    visual_anchor,
                )?;
                return Ok(true);
            }
            if key_matches_plain_letter(&key, 'd') {
                self.pending_g = false;
                self.start_trash_panel_delete_confirmation(
                    pane_id,
                    &entries,
                    selected,
                    search,
                    &marked_ids,
                    visual_anchor,
                )?;
                return Ok(true);
            }
            if key_matches_shifted_letter(&key, 'D') {
                self.pending_g = false;
                self.start_trash_panel_delete_all_confirmation(
                    pane_id,
                    &entries,
                    selected,
                    search,
                    &marked_ids,
                    visual_anchor,
                )?;
                return Ok(true);
            }
            match key.code {
                KeyCode::Down => {
                    if len > 0 {
                        selected = (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                _ if key_matches_plain_letter(&key, 'j') => {
                    if len > 0 {
                        selected = (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(self.take_count_or_one());
                    self.pending_g = false;
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    selected = selected.saturating_sub(self.take_count_or_one());
                    self.pending_g = false;
                }
                _ if key_matches_plain_letter(&key, 'g') => {
                    if self.pending_g {
                        if let Some(count) = self.take_pending_count() {
                            selected = count.saturating_sub(1).min(len.saturating_sub(1));
                        } else {
                            selected = 0;
                        }
                        self.pending_g = false;
                    } else {
                        self.pending_g = true;
                    }
                }
                _ if key_matches_plain_letter(&key, 'f') => {
                    self.clear_pending_count();
                    search.editing = true;
                    self.text_input_mode = RenameMode::Insert;
                    self.text_input_cursor = search.buffer.chars().count();
                    self.pending_g = false;
                }
                _ if key_matches_ctrl_letter(&key, 'd') => {
                    if len > 0 {
                        selected =
                            (selected + self.take_panel_page_step()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                _ if key_matches_ctrl_letter(&key, 'u') => {
                    selected = selected.saturating_sub(self.take_panel_page_step());
                    self.pending_g = false;
                }
                _ if key_matches_shifted_letter(&key, 'J') => {
                    if len > 0 {
                        selected =
                            (selected + self.take_large_move_step()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                _ if key_matches_shifted_letter(&key, 'K') => {
                    selected = selected.saturating_sub(self.take_large_move_step());
                    self.pending_g = false;
                }
                KeyCode::Enter => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.start_trash_panel_restore_confirmation(
                        pane_id,
                        &entries,
                        selected,
                        search,
                        &marked_ids,
                        visual_anchor,
                    )?;
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.start_trash_panel_restore_confirmation(
                        pane_id,
                        &entries,
                        selected,
                        search,
                        &marked_ids,
                        visual_anchor,
                    )?;
                    return Ok(true);
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(anchor) = visual_anchor.take() {
                        let added = self.commit_trash_visual_selection(
                            &entries,
                            &mut marked_ids,
                            anchor,
                            selected,
                        );
                        self.status = if added == 0 {
                            format!("trash: kept {} marked items", marked_ids.len())
                        } else {
                            format!("trash: marked {} items", marked_ids.len())
                        };
                    } else if !marked_ids.is_empty() {
                        let cleared = marked_ids.len();
                        marked_ids.clear();
                        self.status = format!("trash: cleared {cleared} marks");
                    } else {
                        self.status = String::from("normal mode");
                        return Ok(true);
                    }
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
                }
                _ => {
                    self.clear_pending_count();
                    self.pending_g = false;
                }
            }
            let status = if let Some(anchor) = visual_anchor {
                self.trash_visual_status_label(selected, anchor, marked_ids.len())
            } else {
                trash_panel_status(
                    &search.buffer,
                    len,
                    selected,
                    search.editing,
                    marked_ids.len(),
                )
            };
            self.pending_action = Some(PendingAction::TrashPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            });
            self.status = status;
        }

        Ok(true)
    }
}
