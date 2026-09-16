use super::*;

impl App {
    pub(crate) fn handle_create_entry_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
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
                    self.pending_action = Some(PendingAction::CreateEntry {
                        pane_id,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = create_status_label("insert");
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Char(_) => {
                        if let Some(c) = typed_char_from_key(&key) {
                            insert_char(&mut buffer, &mut cursor, c);
                        }
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Backspace => {
                        backspace_char(&mut buffer, &mut cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Delete => {
                        delete_char_at(&mut buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Home => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::End => {
                        cursor = buffer.chars().count();
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Enter => {
                        self.confirm_create_entry(pane_id, &buffer)?;
                    }
                    KeyCode::Esc => {
                        if buffer.trim().is_empty() {
                            self.status = String::from("create cancelled");
                        } else {
                            mode = RenameMode::Normal;
                            self.pending_action = Some(PendingAction::CreateEntry {
                                pane_id,
                                buffer,
                                cursor,
                                mode,
                            });
                            self.status = create_status_label("normal");
                        }
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
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
                    self.pending_action = Some(PendingAction::CreateEntry {
                        pane_id,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = create_status_label("insert");
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
                    self.pending_action = Some(PendingAction::CreateEntry {
                        pane_id,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = create_status_label("normal");
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
                    self.pending_action = Some(PendingAction::CreateEntry {
                        pane_id,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = create_status_label("normal");
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'h') => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Home | KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::End | KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Delete => {
                        delete_char_at(&mut buffer, cursor);
                        cursor = cursor.min(rename_line_end_cursor(&buffer));
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'x') => {
                        delete_char_at(&mut buffer, cursor);
                        cursor = cursor.min(rename_line_end_cursor(&buffer));
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'w') => {
                        cursor = rename_next_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'b') => {
                        cursor = rename_previous_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'e') => {
                        cursor = rename_word_end(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'i') => {
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    _ if key_matches_plain_letter(&key, 'a') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Enter => {
                        self.confirm_create_entry(pane_id, &buffer)?;
                    }
                    KeyCode::Esc => {
                        self.status = String::from("create cancelled");
                    }
                    _ if key_matches_plain_letter(&key, 'q') => {
                        self.status = String::from("create cancelled");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
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
}
