use std::path::PathBuf;

use super::*;

impl App {
    pub(crate) fn handle_bookmark_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 'b') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("normal mode");
                Ok(true)
            }
            _ if key_matches_plain_letter(&key, 'a') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.add_bookmark_with_auto_key(pane_id)?;
                Ok(true)
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_bookmark_list_with_mode(pane_id, BookmarkListMode::Jump);
                Ok(true)
            }
            _ if key_matches_plain_letter(&key, 'd') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_bookmark_list_with_mode(pane_id, BookmarkListMode::Delete);
                Ok(true)
            }
            _ if key_matches_shifted_letter(&key, 'D') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.delete_all_bookmarks()?;
                Ok(true)
            }
            KeyCode::Esc => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("normal mode");
                Ok(true)
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("normal mode");
                Ok(true)
            }
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_action = Some(PendingAction::BookmarkPicker { pane_id });
                self.status = String::from("bookmark: choose a/g/d/D from the panel");
                Ok(true)
            }
        }
    }

    pub(crate) fn handle_bookmark_list_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        mut selected: usize,
        mode: BookmarkListMode,
        mut search: PanelSearchState,
    ) -> Result<bool> {
        let entries = self.bookmark_store.list();
        let filtered_entries = filtered_bookmark_entries(entries.clone(), &search.buffer);
        let len = filtered_entries.len();
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
            let next_len =
                filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer).len();
            let status =
                bookmark_list_status(&search.buffer, next_len, selected, mode, search.editing);
            self.pending_action = Some(PendingAction::BookmarkList {
                pane_id,
                selected,
                mode,
                search,
            });
            self.status = status;
        } else {
            if self.capture_pending_count_digit(&key) {
                self.pending_action = Some(PendingAction::BookmarkList {
                    pane_id,
                    selected,
                    mode,
                    search,
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
                let status = bookmark_list_status(&search.buffer, len, selected, mode, false);
                self.pending_action = Some(PendingAction::BookmarkList {
                    pane_id,
                    selected,
                    mode,
                    search,
                });
                self.status = status;
                return Ok(true);
            }
            match key.code {
                KeyCode::Char(bookmark_key)
                    if matches!(mode, BookmarkListMode::Delete)
                        && filtered_entries
                            .iter()
                            .any(|entry| entry.key == bookmark_key) =>
                {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.delete_bookmark(bookmark_key)?;
                    return Ok(true);
                }
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
                _ if key_matches_plain_letter(&key, 'f') => {
                    self.clear_pending_count();
                    search.editing = true;
                    self.text_input_mode = RenameMode::Insert;
                    self.text_input_cursor = search.buffer.chars().count();
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
                KeyCode::Enter => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    match mode {
                        BookmarkListMode::Jump => {
                            self.open_bookmark_from_list(pane_id, &filtered_entries, selected)?;
                        }
                        BookmarkListMode::Delete => {
                            self.delete_bookmark_from_list(&filtered_entries, selected)?;
                        }
                    }
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if matches!(mode, BookmarkListMode::Jump) {
                        self.open_bookmark_from_list(pane_id, &filtered_entries, selected)?;
                    } else {
                        let status =
                            bookmark_list_status(&search.buffer, len, selected, mode, false);
                        self.pending_action = Some(PendingAction::BookmarkList {
                            pane_id,
                            selected,
                            mode,
                            search,
                        });
                        self.status = status;
                    }
                    return Ok(true);
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
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
            let status = bookmark_list_status(&search.buffer, len, selected, mode, false);
            self.pending_action = Some(PendingAction::BookmarkList {
                pane_id,
                selected,
                mode,
                search,
            });
            self.status = status;
        }

        Ok(true)
    }

    pub(crate) fn handle_zoxide_list_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        mut selected: usize,
        entries: Vec<PathBuf>,
        mut search: PanelSearchState,
    ) -> Result<bool> {
        let filtered_entries = filtered_zoxide_entries(&entries, &search.buffer);
        let len = filtered_entries.len();
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
            let next_len = filtered_zoxide_entries(&entries, &search.buffer).len();
            let status = zoxide_list_status(&search.buffer, next_len, selected, search.editing);
            self.pending_action = Some(PendingAction::ZoxideList {
                pane_id,
                selected,
                entries,
                search,
            });
            self.status = status;
        } else {
            if self.capture_pending_count_digit(&key) {
                self.pending_action = Some(PendingAction::ZoxideList {
                    pane_id,
                    selected,
                    entries,
                    search,
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
                let status = zoxide_list_status(&search.buffer, len, selected, false);
                self.pending_action = Some(PendingAction::ZoxideList {
                    pane_id,
                    selected,
                    entries,
                    search,
                });
                self.status = status;
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
                _ if key_matches_plain_letter(&key, 'f') => {
                    self.clear_pending_count();
                    search.editing = true;
                    self.text_input_mode = RenameMode::Insert;
                    self.text_input_cursor = search.buffer.chars().count();
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
                KeyCode::Enter => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.open_zoxide_from_list(pane_id, &filtered_entries, selected)?;
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.open_zoxide_from_list(pane_id, &filtered_entries, selected)?;
                    return Ok(true);
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
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
            let status = zoxide_list_status(&search.buffer, len, selected, false);
            self.pending_action = Some(PendingAction::ZoxideList {
                pane_id,
                selected,
                entries,
                search,
            });
            self.status = status;
        }

        Ok(true)
    }

    /// 處理等待書籤按鍵時的輸入。
    pub(crate) fn handle_bookmark_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(prompt) = self.pending_bookmark.take() else {
            return Ok(true);
        };

        match key.code {
            KeyCode::Esc => {
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') => {
                self.status = String::from("normal mode");
            }
            KeyCode::Char(bookmark) => match prompt {
                BookmarkPrompt::Jump => self.jump_to_bookmark(bookmark)?,
            },
            _ => {
                self.pending_bookmark = Some(prompt);
                self.status = match prompt {
                    BookmarkPrompt::Jump => String::from("bookmark: choose an existing key"),
                };
            }
        }

        Ok(true)
    }
}
