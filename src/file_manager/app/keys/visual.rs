use super::*;

impl App {
    /// 處理 visual selection 模式下的鍵盤輸入。
    pub(crate) fn handle_visual_selection_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V') {
            self.clear_pending_count();
            self.nav_acceleration = None;
            self.commit_visual_selection()?;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            self.nav_acceleration = None;
            if let Some(count) = self.take_pending_count() {
                self.current_pane_mut()?
                    .move_to_visible_index(count.saturating_sub(1));
            } else {
                self.current_pane_mut()?.move_bottom();
            }
            self.sync_visual_selection_cursor();
            self.pending_g = false;
            self.status = self.visual_status_label();
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc => {
                self.clear_pending_count();
                self.nav_acceleration = None;
                self.commit_visual_selection()?;
            }
            _ if key_matches_plain_letter(&key, 'q') => {
                self.clear_pending_count();
                self.nav_acceleration = None;
                self.visual_selection = None;
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_vertical_nav_step(NavDirection::Down);
                self.current_pane_mut()?.move_down_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_vertical_nav_step(NavDirection::Up);
                self.current_pane_mut()?.move_up_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_down_by(step);
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_up_by(step);
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                self.clear_pending_count();
                self.current_pane_mut()?.page_down();
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                self.clear_pending_count();
                self.current_pane_mut()?.page_up();
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                let pending_line = self.pending_count;
                if self.pending_g {
                    if let Some(count) = self.take_pending_count() {
                        self.current_pane_mut()?
                            .move_to_visible_index(count.saturating_sub(1));
                    } else {
                        self.current_pane_mut()?.move_top();
                    }
                    self.sync_visual_selection_cursor();
                    self.pending_g = false;
                    self.status = self.visual_status_label();
                } else {
                    self.pending_g = true;
                    self.status = if let Some(count) = pending_line {
                        format!("visual: pending {count}g")
                    } else {
                        String::from("visual: pending g")
                    };
                }
            }
            _ => {
                self.clear_pending_count();
                self.nav_acceleration = None;
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
        }

        Ok(true)
    }
}
