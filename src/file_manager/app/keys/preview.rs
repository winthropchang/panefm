use super::*;

impl App {
    /// 處理 preview mode 的鍵盤輸入，讓使用者可以專心在預覽區捲動內容。
    pub(crate) fn handle_preview_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }
        if key.code == KeyCode::Tab {
            if self.clear_preview_search_if_active() {
                self.clear_pending_count();
                self.pending_g = false;
                return Ok(true);
            }
            self.current_pane_mut()?.set_preview_open(false);
            self.reset_pending_motion_state();
            self.status = String::from("normal mode");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'N') {
            let count = self.take_count_or_one();
            self.pending_g = false;
            self.status = self.jump_preview_match(false, count)?;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'J') {
            let step = self.take_large_move_step();
            self.current_pane_mut()?.scroll_preview_down(step);
            self.pending_g = false;
            self.status = format!("preview: fast down {step}");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'K') {
            let step = self.take_large_move_step();
            self.current_pane_mut()?.scroll_preview_up(step);
            self.pending_g = false;
            self.status = format!("preview: fast up {step}");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            let pending = self.take_pending_count();
            let pane = self.current_pane_mut()?;
            if let Some(count) = pending {
                pane.move_preview_cursor_to_line(count);
                let line_no = pane.preview_cursor + 1;
                let total = pane.preview_total_lines().max(1);
                self.status = format!("preview: line {line_no}/{total}");
            } else {
                pane.scroll_preview_bottom();
                self.status = String::from("preview: bottom");
            }
            self.pending_g = false;
            return Ok(true);
        }
        if key.code == KeyCode::Char(']') {
            let count = self.take_count_or_one();
            let (current_name, current_pos, total) = {
                let pane = self.current_pane_mut()?;
                pane.move_down_by(count);
                let name = pane
                    .selected_entry()
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                let total = pane.visible_indices.len();
                let current_pos = pane.selected + 1;
                (name, current_pos, total)
            };
            self.clear_preview_search_if_active();
            self.pending_g = false;
            self.status = format!("preview: {current_name} ({current_pos}/{total})");
            return Ok(true);
        }
        if key.code == KeyCode::Char('[') {
            let count = self.take_count_or_one();
            let (current_name, current_pos, total) = {
                let pane = self.current_pane_mut()?;
                pane.move_up_by(count);
                let name = pane
                    .selected_entry()
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                let total = pane.visible_indices.len();
                let current_pos = pane.selected + 1;
                (name, current_pos, total)
            };
            self.clear_preview_search_if_active();
            self.pending_g = false;
            self.status = format!("preview: {current_name} ({current_pos}/{total})");
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc => {
                if self.clear_preview_search_if_active() {
                    self.clear_pending_count();
                    self.pending_g = false;
                    return Ok(true);
                }
                self.current_pane_mut()?.set_preview_focused(false);
                self.reset_pending_motion_state();
                self.status = String::from("file list (preview open)");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                if self.clear_preview_search_if_active() {
                    self.clear_pending_count();
                    self.pending_g = false;
                    return Ok(true);
                }
                self.current_pane_mut()?.set_preview_focused(false);
                self.reset_pending_motion_state();
                self.status = String::from("file list (preview open)");
            }
            KeyCode::Char('/') => {
                self.clear_pending_count();
                self.open_preview_search_input();
                self.pending_g = false;
            }
            _ if key_matches_plain_letter(&key, 'n') => {
                let count = self.take_count_or_one();
                self.pending_g = false;
                self.status = self.jump_preview_match(true, count)?;
            }
            _ if key_matches_plain_letter(&key, 'p') => {
                let count = self.take_count_or_one();
                self.pending_g = false;
                self.status = self.jump_preview_match(false, count)?;
            }
            KeyCode::Down => {
                let count = self.take_count_or_one();
                let pane = self.current_pane_mut()?;
                pane.move_preview_cursor_down(count);
                let line_no = pane.preview_cursor + 1;
                let total = pane.preview_total_lines().max(1);
                self.pending_g = false;
                self.status = format!("preview: line {line_no}/{total}");
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                let pane = self.current_pane_mut()?;
                pane.move_preview_cursor_down(count);
                let line_no = pane.preview_cursor + 1;
                let total = pane.preview_total_lines().max(1);
                self.pending_g = false;
                self.status = format!("preview: line {line_no}/{total}");
            }
            KeyCode::Up => {
                let count = self.take_count_or_one();
                let pane = self.current_pane_mut()?;
                pane.move_preview_cursor_up(count);
                let line_no = pane.preview_cursor + 1;
                let total = pane.preview_total_lines().max(1);
                self.pending_g = false;
                self.status = format!("preview: line {line_no}/{total}");
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                let pane = self.current_pane_mut()?;
                pane.move_preview_cursor_up(count);
                let line_no = pane.preview_cursor + 1;
                let total = pane.preview_total_lines().max(1);
                self.pending_g = false;
                self.status = format!("preview: line {line_no}/{total}");
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                let pending_line = self.pending_count;
                if self.pending_g {
                    let pending = self.take_pending_count();
                    let pane = self.current_pane_mut()?;
                    if let Some(count) = pending {
                        pane.move_preview_cursor_to_line(count);
                        let line_no = pane.preview_cursor + 1;
                        let total = pane.preview_total_lines().max(1);
                        self.status = format!("preview: line {line_no}/{total}");
                    } else {
                        pane.scroll_preview_top();
                        self.status = String::from("preview: top");
                    }
                    self.pending_g = false;
                } else {
                    self.pending_g = true;
                    self.status = if let Some(count) = pending_line {
                        format!("preview: pending {count}g")
                    } else {
                        String::from("preview: pending g")
                    };
                }
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                self.clear_pending_count();
                self.toggle_preview_diff_mode();
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                self.clear_pending_count();
                self.current_pane_mut()?.page_preview_up();
                self.pending_g = false;
                self.status = String::from("preview: half page up");
            }
            _ if key_matches_ctrl_letter(&key, 'f') => {
                self.clear_pending_count();
                self.current_pane_mut()?.full_page_preview_down();
                self.pending_g = false;
                self.status = String::from("preview: page down");
            }
            _ if key_matches_ctrl_letter(&key, 'b') => {
                self.clear_pending_count();
                self.current_pane_mut()?.full_page_preview_up();
                self.pending_g = false;
                self.status = String::from("preview: page up");
            }
            KeyCode::PageDown => {
                self.clear_pending_count();
                self.current_pane_mut()?.full_page_preview_down();
                self.pending_g = false;
                self.status = String::from("preview: page down");
            }
            KeyCode::PageUp => {
                self.clear_pending_count();
                self.current_pane_mut()?.full_page_preview_up();
                self.pending_g = false;
                self.status = String::from("preview: page up");
            }
            KeyCode::Home => {
                self.clear_pending_count();
                self.current_pane_mut()?.scroll_preview_top();
                self.pending_g = false;
                self.status = String::from("preview: top");
            }
            KeyCode::End => {
                self.clear_pending_count();
                self.current_pane_mut()?.scroll_preview_bottom();
                self.pending_g = false;
                self.status = String::from("preview: bottom");
            }
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
        }

        Ok(true)
    }

    /// 處理 preview search 輸入框中的鍵盤輸入，並在每次輸入後立即更新命中位置。
    pub(crate) fn handle_preview_search_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.preview_search.take() else {
            return Ok(true);
        };

        let edit_result = self.edit_text_buffer(&mut search.buffer, &key);
        if matches!(edit_result, TextEditResult::Changed) {
            self.apply_preview_search_buffer(&search);
            self.status =
                preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
            self.preview_search = Some(search);
            return Ok(true);
        }
        if matches!(edit_result, TextEditResult::Consumed) {
            self.preview_search = Some(search);
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc if search.buffer.trim().is_empty() => {
                self.apply_preview_search_buffer(&search);
                self.preview_search = None;
                self.status = String::from("preview mode");
            }
            KeyCode::Esc | KeyCode::Enter => {
                search.editing = false;
                self.status = if search.buffer.is_empty() {
                    String::from("preview mode")
                } else {
                    format!(
                        "preview search locked: {} ({})",
                        search.buffer,
                        self.preview_match_count(search.pane_id)
                    )
                };
                self.preview_search = Some(search);
            }
            _ => {
                self.preview_search = Some(search);
            }
        }

        Ok(true)
    }
}
