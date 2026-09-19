//! Normal 模式檔案操作、編輯模式觸發、快捷選單與確認按鍵分派。

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::super::keys_util::*;
use super::super::*;

impl App {
    /// 處理 Normal 模式下的檔案操作、選擇器喚起與模式切換按鍵。
    /// 回傳 `Ok(Some(bool))` 代表此鍵為操作鍵並已處理；`Ok(None)` 代表非操作鍵。
    pub(crate) fn handle_normal_ops_key(&mut self, key: &KeyEvent) -> Result<Option<bool>> {
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
            self.open_selected_with_picker()?;
            self.reset_pending_motion_state();
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'O') {
            self.open_selected_with_picker()?;
            self.reset_pending_motion_state();
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'v') || key_matches_shifted_letter(key, 'V') {
            self.open_visual_selection()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'c') {
            self.open_copy_picker()?;
            self.reset_pending_motion_state();
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'q') {
            return Ok(Some(false));
        }

        if (key.code == KeyCode::Char(':')
            && (key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::SHIFT)))
            || (key.code == KeyCode::Char(';') && key.modifiers.contains(KeyModifiers::SHIFT))
        {
            self.open_prefilled_command("");
            return Ok(Some(true));
        }

        if key_matches_ctrl_letter(key, 'p') {
            self.open_prefilled_command("panel ");
            return Ok(Some(true));
        }

        if key.code == KeyCode::Enter || key_matches_plain_letter(key, 'o') {
            self.clear_pending_count();
            self.open_selected_with_default()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'd') {
            self.clear_pending_count();
            self.start_delete_confirmation(false);
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'D') {
            self.clear_pending_count();
            self.start_delete_confirmation(true);
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'r') {
            self.clear_pending_count();
            self.start_rename();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'R') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_prefilled_command("rename-regex ");
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'U') {
            self.clear_pending_count();
            self.clear_marks_in_focused_pane()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'A') {
            self.clear_pending_count();
            self.mark_all_in_focused_pane()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_ctrl_letter(key, 'r') {
            self.clear_pending_count();
            self.invert_marks_in_focused_pane()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key.code == KeyCode::Char(' ') {
            self.clear_pending_count();
            self.toggle_mark_selected_in_focused_pane()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key.code == KeyCode::Char('/') {
            self.clear_pending_count();
            self.open_list_find_input();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key.code == KeyCode::Char(',') {
            self.clear_pending_count();
            self.open_sort_picker();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'w') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_window_picker();
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'f') {
            self.clear_pending_count();
            self.open_filter_input(FilterMode::Normal);
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'F') {
            self.clear_pending_count();
            self.open_filter_input(FilterMode::Fuzzy);
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'e') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_easymotion();
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 's') {
            self.clear_pending_count();
            self.open_global_search()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'S') {
            self.clear_pending_count();
            self.open_content_search()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key.code == KeyCode::Char('.') {
            self.clear_pending_count();
            self.toggle_hidden_files()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'a') {
            self.clear_pending_count();
            self.start_create_entry();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'x') {
            self.clear_pending_count();
            self.cut_selected();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'X') {
            self.clear_pending_count();
            self.clear_clipboard(ClipboardOperation::Cut);
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'p') {
            self.clear_pending_count();
            self.paste_into_focused_pane()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'P') {
            self.clear_pending_count();
            self.paste_into_focused_pane_with_overwrite()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'u') {
            self.clear_pending_count();
            self.undo_latest_file_operation()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'y') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_bookmark = None;
            self.pending_y = false;
            self.open_yank_picker();
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'Y') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_bookmark = None;
            self.pending_y = false;
            self.clear_clipboard(ClipboardOperation::Copy);
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'b') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_bookmark_picker();
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 'm') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_linemode_picker();
            return Ok(Some(true));
        }

        if key_matches_plain_letter(key, 't') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_theme_command_picker();
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'T') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_task_panel();
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'C') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.compress_selected_entries()?;
            return Ok(Some(true));
        }

        if key_matches_shifted_letter(key, 'E') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.extract_selected_archives()?;
            return Ok(Some(true));
        }

        if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::ALT) {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.open_diff_matrix(None)?;
            return Ok(Some(true));
        }

        if key.code == KeyCode::Esc {
            self.reset_pending_motion_state();
            self.pending_bookmark = None;
            self.handle_escape_in_normal_mode();
            return Ok(Some(true));
        }

        Ok(None)
    }

    /// 處理 Normal 模式下的 Escape 按鍵行為（關閉過濾、清除查找、取消標記等）。
    pub(crate) fn handle_escape_in_normal_mode(&mut self) {
        if let Some(filter) = self.filter.take() {
            if filter.editing {
                self.filter = Some(FilterState {
                    editing: false,
                    ..filter
                });
                self.status = String::from("filter active");
            } else {
                if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
                    pane.clear_filter();
                }
                self.status = String::from("normal mode");
            }
            return;
        }

        if self.clear_list_find_if_active() {
            return;
        }

        if self.has_any_marks() {
            self.clear_all_marks();
            return;
        }

        self.status = String::from("normal mode");
    }
}
