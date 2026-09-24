//! Normal 模式導航、移動、翻頁與路徑跳轉按鍵分派。

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use super::super::keys_util::*;
use super::super::*;

impl App {
    /// 處理 Normal 模式下的導航、移動、翻頁與路徑跳轉按鍵。
    /// 回傳 `Ok(Some(bool))` 代表此鍵為導航鍵並已處理；`Ok(None)` 代表非導航鍵，交由後續處理。
    pub(crate) fn handle_normal_nav_key(&mut self, key: &KeyEvent) -> Result<Option<bool>> {
        if key_matches_plain_letter(key, 'j') {
            let count = self.take_vertical_nav_step(NavDirection::Down);
            self.current_pane_mut()?.move_down_by(count);
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'k') {
            let count = self.take_vertical_nav_step(NavDirection::Up);
            self.current_pane_mut()?.move_up_by(count);
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        self.nav_acceleration = None;

        if key.code == KeyCode::Tab {
            self.clear_pending_count();
            self.open_preview_focus();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'G') {
            if let Some(count) = self.take_pending_count() {
                self.current_pane_mut()?
                    .move_to_visible_index(count.saturating_sub(1));
                self.status = format!("jumped to item {count}");
            } else {
                self.current_pane_mut()?.move_bottom();
                self.status = String::from("jumped to bottom");
            }
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'J') {
            let step = self.take_large_move_step();
            self.current_pane_mut()?.move_down_by(step);
            self.pending_g = false;
            self.pending_y = false;
            self.status = format!("fast down: {step}");
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'K') {
            let step = self.take_large_move_step();
            self.current_pane_mut()?.move_up_by(step);
            self.pending_g = false;
            self.pending_y = false;
            self.status = format!("fast up: {step}");
            return Ok(Some(true));
        }

        if key_matches_ctrl_letter(key, 'd') {
            self.clear_pending_count();
            self.toggle_preview_diff_mode();
            return Ok(Some(true));
        }

        if key_matches_ctrl_letter(key, 'u') {
            self.clear_pending_count();
            let step = self.current_pane_mut()?.page_up();
            self.pending_g = false;
            self.pending_y = false;
            self.status = format!("half page up: {step}");
            return Ok(Some(true));
        }

        if key_matches_ctrl_letter(key, 'f') {
            self.clear_pending_count();
            let step = self.current_pane_mut()?.full_page_down();
            self.pending_g = false;
            self.pending_y = false;
            self.status = format!("page down: {step}");
            return Ok(Some(true));
        }

        if key_matches_ctrl_letter(key, 'b') {
            self.clear_pending_count();
            let step = self.current_pane_mut()?.full_page_up();
            self.pending_g = false;
            self.pending_y = false;
            self.status = format!("page up: {step}");
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'h') {
            self.clear_pending_count();
            let pane_id = self.focused_pane;
            let is_loading = self.directory_load_jobs.contains_key(&pane_id);
            let (previous_cwd, cached_chunk, previous_selected) = {
                let pane = self
                    .panes
                    .get(&pane_id)
                    .ok_or_else(|| anyhow::anyhow!("focused pane not found"))?;
                let cwd = pane.cwd.clone();
                let selected = pane.selected_entry().map(|e| e.path.clone());
                let chunk = if !is_loading && !pane.entries.is_empty() {
                    if pane.entries.len() > 2000 {
                        Some(pane.entries[..2000].to_vec())
                    } else {
                        Some(pane.entries.clone())
                    }
                } else {
                    None
                };
                (cwd, chunk, selected)
            };
            if let Some(chunk) = cached_chunk {
                self.store_directory_cache(previous_cwd.clone(), chunk);
            }
            if let Some(cursor) = previous_selected {
                self.directory_cache_cursors
                    .insert(previous_cwd, Some(cursor));
            }
            if let Some((cwd, selected_path)) = self.current_pane_mut()?.begin_go_parent() {
                self.start_directory_load(pane_id, cwd, Some(selected_path));
            }
            self.track_focused_pane_cwd_in_zoxide();
            self.status = String::from("moved to parent directory");
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'l') {
            self.clear_pending_count();
            let pane_id = self.focused_pane;
            if let Some(pane) = self.panes.get_mut(&pane_id)
                && pane.is_preview_open()
            {
                let is_dir = pane.selected_entry().map(|e| e.is_dir).unwrap_or(false);
                if !is_dir {
                    pane.set_preview_focused(true);
                    self.status = String::from("preview focused (press 'h' to return to list)");
                    self.pending_g = false;
                    self.pending_y = false;
                    return Ok(Some(true));
                }
            }
            if let Some(entry) = self.panes.get(&pane_id).and_then(|p| p.selected_entry())
                && entry.is_dir
                && let Some((task_id, title, progress)) = self.active_file_job_for_path(&entry.path)
            {
                let pct_str = progress.map(|p| format!(" ({p}%)")).unwrap_or_default();
                self.status = format!(
                    "cannot enter '{}': transfer in progress [task #{task_id}: {title}{pct_str}]",
                    entry.display_name()
                );
                self.pending_g = false;
                self.pending_y = false;
                return Ok(Some(true));
            }
            let is_loading = self.directory_load_jobs.contains_key(&pane_id);
            let (previous_cwd, cached_chunk, previous_selected) = {
                let pane = self
                    .panes
                    .get(&pane_id)
                    .ok_or_else(|| anyhow::anyhow!("focused pane not found"))?;
                let cwd = pane.cwd.clone();
                let selected = pane.selected_entry().map(|e| e.path.clone());
                let chunk = if !is_loading && !pane.entries.is_empty() {
                    if pane.entries.len() > 2000 {
                        Some(pane.entries[..2000].to_vec())
                    } else {
                        Some(pane.entries.clone())
                    }
                } else {
                    None
                };
                (cwd, chunk, selected)
            };
            if let Some(chunk) = cached_chunk {
                self.store_directory_cache(previous_cwd.clone(), chunk);
            }
            if let Some(cursor) = previous_selected {
                self.directory_cache_cursors
                    .insert(previous_cwd, Some(cursor));
            }
            if let Some(cwd) = self.current_pane_mut()?.begin_enter_selected() {
                self.start_directory_load(pane_id, cwd, None);
            }
            self.track_focused_pane_cwd_in_zoxide();
            self.status = String::from("opened directory");
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'g') {
            self.pending_y = false;
            self.open_go_picker();
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'z') {
            self.clear_pending_count();
            self.open_fzf_jump();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'Z') {
            self.clear_pending_count();
            self.open_zoxide_list();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'n') {
            if self
                .panes
                .get(&self.focused_pane)
                .is_some_and(|pane| pane.has_list_find())
            {
                let count = self.take_count_or_one();
                self.status = self.jump_list_find_match(true, count)?;
            } else {
                self.clear_pending_count();
            }
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'N') {
            if self
                .panes
                .get(&self.focused_pane)
                .is_some_and(|pane| pane.has_list_find())
            {
                let count = self.take_count_or_one();
                self.status = self.jump_list_find_match(false, count)?;
            } else {
                self.clear_pending_count();
            }
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key.code == KeyCode::Char('\'') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.pending_bookmark = Some(BookmarkPrompt::Jump);
            self.status = String::from("bookmark: press a key to jump");
            return Ok(Some(true));
        }

        Ok(None)
    }
}
