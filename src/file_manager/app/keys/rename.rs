use super::*;

impl App {
    pub(crate) fn handle_rename_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        original_name: String,
        mut buffer: String,
        mut cursor: usize,
        mut mode: RenameMode,
    ) -> Result<bool> {
        match mode {
            RenameMode::Insert => {
                if key_matches_ctrl_letter(&key, 'v') {
                    if let Some(text) = read_text_from_system_clipboard() {
                        let text = sanitize_pasted_text(&text);
                        if !text.is_empty() {
                            insert_str(&mut buffer, &mut cursor, &text);
                        }
                    }
                    self.pending_action = Some(PendingAction::Rename {
                        pane_id,
                        original_name,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = String::from("rename: insert");
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Char(_) => {
                        if let Some(c) = typed_char_from_key(&key) {
                            insert_char(&mut buffer, &mut cursor, c);
                        }
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Backspace => {
                        backspace_char(&mut buffer, &mut cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Delete => {
                        delete_char_at(&mut buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Home => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::End => {
                        cursor = buffer.chars().count();
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Enter => {
                        self.confirm_rename(pane_id, &original_name, &buffer)?;
                    }
                    KeyCode::Esc => {
                        if buffer.trim().is_empty() {
                            self.status = format!("rename cancelled: {original_name}");
                        } else {
                            mode = RenameMode::Normal;
                            self.pending_action = Some(PendingAction::Rename {
                                pane_id,
                                original_name,
                                buffer,
                                cursor,
                                mode,
                            });
                            self.status = String::from("rename: normal");
                        }
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                }
            }
            RenameMode::Normal => {
                if key_matches_shifted_letter(&key, 'A') {
                    cursor = buffer.chars().count();
                    mode = RenameMode::Insert;
                    self.pending_action = Some(PendingAction::Rename {
                        pane_id,
                        original_name,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = String::from("rename: insert");
                    return Ok(true);
                }
                if key_matches_plain_letter(&key, 'p') {
                    if let Some(text) = read_text_from_system_clipboard() {
                        let text = sanitize_pasted_text(&text);
                        if !text.is_empty() {
                            cursor = move_cursor_right(&buffer, cursor);
                            insert_str(&mut buffer, &mut cursor, &text);
                            cursor = normal_cursor(&buffer, cursor.saturating_sub(1));
                        }
                    }
                    self.pending_action = Some(PendingAction::Rename {
                        pane_id,
                        original_name,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = String::from("rename: normal");
                    return Ok(true);
                }
                if key_matches_shifted_letter(&key, 'P') {
                    if let Some(text) = read_text_from_system_clipboard() {
                        let text = sanitize_pasted_text(&text);
                        if !text.is_empty() {
                            insert_str(&mut buffer, &mut cursor, &text);
                            cursor = normal_cursor(&buffer, cursor.saturating_sub(1));
                        }
                    }
                    self.pending_action = Some(PendingAction::Rename {
                        pane_id,
                        original_name,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = String::from("rename: normal");
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'h') => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Home | KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::End | KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Delete => {
                        delete_char_at(&mut buffer, cursor);
                        cursor = cursor.min(rename_line_end_cursor(&buffer));
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'x') => {
                        delete_char_at(&mut buffer, cursor);
                        cursor = cursor.min(rename_line_end_cursor(&buffer));
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'w') => {
                        cursor = rename_next_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'b') => {
                        cursor = rename_previous_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'e') => {
                        cursor = rename_word_end(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'i') => {
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    _ if key_matches_plain_letter(&key, 'a') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Enter => {
                        self.confirm_rename(pane_id, &original_name, &buffer)?;
                    }
                    KeyCode::Esc => {
                        self.status = format!("rename cancelled: {original_name}");
                    }
                    _ if key_matches_plain_letter(&key, 'q') => {
                        self.status = format!("rename cancelled: {original_name}");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                }
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_regex_rename_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        pattern: String,
        replacement: String,
        mut selected: usize,
        previews: Vec<RegexRenamePreview>,
    ) -> Result<bool> {
        let len = previews.len();
        if self.capture_pending_count_digit(&key) {
            self.pending_action = Some(PendingAction::RegexRename {
                pane_id,
                pattern,
                replacement,
                selected,
                previews,
            });
            if let Some(action) = self.pending_action.as_ref() {
                self.status = self.status_for_pending_action(action)?;
            }
            return Ok(true);
        }
        match key.code {
            KeyCode::Down => {
                if len > 0 {
                    selected = (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                }
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                if len > 0 {
                    selected = (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                }
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(self.take_count_or_one());
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                selected = selected.saturating_sub(self.take_count_or_one());
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                if len > 0 {
                    selected = (selected + self.take_panel_page_step()).min(len.saturating_sub(1));
                }
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                selected = selected.saturating_sub(self.take_panel_page_step());
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                if len > 0 {
                    selected = (selected + self.take_large_move_step()).min(len.saturating_sub(1));
                }
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                selected = selected.saturating_sub(self.take_large_move_step());
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
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
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            _ if key_matches_shifted_letter(&key, 'G') => {
                if let Some(count) = self.take_pending_count() {
                    if len > 0 {
                        selected = count.saturating_sub(1).min(len.saturating_sub(1));
                    }
                } else if len > 0 {
                    selected = len - 1;
                }
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
            KeyCode::Enter => {
                self.clear_pending_count();
                self.pending_g = false;
                self.apply_regex_rename_preview(pane_id, &previews)?;
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.apply_regex_rename_preview(pane_id, &previews)?;
            }
            KeyCode::Esc => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("regex rename cancelled");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("regex rename cancelled");
            }
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_action = Some(PendingAction::RegexRename {
                    pane_id,
                    pattern,
                    replacement,
                    selected,
                    previews,
                });
            }
        }
        if let Some(action) = self.pending_action.as_ref() {
            self.status = self.status_for_pending_action(action)?;
        }

        Ok(true)
    }
}
