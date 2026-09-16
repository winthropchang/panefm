use super::*;

impl App {
    pub(crate) fn handle_help_panel_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        mut selected: usize,
        mut search: PanelSearchState,
        custom_title: Option<String>,
        custom_entries: Option<Vec<HelpEntry>>,
    ) -> Result<bool> {
        let filtered_entries = if let Some(custom) = &custom_entries {
            filter_custom_help_entries(custom, &search.buffer)
        } else {
            help_entries(&search.buffer)
        };
        let filtered_len = filtered_entries.len();
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
            let next_len = if let Some(custom) = &custom_entries {
                filter_custom_help_entries(custom, &search.buffer).len()
            } else {
                help_entries(&search.buffer).len()
            };
            let status = if custom_title.is_some() {
                format!("cheatsheet search: {} ({next_len})", search.buffer)
            } else {
                help_panel_status(&search.buffer, next_len, search.editing)
            };
            self.pending_action = Some(PendingAction::HelpPanel {
                pane_id,
                selected,
                search,
                custom_title,
                custom_entries,
            });
            self.status = status;
        } else {
            if self.capture_pending_count_digit(&key) {
                self.pending_action = Some(PendingAction::HelpPanel {
                    pane_id,
                    selected,
                    search,
                    custom_title,
                    custom_entries,
                });
                return Ok(true);
            }
            if key_matches_shifted_letter(&key, 'G') {
                if let Some(count) = self.take_pending_count() {
                    if filtered_len > 0 {
                        selected = count.saturating_sub(1).min(filtered_len.saturating_sub(1));
                    }
                } else if filtered_len > 0 {
                    selected = filtered_len - 1;
                }
                self.pending_g = false;
                let status = if let Some(title) = &custom_title {
                    format!("{title} ({filtered_len} keys) (?/Esc/q to return)")
                } else {
                    help_panel_status(&search.buffer, filtered_len, false)
                };
                self.pending_action = Some(PendingAction::HelpPanel {
                    pane_id,
                    selected,
                    search,
                    custom_title,
                    custom_entries,
                });
                self.status = status;
                return Ok(true);
            }
            match key.code {
                KeyCode::Down => {
                    if filtered_len > 0 {
                        selected = (selected + self.take_count_or_one())
                            .min(filtered_len.saturating_sub(1));
                    }
                }
                _ if key_matches_plain_letter(&key, 'j') => {
                    if filtered_len > 0 {
                        selected = (selected + self.take_count_or_one())
                            .min(filtered_len.saturating_sub(1));
                    }
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(self.take_count_or_one());
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    selected = selected.saturating_sub(self.take_count_or_one());
                }
                _ if key_matches_plain_letter(&key, 'g') => {
                    if self.pending_g {
                        if let Some(count) = self.take_pending_count() {
                            selected = count.saturating_sub(1).min(filtered_len.saturating_sub(1));
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
                    if filtered_len > 0 {
                        selected = (selected + self.take_panel_page_step())
                            .min(filtered_len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                _ if key_matches_ctrl_letter(&key, 'u') => {
                    selected = selected.saturating_sub(self.take_panel_page_step());
                    self.pending_g = false;
                }
                _ if key_matches_shifted_letter(&key, 'J') => {
                    if filtered_len > 0 {
                        selected = (selected + self.take_large_move_step())
                            .min(filtered_len.saturating_sub(1));
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
                    self.execute_help_entry(&filtered_entries, selected)?;
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.execute_help_entry(&filtered_entries, selected)?;
                    return Ok(true);
                }
                KeyCode::Esc | KeyCode::F(1) => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.restore_help_return_state(false)?;
                    return Ok(true);
                }
                _ if key_matches_tilde(&key) || key_matches_question_mark(&key) => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.restore_help_return_state(false)?;
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.restore_help_return_state(false)?;
                    return Ok(true);
                }
                _ => {
                    self.clear_pending_count();
                    self.pending_g = false;
                }
            }
            let next_count = if let Some(custom) = &custom_entries {
                filter_custom_help_entries(custom, &search.buffer).len()
            } else {
                help_entries(&search.buffer).len()
            };
            let status = if let Some(title) = &custom_title {
                if search.buffer.is_empty() {
                    format!("{title} ({next_count} keys) (?/Esc/q to return)")
                } else {
                    format!("cheatsheet search: {} ({next_count})", search.buffer)
                }
            } else {
                help_panel_status(&search.buffer, next_count, false)
            };
            self.pending_action = Some(PendingAction::HelpPanel {
                pane_id,
                selected,
                search,
                custom_title,
                custom_entries,
            });
            self.status = status;
        }

        Ok(true)
    }
}
