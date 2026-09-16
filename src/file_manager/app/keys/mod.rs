#![allow(unused_imports)]

use super::*;

mod bookmarks;
mod command;
mod create;
mod dialogs;
mod diff;
mod easymotion;
mod help;
mod pickers;
mod preview;
mod rename;
mod search;
mod tasks;
mod trash;
mod visual;

pub(crate) use bookmarks::*;
pub(crate) use command::*;
pub(crate) use create::*;
pub(crate) use dialogs::*;
pub(crate) use diff::*;
pub(crate) use easymotion::*;
pub(crate) use help::*;
pub(crate) use pickers::*;
pub(crate) use preview::*;
pub(crate) use rename::*;
pub(crate) use search::*;
pub(crate) use tasks::*;
pub(crate) use trash::*;
pub(crate) use visual::*;

impl App {
    /// 嘗試把目前按鍵視為 count prefix 的下一個數字。
    ///
    /// 規則：
    /// - `1..=9` 永遠可以開始或延續 count。
    /// - `0` 只有在已經有 count 時，才會被視為後續位數。
    pub(crate) fn capture_pending_count_digit(&mut self, key: &KeyEvent) -> bool {
        if !key.modifiers.is_empty() {
            return false;
        }

        let KeyCode::Char(ch) = key.code else {
            return false;
        };

        if !ch.is_ascii_digit() {
            return false;
        }
        if ch == '0' && self.pending_count.is_none() {
            return false;
        }

        let digit = ch.to_digit(10).unwrap_or(0) as usize;
        let next = self
            .pending_count
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(digit);
        self.pending_count = Some(next);
        self.status = format!("count: {next}");
        true
    }

    /// 取出目前暫存的 count prefix。
    pub(crate) fn take_pending_count(&mut self) -> Option<usize> {
        self.pending_count.take()
    }

    /// 取出目前暫存的 count；若沒有則回傳 1。
    pub(crate) fn take_count_or_one(&mut self) -> usize {
        self.take_pending_count().unwrap_or(1).max(1)
    }

    /// 取出目前 count，並轉成固定大步長移動的實際步數。
    pub(crate) fn take_large_move_step(&mut self) -> usize {
        self.take_count_or_one()
            .saturating_mul(self.config.navigation.fast_move_step.max(1))
    }

    /// 取出目前 count，並轉成一般彈窗列表使用的 page 步長。
    pub(crate) fn take_panel_page_step(&mut self) -> usize {
        self.take_count_or_one()
            .saturating_mul(self.config.navigation.panel_page_step.max(1))
    }

    /// 清除目前暫存的 count prefix。
    pub(crate) fn clear_pending_count(&mut self) {
        self.pending_count = None;
    }

    /// 清除和一般移動相關的暫存狀態，例如 count、pending g、pending y。
    pub(crate) fn reset_pending_motion_state(&mut self) {
        self.clear_pending_count();
        self.pending_g = false;
        self.pending_y = false;
    }

    /// 依照目前互動狀態，把一個鍵盤事件分派給唯一的處理流程。
    ///
    /// 參數：`key: KeyEvent`，已由終端事件迴圈過濾成 Press/Repeat 的按鍵。
    /// 回傳：`Result<bool>`；`true` 代表繼續執行，`false` 只由 quit 流程回傳，錯誤
    /// 則向上交給事件迴圈統一還原 terminal。
    ///
    /// 分派順序不可隨意調換：暫時面板與文字輸入必須先攔截按鍵，否則使用者在
    /// command 輸入 `d` 時可能同時觸發刪除；一般列表快捷鍵永遠是最後一層。
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // Help 被允許從任何上下文打開（F1 永遠支援；~ 僅在非文字輸入狀態下生效，
        // 避免在 filter、goto 或 rename 中輸入 ~ 被誤當成開啟說明）；
        // `help_return` 會保存原上下文，關閉說明後才能回到原 panel/輸入流程。
        if key.code == KeyCode::F(1) || (key_matches_tilde(&key) && !self.is_text_input_active()) {
            if matches!(self.pending_action, Some(PendingAction::HelpPanel { .. })) {
                return self.handle_pending_action_key(key);
            }
            self.open_help_from_current();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_question_mark(&key) && !self.is_text_input_active() {
            if matches!(self.pending_action, Some(PendingAction::HelpPanel { .. })) {
                return self.handle_pending_action_key(key);
            }
            self.open_cheatsheet_from_current();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if self.pending_action.is_some() {
            return self.handle_pending_action_key(key);
        }
        // 下列輸入狀態互斥，依 UI 層級交給各自 handler；handler 內再共用
        // `edit_text_buffer`，以維持 Insert/Normal、Unicode 游標與 Esc 行為一致。
        if self.filter.as_ref().is_some_and(|filter| filter.editing) {
            return self.handle_filter_input_key(key);
        }
        if self
            .preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
        {
            return self.handle_preview_search_input_key(key);
        }
        if self.list_find.is_some() {
            return self.handle_list_find_input_key(key);
        }
        if self.global_search.is_some() {
            return self.handle_global_search_key(key);
        }
        if self.visual_selection.is_some() {
            return self.handle_visual_selection_key(key);
        }
        if self.command_mode {
            return self.handle_command_key(key);
        }
        if self.pending_bookmark.is_some() {
            return self.handle_bookmark_key(key);
        }
        // UNC 目錄可能被 Windows 網路層長時間阻塞。背景跳轉期間允許 Esc 立即
        // 捨棄接收端並回到一般操作，不必等待作業系統的檔案系統呼叫結束。
        if key.code == KeyCode::Esc && self.active_network_goto_task_id.is_some() {
            self.cancel_network_goto("cancelled by user");
            return Ok(true);
        }
        // 不在輸入或暫時 UI 時，數字與跨 panel 快捷鍵才有意義。這段必須放在
        // count prefix 之前，否則多 panel 下按 2 會被誤解成 `2j` 的前綴。
        if self.panes.len() > 1
            && let Some(target_pane_id) = plain_digit_target_pane_id(&key)
        {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.focus_pane_by_id(target_pane_id);
            return Ok(true);
        }
        if let Some(target_pane_id) = ctrl_digit_target_pane_id(&key) {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.focus_pane_by_id(target_pane_id);
            return Ok(true);
        }
        if key_matches_ctrl_letter(&key, 's') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.split_current(SplitDirection::Horizontal)?;
            return Ok(true);
        }
        if key_matches_ctrl_letter(&key, 'v') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.split_current(SplitDirection::Vertical)?;
            return Ok(true);
        }
        if self
            .panes
            .get(&self.focused_pane)
            .is_some_and(PaneState::is_preview_active)
        {
            return self.handle_preview_key(key);
        }
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }
        if key.code == KeyCode::Tab {
            self.clear_pending_count();
            self.open_preview_focus();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
            self.open_selected_with_picker()?;
            self.reset_pending_motion_state();
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'O') {
            self.open_selected_with_picker()?;
            self.reset_pending_motion_state();
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
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
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V') {
            self.open_visual_selection()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'c') {
            self.open_copy_picker()?;
            self.reset_pending_motion_state();
            return Ok(true);
        }

        let should_continue = match key.code {
            _ if key_matches_plain_letter(&key, 'q') => false,
            KeyCode::Char(':')
                if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.open_prefilled_command("");
                true
            }
            KeyCode::Char(';') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.open_prefilled_command("");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'p') => {
                self.open_prefilled_command("panel ");
                true
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_down_by(count);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_down_by(step);
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("fast down: {step}");
                true
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_up_by(count);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_up_by(step);
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("fast up: {step}");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                self.clear_pending_count();
                self.toggle_preview_diff_mode();
                true
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                self.clear_pending_count();
                let step = self.current_pane_mut()?.page_up();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("half page up: {step}");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'f') => {
                self.clear_pending_count();
                let step = self.current_pane_mut()?.full_page_down();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("page down: {step}");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'b') => {
                self.clear_pending_count();
                let step = self.current_pane_mut()?.full_page_up();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("page up: {step}");
                true
            }
            _ if key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                let pane_id = self.focused_pane;
                let is_loading = self.directory_load_jobs.contains_key(&pane_id);
                let previous_cwd = self.current_pane_mut()?.cwd.clone();
                let previous_entries = self.current_pane_mut()?.entries.as_slice();
                if !is_loading && !previous_entries.is_empty() {
                    let cached_chunk = if previous_entries.len() > 2000 {
                        previous_entries[..2000].to_vec()
                    } else {
                        previous_entries.to_vec()
                    };
                    self.directory_entry_cache
                        .insert(previous_cwd, cached_chunk);
                }
                if let Some((cwd, selected_path)) = self.current_pane_mut()?.begin_go_parent() {
                    self.start_directory_load(pane_id, cwd, Some(selected_path));
                }
                self.track_focused_pane_cwd_in_zoxide();
                self.status = String::from("moved to parent directory");
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'l') => {
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
                        return Ok(true);
                    }
                }
                if let Some(entry) = self.panes.get(&pane_id).and_then(|p| p.selected_entry())
                    && entry.is_dir
                    && let Some((task_id, title, progress)) =
                        self.active_file_job_for_path(&entry.path)
                {
                    let pct_str = progress.map(|p| format!(" ({p}%)")).unwrap_or_default();
                    self.status = format!(
                        "cannot enter '{}': transfer in progress [task #{task_id}: {title}{pct_str}]",
                        entry.display_name()
                    );
                    self.pending_g = false;
                    self.pending_y = false;
                    return Ok(true);
                }
                let is_loading = self.directory_load_jobs.contains_key(&pane_id);
                let previous_cwd = self.current_pane_mut()?.cwd.clone();
                let previous_entries = self.current_pane_mut()?.entries.as_slice();
                if !is_loading && !previous_entries.is_empty() {
                    let cached_chunk = if previous_entries.len() > 2000 {
                        previous_entries[..2000].to_vec()
                    } else {
                        previous_entries.to_vec()
                    };
                    self.directory_entry_cache
                        .insert(previous_cwd, cached_chunk);
                }
                if let Some(cwd) = self.current_pane_mut()?.begin_enter_selected() {
                    self.start_directory_load(pane_id, cwd, None);
                }
                self.track_focused_pane_cwd_in_zoxide();
                self.status = String::from("opened directory");
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Enter => {
                self.clear_pending_count();
                self.open_selected_with_default()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'o') => {
                self.clear_pending_count();
                self.open_selected_with_default()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                self.pending_y = false;
                self.open_go_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 'd') => {
                self.clear_pending_count();
                self.start_delete_confirmation(false);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'D') => {
                self.clear_pending_count();
                self.start_delete_confirmation(true);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'r') => {
                self.clear_pending_count();
                self.start_rename();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'R') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_prefilled_command("rename-regex ");
                true
            }
            _ if key_matches_plain_letter(&key, 'z') => {
                self.clear_pending_count();
                self.open_fzf_jump();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'Z') => {
                self.clear_pending_count();
                self.open_zoxide_list();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'U') => {
                self.clear_pending_count();
                self.clear_marks_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'A') => {
                self.clear_pending_count();
                self.mark_all_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_ctrl_letter(&key, 'r') => {
                self.clear_pending_count();
                self.invert_marks_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char(' ') => {
                self.clear_pending_count();
                self.toggle_mark_selected_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('/') => {
                self.clear_pending_count();
                self.open_list_find_input();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char(',') => {
                self.clear_pending_count();
                self.open_sort_picker();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'w') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_window_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 'f') => {
                self.clear_pending_count();
                self.open_filter_input(FilterMode::Normal);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'F') => {
                self.clear_pending_count();
                self.open_filter_input(FilterMode::Fuzzy);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'e') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_easymotion();
                true
            }
            _ if key_matches_plain_letter(&key, 's') => {
                self.clear_pending_count();
                self.open_global_search()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'S') => {
                self.clear_pending_count();
                self.open_content_search()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('.') => {
                self.clear_pending_count();
                self.toggle_hidden_files()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'a') => {
                self.clear_pending_count();
                self.start_create_entry();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'x') => {
                self.clear_pending_count();
                self.cut_selected();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'X') => {
                self.clear_pending_count();
                self.clear_clipboard(ClipboardOperation::Cut);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'p') => {
                self.clear_pending_count();
                self.paste_into_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'P') => {
                self.clear_pending_count();
                self.paste_into_focused_pane_with_overwrite()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'u') => {
                self.clear_pending_count();
                self.undo_latest_file_operation()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'n') => {
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
                true
            }
            _ if key_matches_shifted_letter(&key, 'N') => {
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
                true
            }
            _ if key_matches_plain_letter(&key, 'y') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_bookmark = None;
                self.pending_y = false;
                self.open_yank_picker();
                true
            }
            _ if key_matches_shifted_letter(&key, 'Y') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_bookmark = None;
                self.pending_y = false;
                self.clear_clipboard(ClipboardOperation::Copy);
                true
            }
            _ if key_matches_plain_letter(&key, 'b') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_bookmark_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 'm') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_linemode_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 't') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_theme_command_picker();
                true
            }
            _ if key_matches_shifted_letter(&key, 'T') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_task_panel();
                true
            }
            _ if key_matches_shifted_letter(&key, 'C') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.compress_selected_entries()?;
                true
            }
            _ if key_matches_shifted_letter(&key, 'E') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.extract_selected_archives()?;
                true
            }
            _ if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::ALT) => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_diff_matrix(None)?;
                true
            }
            KeyCode::Char('\'') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = Some(BookmarkPrompt::Jump);
                self.status = String::from("bookmark: press a key to jump");
                true
            }
            KeyCode::Esc => {
                self.reset_pending_motion_state();
                self.pending_bookmark = None;
                self.handle_escape_in_normal_mode();
                true
            }
            _ => {
                self.reset_pending_motion_state();
                self.pending_bookmark = None;
                true
            }
        };

        Ok(should_continue)
    }

    /// 處理目前 pending action 所代表的暫時面板、選單或確認視窗。
    ///
    /// 參數：`key: KeyEvent`，要交給最上層暫時 UI 的按鍵。
    /// 回傳：`Result<bool>`，暫時 UI 一律消耗事件並回傳 `true`；檔案操作失敗則回傳
    /// error。函數開頭先 `take()` action，只有需要保留 UI 的分支才放回去，因此某
    /// 分支若未重設 `pending_action`，語意就是操作完成並關閉面板。
    pub(crate) fn handle_pending_action_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut action) = self.pending_action.take() else {
            return Ok(true);
        };

        // 所有具搜尋框的列表先共用文字編輯器。Normal 模式的 h/l 不可落入下面的
        // panel 導航，否則使用者只想移動輸入游標時會意外關閉或執行選項。
        let panel_edit_result = match &mut action {
            PendingAction::TrashPanel {
                selected, search, ..
            }
            | PendingAction::HelpPanel {
                selected, search, ..
            }
            | PendingAction::TaskPanel {
                selected, search, ..
            }
            | PendingAction::BookmarkList {
                selected, search, ..
            }
            | PendingAction::ZoxideList {
                selected, search, ..
            } if search.editing => {
                let result = self.edit_text_buffer(&mut search.buffer, &key);
                if matches!(result, TextEditResult::Changed) {
                    *selected = 0;
                }
                Some(result)
            }
            _ => None,
        };
        if panel_edit_result.is_some_and(|result| {
            matches!(result, TextEditResult::Changed | TextEditResult::Consumed)
        }) {
            self.status = self.status_for_pending_action(&action)?;
            self.pending_action = Some(action);
            return Ok(true);
        }

        match action {
            PendingAction::EasyMotion {
                pane_id,
                target_char,
                labels,
            } => self.handle_easymotion_action_key(key, pane_id, target_char, labels),
            PendingAction::ToolPanel { pane_id, selected } => {
                self.handle_tool_panel_action_key(key, pane_id, selected)
            }
            PendingAction::ConfirmDelete {
                pane_id,
                target_name,
                permanent,
                warning_message,
            } => self.handle_confirm_delete_action_key(
                key,
                pane_id,
                target_name,
                permanent,
                warning_message,
            ),
            PendingAction::ConfirmPasteOverwrite {
                pane_id,
                target_name,
                entry_count,
                operation,
            } => self.handle_confirm_paste_overwrite_action_key(
                key,
                pane_id,
                target_name,
                entry_count,
                operation,
            ),
            PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                marked_ids,
                visual_anchor,
            } => self.handle_confirm_trash_action_key(
                key,
                action,
                target_name,
                entry_count,
                marked_ids,
                visual_anchor,
            ),
            PendingAction::GoPicker { pane_id } => self.handle_go_picker_action_key(key, pane_id),
            PendingAction::ThemeCommandPicker { pane_id } => {
                self.handle_theme_command_picker_action_key(key, pane_id)
            }
            PendingAction::SortPicker { pane_id } => {
                self.handle_sort_picker_action_key(key, pane_id)
            }
            PendingAction::WindowPicker { pane_id } => {
                self.handle_window_picker_action_key(key, pane_id)
            }
            PendingAction::WindowResize { pane_id } => {
                self.handle_window_resize_action_key(key, pane_id)
            }
            PendingAction::LineModePicker { pane_id } => {
                self.handle_line_mode_picker_action_key(key, pane_id)
            }
            PendingAction::YankPicker { pane_id } => {
                self.handle_yank_picker_action_key(key, pane_id)
            }
            PendingAction::ThemePicker { selected, original } => {
                self.handle_theme_picker_action_key(key, selected, original)
            }
            PendingAction::TrashPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            } => self.handle_trash_panel_action_key(
                key,
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            ),
            PendingAction::HelpPanel {
                pane_id,
                selected,
                search,
                custom_title,
                custom_entries,
            } => self.handle_help_panel_action_key(
                key,
                pane_id,
                selected,
                search,
                custom_title,
                custom_entries,
            ),
            PendingAction::TaskPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            } => self.handle_task_panel_action_key(
                key,
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            ),
            PendingAction::BookmarkPicker { pane_id } => {
                self.handle_bookmark_picker_action_key(key, pane_id)
            }
            PendingAction::BookmarkList {
                pane_id,
                selected,
                mode,
                search,
            } => self.handle_bookmark_list_action_key(key, pane_id, selected, mode, search),
            PendingAction::ZoxideList {
                pane_id,
                selected,
                entries,
                search,
            } => self.handle_zoxide_list_action_key(key, pane_id, selected, entries, search),
            PendingAction::CopyPicker {
                pane_id,
                target,
                selected,
            } => self.handle_copy_picker_action_key(key, pane_id, target, selected),
            PendingAction::OpenPicker {
                pane_id,
                target,
                selected,
                options,
            } => self.handle_open_picker_action_key(key, pane_id, target, selected, options),
            PendingAction::Rename {
                pane_id,
                original_name,
                buffer,
                cursor,
                mode,
            } => self.handle_rename_action_key(key, pane_id, original_name, buffer, cursor, mode),
            PendingAction::CreateEntry {
                pane_id,
                buffer,
                cursor,
                mode,
            } => self.handle_create_entry_action_key(key, pane_id, buffer, cursor, mode),
            PendingAction::RegexRename {
                pane_id,
                pattern,
                replacement,
                selected,
                previews,
            } => self.handle_regex_rename_action_key(
                key,
                pane_id,
                pattern,
                replacement,
                selected,
                previews,
            ),
            PendingAction::DiffMatrix(state) => self.handle_diff_matrix_action_key(key, state),
        }
    }

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

    /// 處理終端送入的 bracketed paste 事件（例如滑鼠右鍵貼上或終端快捷鍵貼上）。
    ///
    /// 依目前焦點所在的輸入框（command mode、filter、preview search、list find、
    /// global search、rename、create entry 或 modal search）直接貼入文字。
    pub(crate) fn handle_bracketed_paste(&mut self, text: &str) -> Result<bool> {
        let text = sanitize_pasted_text(text);
        if text.is_empty() {
            return Ok(true);
        }

        // 1. PendingAction 專屬輸入框
        if let Some(mut action) = self.pending_action.take() {
            match &mut action {
                PendingAction::Rename {
                    buffer,
                    cursor,
                    mode,
                    ..
                } => {
                    insert_str(buffer, cursor, &text);
                    self.status = match mode {
                        RenameMode::Insert => String::from("rename: insert"),
                        RenameMode::Normal => String::from("rename: normal"),
                    };
                    self.pending_action = Some(action);
                    return Ok(true);
                }
                PendingAction::CreateEntry {
                    buffer,
                    cursor,
                    mode,
                    ..
                } => {
                    insert_str(buffer, cursor, &text);
                    self.status = match mode {
                        RenameMode::Insert => create_status_label("insert"),
                        RenameMode::Normal => create_status_label("normal"),
                    };
                    self.pending_action = Some(action);
                    return Ok(true);
                }
                PendingAction::TrashPanel {
                    selected, search, ..
                }
                | PendingAction::HelpPanel {
                    selected, search, ..
                }
                | PendingAction::TaskPanel {
                    selected, search, ..
                }
                | PendingAction::BookmarkList {
                    selected, search, ..
                }
                | PendingAction::ZoxideList {
                    selected, search, ..
                } if search.editing => {
                    insert_str(&mut search.buffer, &mut self.text_input_cursor, &text);
                    *selected = 0;
                    self.status = self.status_for_pending_action(&action)?;
                    self.pending_action = Some(action);
                    return Ok(true);
                }
                _ => {
                    self.pending_action = Some(action);
                }
            }
        }

        // 2. Filter
        if self.filter.as_ref().is_some_and(|f| f.editing) {
            let mut filter = self.filter.take().unwrap();
            insert_str(&mut filter.buffer, &mut self.text_input_cursor, &text);
            self.apply_filter_buffer(&filter);
            self.status = format_filter_status(&filter);
            self.filter = Some(filter);
            return Ok(true);
        }

        // 3. Preview search
        if self.preview_search.as_ref().is_some_and(|s| s.editing) {
            let mut search = self.preview_search.take().unwrap();
            insert_str(&mut search.buffer, &mut self.text_input_cursor, &text);
            self.apply_preview_search_buffer(&search);
            self.status =
                preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
            self.preview_search = Some(search);
            return Ok(true);
        }

        // 4. List find
        if let Some(mut find) = self.list_find.take() {
            insert_str(&mut find.buffer, &mut self.text_input_cursor, &text);
            self.apply_list_find_buffer(&find);
            self.status = list_find_status(&find.buffer, self.list_find_match_count(find.pane_id));
            self.list_find = Some(find);
            return Ok(true);
        }

        // 5. Global search
        if let Some(mut search) = self.global_search.take() {
            if search.editing {
                insert_str(&mut search.buffer, &mut self.text_input_cursor, &text);
                search.searched = false;
                search.loading = false;
                search.selected = 0;
                search.results.clear();
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
                return Ok(true);
            } else if search.filter.editing {
                insert_str(
                    &mut search.filter.buffer,
                    &mut self.text_input_cursor,
                    &text,
                );
                search.selected = 0;
                let visible =
                    filtered_global_search_entries(&search.results, &search.filter.buffer);
                search.selected = search.selected.min(visible.len().saturating_sub(1));
                self.status = global_search_filter_status(&search.filter, visible.len());
                self.global_search = Some(search);
                return Ok(true);
            } else {
                self.global_search = Some(search);
            }
        }

        // 6. Command mode
        if self.command_mode {
            insert_str(&mut self.command_buffer, &mut self.text_input_cursor, &text);
            self.command_suggestion_selected = 0;
            self.command_completion_cycle = None;
            return Ok(true);
        }

        Ok(true)
    }
}
