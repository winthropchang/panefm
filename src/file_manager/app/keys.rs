#![allow(unused_imports)]

use super::*;

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

    /// 打開 command UI，並可選擇先填入一段命令前綴，方便使用者直接補參數。
    pub(crate) fn open_prefilled_command(&mut self, prefill: impl Into<String>) {
        self.command_mode = true;
        self.command_buffer = prefill.into();
        self.begin_text_input_at_end();
        self.command_suggestion_selected = 0;
        self.command_completion_cycle = None;
        self.status = String::from("command mode");
        self.pending_g = false;
        self.pending_y = false;
        self.pending_bookmark = None;
    }

    /// 讓新開啟的文字輸入 UI 從 Insert 模式開始，並把游標放到文字尾端。
    ///
    /// 參數：無，文字內容會從目前的 `command_buffer` 取得；其他輸入框可在設定
    /// buffer 後直接重設這兩個欄位。
    /// 回傳：`()`, 只更新共用文字編輯狀態。
    pub(crate) fn begin_text_input_at_end(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = self.command_buffer.chars().count();
    }

    /// 使用共用 Vim 規則編輯指定字串。
    ///
    /// 參數：
    /// - `buffer: &mut String`，目前輸入框的 UTF-8 文字。
    /// - `key: &KeyEvent`，正規化後要處理的鍵盤事件。
    ///
    /// 回傳：`TextEditResult`，指出內容已改變、按鍵已被模式處理，或應交回 UI
    /// 處理 Enter、Tab、空輸入框的第一次 Esc，以及 Normal 模式下第二次 Esc 等
    /// 業務行為。空輸入框沒有可供 Vim 游標修正的文字，因此 Esc 會直接交回 UI 關閉。
    pub(crate) fn edit_text_buffer(
        &mut self,
        buffer: &mut String,
        key: &KeyEvent,
    ) -> TextEditResult {
        self.text_input_cursor = self.text_input_cursor.min(buffer.chars().count());
        match self.text_input_mode {
            RenameMode::Insert => match key.code {
                KeyCode::Char(_) => {
                    if let Some(character) = typed_char_from_key(key) {
                        insert_char(buffer, &mut self.text_input_cursor, character);
                        TextEditResult::Changed
                    } else {
                        TextEditResult::Consumed
                    }
                }
                KeyCode::Backspace => {
                    backspace_char(buffer, &mut self.text_input_cursor);
                    TextEditResult::Changed
                }
                KeyCode::Delete => {
                    delete_char_at(buffer, self.text_input_cursor);
                    TextEditResult::Changed
                }
                KeyCode::Left => {
                    self.text_input_cursor = self.text_input_cursor.saturating_sub(1);
                    TextEditResult::Consumed
                }
                KeyCode::Right => {
                    self.text_input_cursor = move_cursor_right(buffer, self.text_input_cursor);
                    TextEditResult::Consumed
                }
                KeyCode::Home => {
                    self.text_input_cursor = 0;
                    TextEditResult::Consumed
                }
                KeyCode::End => {
                    self.text_input_cursor = buffer.chars().count();
                    TextEditResult::Consumed
                }
                KeyCode::Esc => {
                    if buffer.trim().is_empty() {
                        return TextEditResult::PassThrough;
                    }
                    self.text_input_mode = RenameMode::Normal;
                    self.text_input_cursor = normal_cursor(buffer, self.text_input_cursor);
                    TextEditResult::Consumed
                }
                _ => TextEditResult::PassThrough,
            },
            RenameMode::Normal => {
                if key_matches_shifted_letter(key, 'A') {
                    self.text_input_cursor = buffer.chars().count();
                    self.text_input_mode = RenameMode::Insert;
                    return TextEditResult::Consumed;
                }
                match key.code {
                    KeyCode::Left => {
                        self.text_input_cursor = self.text_input_cursor.saturating_sub(1);
                    }
                    KeyCode::Right => {
                        self.text_input_cursor = normal_move_right(buffer, self.text_input_cursor);
                    }
                    KeyCode::Home | KeyCode::Char('0') => self.text_input_cursor = 0,
                    KeyCode::End | KeyCode::Char('$') => {
                        self.text_input_cursor = rename_line_end_cursor(buffer)
                    }
                    _ if key_matches_plain_letter(key, 'h') => {
                        self.text_input_cursor = self.text_input_cursor.saturating_sub(1)
                    }
                    _ if key_matches_plain_letter(key, 'l') => {
                        self.text_input_cursor = normal_move_right(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'w') => {
                        self.text_input_cursor =
                            rename_next_word_start(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'b') => {
                        self.text_input_cursor =
                            rename_previous_word_start(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'e') => {
                        self.text_input_cursor = rename_word_end(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'i') => {
                        self.text_input_mode = RenameMode::Insert
                    }
                    _ if key_matches_plain_letter(key, 'a') => {
                        self.text_input_cursor = move_cursor_right(buffer, self.text_input_cursor);
                        self.text_input_mode = RenameMode::Insert;
                    }
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab => {
                        return TextEditResult::PassThrough;
                    }
                    _ => {}
                }
                TextEditResult::Consumed
            }
        }
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
        if key_matches_plain_letter(&key, '?') && !self.is_text_input_active() {
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
                let step = self.current_pane_mut()?.page_down();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("half page down: {step}");
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
                if let Some(entry) = self.panes.get(&pane_id).and_then(|p| p.selected_entry()) {
                    if entry.is_dir {
                        if let Some((task_id, title, progress)) =
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
                    }
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
            _ if key_matches_ctrl_shift_letter(&key, 'a') => {
                self.clear_pending_count();
                self.clear_marks_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_ctrl_letter(&key, 'a') => {
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
                self.copy_selected();
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
            self.current_pane_mut()?.set_preview_active(false);
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
            if let Some(count) = self.take_pending_count() {
                self.current_pane_mut()?
                    .scroll_preview_down(count.saturating_sub(1));
                self.status = format!("preview: moved {count}");
            } else {
                self.current_pane_mut()?.scroll_preview_bottom();
                self.status = String::from("preview: bottom");
            }
            self.pending_g = false;
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc => {
                if self.clear_preview_search_if_active() {
                    self.clear_pending_count();
                    self.pending_g = false;
                    return Ok(true);
                }
                self.current_pane_mut()?.set_preview_active(false);
                self.reset_pending_motion_state();
                self.status = String::from("normal mode");
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
                self.current_pane_mut()?.scroll_preview_down(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.scroll_preview_down(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            KeyCode::Up => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.scroll_preview_up(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.scroll_preview_up(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                let pending_line = self.pending_count;
                if self.pending_g {
                    if let Some(count) = self.take_pending_count() {
                        self.current_pane_mut()?
                            .scroll_preview_down(count.saturating_sub(1));
                        self.status = format!("preview: moved {count}");
                    } else {
                        self.current_pane_mut()?.scroll_preview_top();
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
                self.current_pane_mut()?.page_preview_down();
                self.pending_g = false;
                self.status = String::from("preview: half page down");
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
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
        }

        Ok(true)
    }

    /// 處理 visual selection 模式下的鍵盤輸入。
    pub(crate) fn handle_visual_selection_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V') {
            self.clear_pending_count();
            self.commit_visual_selection()?;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
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
                self.commit_visual_selection()?;
            }
            KeyCode::Down => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_down_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_down_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            KeyCode::Up => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_up_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
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
                self.pending_g = false;
                self.status = self.visual_status_label();
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

    /// 處理 global search 面板中的輸入、結果瀏覽與跳轉。
    pub(crate) fn handle_global_search_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.global_search.take() else {
            return Ok(true);
        };

        if search.editing {
            let edit_result = self.edit_text_buffer(&mut search.buffer, &key);
            if matches!(edit_result, TextEditResult::Changed) {
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
            }
            if matches!(edit_result, TextEditResult::Consumed) {
                self.global_search = Some(search);
                return Ok(true);
            }
            match key.code {
                KeyCode::Enter => {
                    if matches!(search.mode, SearchMode::Content) && search.buffer.trim().is_empty()
                    {
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
                    }
                    self.start_global_search(&mut search)?;
                    search.editing = false;
                }
                KeyCode::Esc => {
                    self.cancel_global_search();
                }
                _ => {}
            }

            if !matches!(key.code, KeyCode::Esc) {
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            return Ok(true);
        }

        if search.filter.editing {
            let edit_result = self.edit_text_buffer(&mut search.filter.buffer, &key);
            if matches!(edit_result, TextEditResult::Changed) {
                search.selected = 0;
                let visible =
                    filtered_global_search_entries(&search.results, &search.filter.buffer);
                search.selected = search.selected.min(visible.len().saturating_sub(1));
                self.status = global_search_filter_status(&search.filter, visible.len());
                self.global_search = Some(search);
                return Ok(true);
            }
            if matches!(edit_result, TextEditResult::Consumed) {
                self.global_search = Some(search);
                return Ok(true);
            }
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    search.filter.editing = false;
                }
                _ => {}
            }
            let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
            search.selected = search.selected.min(visible.len().saturating_sub(1));
            self.status = global_search_filter_status(&search.filter, visible.len());
            self.global_search = Some(search);
            return Ok(true);
        }

        if self
            .panes
            .get(&search.pane_id)
            .is_some_and(PaneState::is_preview_active)
            && matches!(search.mode, SearchMode::Content)
        {
            match key.code {
                KeyCode::Tab => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                        pane.set_preview_active(false);
                    }
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    self.global_search = Some(search);
                    return Ok(true);
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                        pane.set_preview_active(false);
                    }
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    search.preview_scroll = None;
                    search.preview_current_match = None;
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                        pane.set_preview_active(false);
                    }
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.move_search_preview_match(&mut search, true);
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'p')
                    || key_matches_shifted_letter(&key, 'N') =>
                {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.move_search_preview_match(&mut search, false);
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    search.preview_scroll =
                        Some(search.preview_scroll.unwrap_or(0).saturating_add(1));
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    search.preview_scroll =
                        Some(search.preview_scroll.unwrap_or(0).saturating_sub(1));
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ => {}
            }
        }

        if self.capture_pending_count_digit(&key) {
            self.global_search = Some(search);
            return Ok(true);
        }

        match key.code {
            KeyCode::Down => {
                let count = self.take_count_or_one();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + count).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + count).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                let step = self.take_large_move_step();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + step).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            KeyCode::Up => {
                let count = self.take_count_or_one();
                search.selected = search.selected.saturating_sub(count);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                let step = self.take_large_move_step();
                search.selected = search.selected.saturating_sub(step);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                let step = self.take_panel_page_step();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + step).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                let step = self.take_panel_page_step();
                search.selected = search.selected.saturating_sub(step);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                search.selected = search.selected.saturating_sub(count);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                let pending_line = self.pending_count;
                if self.pending_g {
                    if let Some(count) = self.take_pending_count() {
                        search.selected = count
                            .saturating_sub(1)
                            .min(global_search_visible_len(&search).saturating_sub(1));
                    } else {
                        search.selected = 0;
                    }
                    search.preview_scroll = None;
                    search.preview_current_match = None;
                    self.pending_g = false;
                } else {
                    self.pending_g = true;
                }
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                if self.pending_g {
                    self.status = if let Some(count) = pending_line {
                        format!("{} (normal): pending {count}g", search.mode.status_label())
                    } else {
                        format!("{} (normal): pending g", search.mode.status_label())
                    };
                }
                self.global_search = Some(search);
            }
            _ if key_matches_shifted_letter(&key, 'G') => {
                if let Some(count) = self.take_pending_count() {
                    if global_search_visible_len(&search) > 0 {
                        search.selected = count
                            .saturating_sub(1)
                            .min(global_search_visible_len(&search).saturating_sub(1));
                    }
                } else if global_search_visible_len(&search) > 0 {
                    search.selected = global_search_visible_len(&search) - 1;
                }
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.pending_g = false;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'i') || key_matches_plain_letter(&key, 's') => {
                self.clear_pending_count();
                search.editing = true;
                self.text_input_mode = RenameMode::Insert;
                self.text_input_cursor = search.buffer.chars().count();
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.pending_g = false;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    true,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'f') => {
                self.clear_pending_count();
                self.pending_g = false;
                if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                    pane.set_preview_active(false);
                }
                search.filter.editing = true;
                self.text_input_mode = RenameMode::Insert;
                self.text_input_cursor = search.filter.buffer.chars().count();
                search.selected = 0;
                self.status =
                    global_search_filter_status(&search.filter, global_search_visible_len(&search));
                self.global_search = Some(search);
            }
            KeyCode::Enter => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_global_search_result(search)?;
            }
            KeyCode::Right => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_global_search_result(search)?;
            }
            KeyCode::Tab if matches!(search.mode, SearchMode::Content) => {
                self.clear_pending_count();
                self.pending_g = false;
                if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                    pane.set_preview_active(true);
                }
                self.status = self.search_preview_status_for(&search);
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_global_search_result(search)?;
            }
            KeyCode::Esc => {
                self.clear_pending_count();
                self.pending_g = false;
                if search.filter.buffer.is_empty() {
                    self.cancel_global_search();
                } else {
                    search.filter = PanelSearchState::default();
                    search.selected = 0;
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    self.global_search = Some(search);
                }
            }
            _ if key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.cancel_global_search();
            }
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
        }

        Ok(true)
    }

    /// 處理列表內 find-next 輸入框的鍵盤輸入，並在每次輸入後立即更新高亮結果。
    pub(crate) fn handle_list_find_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.list_find.take() else {
            return Ok(true);
        };

        let edit_result = self.edit_text_buffer(&mut search.buffer, &key);
        if matches!(edit_result, TextEditResult::Changed) {
            self.apply_list_find_buffer(&search);
            self.status =
                list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
            self.list_find = Some(search);
            return Ok(true);
        }
        if matches!(edit_result, TextEditResult::Consumed) {
            self.list_find = Some(search);
            return Ok(true);
        }

        match key.code {
            KeyCode::Enter => {
                self.apply_list_find_buffer(&search);
                self.status = list_find_locked_status(
                    &search.buffer,
                    self.list_find_match_count(search.pane_id),
                );
            }
            KeyCode::Esc => {
                if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                    pane.clear_list_find();
                }
                self.status = String::from("normal mode");
            }
            _ => {
                self.list_find = Some(search);
            }
        }

        Ok(true)
    }

    /// 處理 filter 輸入框中的鍵盤輸入，並在每次輸入後立即更新列表。
    pub(crate) fn handle_filter_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut filter) = self.filter.take() else {
            return Ok(true);
        };

        if key.code == KeyCode::Tab
            || key_matches_ctrl_letter(&key, 'f')
            || key_matches_ctrl_letter(&key, 's')
        {
            filter.mode = match filter.mode {
                FilterMode::Normal => FilterMode::Fuzzy,
                FilterMode::Fuzzy => FilterMode::Normal,
            };
            self.apply_filter_buffer(&filter);
            self.status = format_filter_status(&filter);
            self.filter = Some(filter);
            return Ok(true);
        }

        let edit_result = self.edit_text_buffer(&mut filter.buffer, &key);
        if matches!(edit_result, TextEditResult::Changed) {
            self.apply_filter_buffer(&filter);
            self.status = format_filter_status(&filter);
            self.filter = Some(filter);
            return Ok(true);
        }
        if matches!(edit_result, TextEditResult::Consumed) {
            self.filter = Some(filter);
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc if filter.buffer.trim().is_empty() => {
                if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
                    pane.clear_filter();
                }
                self.status = String::from("normal mode");
            }
            KeyCode::Esc | KeyCode::Enter => {
                filter.editing = false;
                self.status = format_filter_status(&filter);
                self.filter = Some(filter);
            }
            _ => {
                self.filter = Some(filter);
            }
        }

        Ok(true)
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
            PendingAction::ToolPanel {
                pane_id,
                mut selected,
            } => {
                let tools = external_tool_statuses();
                let len = tools.len();
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => {
                        self.status = String::from("dependency panel closed");
                    }
                    _ if key_matches_plain_letter(&key, 'j') || key.code == KeyCode::Down => {
                        selected = if len == 0 {
                            0
                        } else {
                            (selected + 1).min(len - 1)
                        };
                        self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
                    }
                    _ if key_matches_plain_letter(&key, 'k') || key.code == KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
                    }
                }
            }
            PendingAction::ConfirmDelete {
                pane_id,
                target_name,
                permanent,
                ref warning_message,
            } => match key.code {
                _ if key_matches_shifted_letter(&key, 'D') => {
                    self.confirm_delete(pane_id, &target_name, true)?;
                }
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.status = if permanent {
                        format!("delete cancelled: {target_name}")
                    } else {
                        format!("trash cancelled: {target_name}")
                    };
                }
                _ if key_matches_letter_any_case(&key, 'y') => {
                    self.confirm_delete(pane_id, &target_name, permanent)?;
                }
                KeyCode::Esc => {
                    self.status = if permanent {
                        format!("delete cancelled: {target_name}")
                    } else {
                        format!("trash cancelled: {target_name}")
                    };
                }
                _ if key_matches_letter_any_case(&key, 'n') => {
                    self.status = if permanent {
                        format!("delete cancelled: {target_name}")
                    } else {
                        format!("trash cancelled: {target_name}")
                    };
                }
                _ => {
                    let warning_message = warning_message.clone();
                    self.pending_action = Some(PendingAction::ConfirmDelete {
                        pane_id,
                        target_name: target_name.clone(),
                        permanent,
                        warning_message,
                    });
                    self.status = if permanent {
                        format!("confirm delete {target_name}: y/n")
                    } else {
                        format!("confirm trash {target_name}: y/n")
                    };
                }
            },
            PendingAction::ConfirmPasteOverwrite {
                pane_id,
                target_name,
                entry_count,
                operation,
            } => match key.code {
                _ if key_matches_letter_any_case(&key, 'y') || key.code == KeyCode::Enter => {
                    self.confirm_paste_overwrite(pane_id, target_name, entry_count, operation)?;
                }
                KeyCode::Esc => {
                    self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
                }
                _ if key_matches_letter_any_case(&key, 'n') => {
                    self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                        pane_id,
                        target_name: target_name.clone(),
                        entry_count,
                        operation,
                    });
                    self.status = paste_overwrite_confirm_status(&target_name, entry_count);
                }
            },
            PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                marked_ids,
                visual_anchor,
            } => match key.code {
                _ if key_matches_plain_letter(&key, 'd')
                    && matches!(&action, TrashConfirmAction::DeleteFromPanel { .. }) =>
                {
                    self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                        &action,
                        marked_ids,
                        visual_anchor,
                    ));
                    self.status =
                        trash_confirm_cancelled_status(&action, &target_name, entry_count);
                }
                _ if key_matches_letter_any_case(&key, 'y') || key.code == KeyCode::Enter => {
                    self.confirm_trash_action(action, target_name, entry_count)?;
                }
                KeyCode::Esc => {
                    self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                        &action,
                        marked_ids,
                        visual_anchor,
                    ));
                    self.status =
                        trash_confirm_cancelled_status(&action, &target_name, entry_count);
                }
                _ if key_matches_letter_any_case(&key, 'n') => {
                    self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                        &action,
                        marked_ids,
                        visual_anchor,
                    ));
                    self.status =
                        trash_confirm_cancelled_status(&action, &target_name, entry_count);
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmTrashAction {
                        action: action.clone(),
                        target_name: target_name.clone(),
                        entry_count,
                        marked_ids,
                        visual_anchor,
                    });
                    self.status = trash_confirm_status(&action, &target_name, entry_count);
                }
            },
            PendingAction::GoPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'g') => {
                    if let Some(count) = self.take_pending_count() {
                        self.current_pane_mut()?
                            .move_to_visible_index(count.saturating_sub(1));
                        self.status = format!("jumped to item {count}");
                    } else {
                        self.current_pane_mut()?.move_top();
                        self.status = String::from("jumped to top");
                    }
                    self.pending_g = false;
                    self.pending_y = false;
                }
                _ if key_matches_plain_letter(&key, 't') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.open_prefilled_command("goto ");
                }
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.go_to_special_directory(GoSpecialDirectory::Documents)?;
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.go_to_special_directory(GoSpecialDirectory::Desktop)?;
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::GoPicker { pane_id });
                    self.status = String::from("go: choose g/t/d/k from the panel");
                }
            },
            PendingAction::ThemeCommandPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 't') => {
                    self.focused_pane = pane_id;
                    self.open_trash_panel()?;
                }
                _ if key_matches_plain_letter(&key, 'u') => {
                    self.focused_pane = pane_id;
                    self.restore_latest_from_trash()?;
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.open_theme_picker();
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.cycle_theme();
                }
                KeyCode::Esc => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ThemeCommandPicker { pane_id });
                    self.status = String::from("theme/trash: choose l/n/t/u from the panel");
                }
            },
            PendingAction::SortPicker { pane_id } => match key.code {
                KeyCode::Char(',') => {
                    self.status = String::from("sort cancelled");
                }
                _ if key_matches_shifted_letter(&key, 'M') => {
                    self.apply_sort_mode(pane_id, SortMode::Modified { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'm') => {
                    self.apply_sort_mode(pane_id, SortMode::Modified { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'B') => {
                    self.apply_sort_mode(pane_id, SortMode::Created { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'b') => {
                    self.apply_sort_mode(pane_id, SortMode::Created { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'A') => {
                    self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'a') => {
                    self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'N') => {
                    self.apply_sort_mode(pane_id, SortMode::Natural { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.apply_sort_mode(pane_id, SortMode::Natural { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'E') => {
                    self.apply_sort_mode(pane_id, SortMode::Extension { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'e') => {
                    self.apply_sort_mode(pane_id, SortMode::Extension { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'S') => {
                    self.apply_sort_mode(pane_id, SortMode::Size { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 's') => {
                    self.apply_sort_mode(pane_id, SortMode::Size { reverse: false })?
                }
                _ if key_matches_plain_letter(&key, 'r') => {
                    self.apply_sort_mode(pane_id, SortMode::Random)?
                }
                KeyCode::Esc => {
                    self.status = String::from("sort cancelled");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.status = String::from("sort cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::SortPicker { pane_id });
                    self.status = String::from("sort: choose a key from the panel");
                }
            },
            PendingAction::WindowPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'w') => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Vertical, SplitPlacement::Before)?;
                }
                _ if key_matches_plain_letter(&key, 'j') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Horizontal, SplitPlacement::After)?;
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Horizontal, SplitPlacement::Before)?;
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Vertical, SplitPlacement::After)?;
                }
                _ if key_matches_plain_letter(&key, 'c') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    if self.focused_pane == pane_id {
                        self.close_current_pane();
                    } else {
                        self.status = String::from("panel focus changed");
                    }
                }
                _ if key_matches_plain_letter(&key, 'o') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    if self.focused_pane == pane_id {
                        self.only_current_pane();
                    } else {
                        self.status = String::from("panel focus changed");
                    }
                }
                _ if key_matches_plain_letter(&key, 't') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    if self.focused_pane == pane_id {
                        self.open_terminal_in_active_panel()?;
                    } else {
                        self.status = String::from("panel focus changed");
                    }
                }
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.open_diff_matrix(None)?;
                }
                _ if key_matches_shifted_letter(&key, 'D') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.open_prefilled_command("diff ");
                }
                KeyCode::Esc => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') => {
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::WindowPicker { pane_id });
                    self.status = String::from("panel: choose h/j/k/l/c/o/t/d from the panel");
                }
            },
            PendingAction::LineModePicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'm') => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 's') => {
                    self.apply_line_mode(pane_id, LineMode::Size)?;
                }
                _ if key_matches_plain_letter(&key, 'p') => {
                    self.apply_line_mode(pane_id, LineMode::Permissions)?;
                }
                _ if key_matches_plain_letter(&key, 'b') => {
                    self.apply_line_mode(pane_id, LineMode::Btime)?;
                }
                _ if key_matches_plain_letter(&key, 't') => {
                    self.apply_line_mode(pane_id, LineMode::Mtime)?;
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.apply_line_mode(pane_id, LineMode::None)?;
                }
                KeyCode::Esc => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::LineModePicker { pane_id });
                    self.status = String::from("linemode: choose a key from the panel");
                }
            },
            PendingAction::ThemePicker {
                mut selected,
                original,
            } => match key.code {
                KeyCode::Down => {
                    selected = (selected + 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_plain_letter(&key, 'j') => {
                    selected = (selected + 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                KeyCode::Up => {
                    selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_shifted_letter(&key, 'J') => {
                    selected = (selected + self.take_large_move_step())
                        .min(ThemePreset::ALL.len().saturating_sub(1));
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_shifted_letter(&key, 'K') => {
                    selected = selected.saturating_sub(self.take_large_move_step());
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_ctrl_letter(&key, 'd') => {
                    selected = (selected + self.take_panel_page_step())
                        .min(ThemePreset::ALL.len().saturating_sub(1));
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_ctrl_letter(&key, 'u') => {
                    selected = selected.saturating_sub(self.take_panel_page_step());
                    self.preview_theme_picker_selection(selected, original);
                }
                KeyCode::Enter => self.apply_theme(ThemePreset::ALL[selected]),
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.apply_theme(ThemePreset::ALL[selected])
                }
                KeyCode::Esc => {
                    self.theme = original.into();
                    self.status = String::from("theme picker cancelled");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.theme = original.into();
                    self.status = String::from("theme picker cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ThemePicker { selected, original });
                    self.status = String::from("theme picker: use j/k preview, l apply, h cancel");
                }
            },
            PendingAction::TrashPanel {
                pane_id,
                mut selected,
                mut search,
                mut marked_ids,
                mut visual_anchor,
            } => {
                let entries = trash_panel_entries(&self.trash_store, &search.buffer)?;
                let len = entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
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
                        self.status = trash_panel_status(
                            &search_buffer,
                            len,
                            selected,
                            search_editing,
                            marked_count,
                        );
                        return Ok(true);
                    }
                    if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V')
                    {
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
                            self.status = self.trash_visual_status_label(
                                selected,
                                selected,
                                marked_ids.len(),
                            );
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
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
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
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
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
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
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
            }
            PendingAction::HelpPanel {
                pane_id,
                mut selected,
                mut search,
                custom_title,
                custom_entries,
            } => {
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
                                selected =
                                    count.saturating_sub(1).min(filtered_len.saturating_sub(1));
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
                                    selected =
                                        count.saturating_sub(1).min(filtered_len.saturating_sub(1));
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
                        _ if key_matches_tilde(&key) || key_matches_plain_letter(&key, '?') => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.restore_help_return_state(false)?;
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
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
            }
            PendingAction::TaskPanel {
                pane_id,
                mut selected,
                mut search,
                mut marked_ids,
                mut visual_anchor,
            } => {
                let tasks = self.tasks_for_pane(pane_id);
                let filtered_tasks = filtered_task_entries(&tasks, &search.buffer);
                let len = filtered_tasks.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = filtered_task_entries(&tasks, &search.buffer).len();
                    let status = task_panel_status(
                        &search.buffer,
                        next_len,
                        selected,
                        search.editing,
                        marked_ids.len(),
                    );
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    self.status = status;
                } else {
                    if self.capture_pending_count_digit(&key) {
                        self.pending_action = Some(PendingAction::TaskPanel {
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
                        let status = if let Some(anchor) = visual_anchor {
                            self.task_visual_status_label(anchor, selected, marked_ids.len())
                        } else {
                            task_panel_status(
                                &search.buffer,
                                len,
                                selected,
                                false,
                                marked_ids.len(),
                            )
                        };
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        self.status = status;
                        return Ok(true);
                    }
                    if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V')
                    {
                        self.pending_g = false;
                        if let Some(anchor) = visual_anchor.take() {
                            let added = self.commit_task_visual_selection(
                                &filtered_tasks,
                                &mut marked_ids,
                                anchor,
                                selected,
                            );
                            self.status = if added == 0 {
                                format!("tasks: kept {} marked items", marked_ids.len())
                            } else {
                                format!("tasks: marked {} items", marked_ids.len())
                            };
                        } else if len > 0 {
                            visual_anchor = Some(selected);
                            self.status =
                                self.task_visual_status_label(selected, selected, marked_ids.len());
                        }
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        return Ok(true);
                    }
                    if key.code == KeyCode::Char(' ') && key.modifiers.is_empty() {
                        self.pending_g = false;
                        if let Some(task) = filtered_tasks.get(selected) {
                            if let Some(pos) = marked_ids.iter().position(|id| *id == task.id) {
                                marked_ids.remove(pos);
                                self.status = format!(
                                    "tasks: unmarked task #{}, {} marked",
                                    task.id,
                                    marked_ids.len()
                                );
                            } else {
                                marked_ids.push(task.id);
                                self.status = format!(
                                    "tasks: marked task #{}, {} marked",
                                    task.id,
                                    marked_ids.len()
                                );
                            }
                        }
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        return Ok(true);
                    }
                    if key_matches_plain_letter(&key, 'a') {
                        self.pending_g = false;
                        if marked_ids.len() == len && len > 0 {
                            marked_ids.clear();
                            self.status = String::from("tasks: cleared all marks");
                        } else {
                            marked_ids = filtered_tasks.iter().map(|t| t.id).collect();
                            self.status = format!("tasks: marked all {} tasks", marked_ids.len());
                        }
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        return Ok(true);
                    }
                    if key_matches_plain_letter(&key, 'd') {
                        self.clear_pending_count();
                        self.pending_g = false;
                        if !marked_ids.is_empty() {
                            let to_delete = std::mem::take(&mut marked_ids);
                            visual_anchor = None;
                            let deleted = self.delete_tasks_by_ids(&to_delete);
                            let next_tasks = self.tasks_for_pane(pane_id);
                            let next_len = filtered_task_entries(&next_tasks, &search.buffer).len();
                            selected = selected.min(next_len.saturating_sub(1));
                            self.status = format!("tasks: deleted {deleted} tasks");
                            self.pending_action = Some(PendingAction::TaskPanel {
                                pane_id,
                                selected,
                                search,
                                marked_ids,
                                visual_anchor,
                            });
                            return Ok(true);
                        } else if let Some(task) = filtered_tasks.get(selected) {
                            let target_id = task.id;
                            let target_kind = task.kind.clone();
                            let _deleted = self.delete_tasks_by_ids(&[target_id]);
                            let next_tasks = self.tasks_for_pane(pane_id);
                            let next_len = filtered_task_entries(&next_tasks, &search.buffer).len();
                            selected = selected.min(next_len.saturating_sub(1));
                            self.status =
                                format!("tasks: deleted task #{target_id} [{target_kind}]");
                            self.pending_action = Some(PendingAction::TaskPanel {
                                pane_id,
                                selected,
                                search,
                                marked_ids,
                                visual_anchor,
                            });
                            return Ok(true);
                        } else {
                            self.status = String::from("tasks: empty");
                            self.pending_action = Some(PendingAction::TaskPanel {
                                pane_id,
                                selected: 0,
                                search,
                                marked_ids,
                                visual_anchor: None,
                            });
                            return Ok(true);
                        }
                    }
                    if key_matches_shifted_letter(&key, 'D') {
                        self.clear_pending_count();
                        self.pending_g = false;
                        marked_ids.clear();
                        visual_anchor = None;
                        let deleted = self.delete_all_tasks_for_pane(pane_id);
                        selected = 0;
                        self.status = if deleted == 0 {
                            String::from("tasks: empty")
                        } else {
                            format!("tasks: cleared {deleted} tasks")
                        };
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Down => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
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
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
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
                        _ if key_matches_plain_letter(&key, 'x')
                            || key_matches_plain_letter(&key, 'c') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            if let Some(task) = filtered_tasks.get(selected) {
                                self.cancel_task_by_id(task.id);
                                let next_tasks = self.tasks_for_pane(pane_id);
                                let next_len =
                                    filtered_task_entries(&next_tasks, &search.buffer).len();
                                selected = selected.min(next_len.saturating_sub(1));
                                let status = task_panel_status(
                                    &search.buffer,
                                    next_len,
                                    selected,
                                    false,
                                    marked_ids.len(),
                                );
                                self.pending_action = Some(PendingAction::TaskPanel {
                                    pane_id,
                                    selected,
                                    search,
                                    marked_ids,
                                    visual_anchor,
                                });
                                if self.status.is_empty() {
                                    self.status = status;
                                }
                                return Ok(true);
                            }
                        }
                        _ if key_matches_shifted_letter(&key, 'X')
                            || key_matches_shifted_letter(&key, 'C') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            let cancelled = self.cancel_running_tasks_for_pane(pane_id);
                            self.status = if cancelled == 0 {
                                String::from("no cancellable running tasks")
                            } else {
                                format!("cancelled {cancelled} tasks")
                            };
                            self.pending_action = Some(PendingAction::TaskPanel {
                                pane_id,
                                selected,
                                search,
                                marked_ids,
                                visual_anchor,
                            });
                            return Ok(true);
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
                        KeyCode::Enter | KeyCode::Right => {
                            if let Some(task) = filtered_tasks.get(selected) {
                                self.status =
                                    format!("task {} [{}] {}", task.id, task.kind, task.detail);
                            } else {
                                self.status = String::from("tasks: empty");
                            }
                        }
                        _ if key_matches_plain_letter(&key, 'l') => {
                            if let Some(task) = filtered_tasks.get(selected) {
                                self.status =
                                    format!("task {} [{}] {}", task.id, task.kind, task.detail);
                            } else {
                                self.status = String::from("tasks: empty");
                            }
                        }
                        KeyCode::Esc => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            if let Some(_) = visual_anchor.take() {
                                self.status = String::from("task visual: cancelled");
                            } else if !marked_ids.is_empty() {
                                let cleared = marked_ids.len();
                                marked_ids.clear();
                                self.status = format!("tasks: cleared {cleared} marks");
                            } else {
                                self.status = String::from("normal mode");
                                return Ok(true);
                            }
                        }
                        _ if key_matches_plain_letter(&key, 't')
                            || key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
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
                        self.task_visual_status_label(anchor, selected, marked_ids.len())
                    } else {
                        task_panel_status(&search.buffer, len, selected, false, marked_ids.len())
                    };
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    if !matches!(key.code, KeyCode::Enter | KeyCode::Right)
                        && !key_matches_plain_letter(&key, 'l')
                    {
                        self.status = status;
                    }
                }
            }
            PendingAction::BookmarkPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'b') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'a') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.add_bookmark_with_auto_key(pane_id)?;
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'g') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.open_bookmark_list_with_mode(pane_id, BookmarkListMode::Jump);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.open_bookmark_list_with_mode(pane_id, BookmarkListMode::Delete);
                    return Ok(true);
                }
                _ if key_matches_shifted_letter(&key, 'D') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.delete_all_bookmarks()?;
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
                    self.pending_action = Some(PendingAction::BookmarkPicker { pane_id });
                    self.status = String::from("bookmark: choose a/g/d/D from the panel");
                    return Ok(true);
                }
            },
            PendingAction::BookmarkList {
                pane_id,
                mut selected,
                mode,
                mut search,
            } => {
                let entries = self.bookmark_store.list();
                let filtered_entries = filtered_bookmark_entries(entries.clone(), &search.buffer);
                let len = filtered_entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len =
                        filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer).len();
                    let status = bookmark_list_status(
                        &search.buffer,
                        next_len,
                        selected,
                        mode,
                        search.editing,
                    );
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
                        let status =
                            bookmark_list_status(&search.buffer, len, selected, mode, false);
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
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
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
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
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
                                    self.open_bookmark_from_list(
                                        pane_id,
                                        &filtered_entries,
                                        selected,
                                    )?;
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
                                let status = bookmark_list_status(
                                    &search.buffer,
                                    len,
                                    selected,
                                    mode,
                                    false,
                                );
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
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
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
            }
            PendingAction::ZoxideList {
                pane_id,
                mut selected,
                entries,
                mut search,
            } => {
                let filtered_entries = filtered_zoxide_entries(&entries, &search.buffer);
                let len = filtered_entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = filtered_zoxide_entries(&entries, &search.buffer).len();
                    let status =
                        zoxide_list_status(&search.buffer, next_len, selected, search.editing);
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
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
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
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
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
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
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
            }
            PendingAction::CopyPicker {
                pane_id,
                target,
                mut selected,
            } => {
                let options = copy_picker_options();
                if self.capture_pending_count_digit(&key) {
                    self.pending_action = Some(PendingAction::CopyPicker {
                        pane_id,
                        target: target.clone(),
                        selected,
                    });
                    self.status = format!("copy to clipboard: {}", target.display_name);
                    return Ok(true);
                }
                match key.code {
                    _ if key_matches_plain_letter(&key, 'c') => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_plain_letter(&key, 'u') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(target.clone(), CopyAction::FileUrl)?;
                    }
                    _ if key_matches_plain_letter(&key, 'd') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(
                            target.clone(),
                            CopyAction::DirectoryUrl,
                        )?;
                    }
                    _ if key_matches_plain_letter(&key, 'f') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(target.clone(), CopyAction::Filename)?;
                    }
                    _ if key_matches_plain_letter(&key, 'n') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(
                            target.clone(),
                            CopyAction::FilenameWithoutExtension,
                        )?;
                    }
                    KeyCode::Down => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'j') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'k') => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'd') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_panel_page_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'u') => {
                        selected = selected.saturating_sub(self.take_panel_page_step());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'J') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_large_move_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'K') => {
                        selected = selected.saturating_sub(self.take_large_move_step());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    KeyCode::Enter => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.copy_target_to_system_clipboard(target.clone(), option.action)?;
                        } else {
                            self.status = String::from("copy picker: no option selected");
                        }
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.copy_target_to_system_clipboard(target.clone(), option.action)?;
                        } else {
                            self.status = String::from("copy picker: no option selected");
                        }
                    }
                    KeyCode::Esc => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_plain_letter(&key, 'q')
                        || key_matches_plain_letter(&key, 'h') =>
                    {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ => {
                        self.clear_pending_count();
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                }
            }
            PendingAction::OpenPicker {
                pane_id,
                target,
                mut selected,
                options,
            } => {
                if self.capture_pending_count_digit(&key) {
                    self.pending_action = Some(PendingAction::OpenPicker {
                        pane_id,
                        target: target.clone(),
                        selected,
                        options: options.clone(),
                    });
                    self.status = format!("open with: {}", target.display_name);
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_shifted_letter(&key, 'O') => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    KeyCode::Down => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'j') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'k') => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'd') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_panel_page_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'u') => {
                        selected = selected.saturating_sub(self.take_panel_page_step());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'J') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_large_move_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'K') => {
                        selected = selected.saturating_sub(self.take_large_move_step());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    KeyCode::Enter => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.queue_open_picker_action(target.clone(), option.action.clone())?;
                        } else {
                            self.status = String::from("open with: no option selected");
                        }
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.queue_open_picker_action(target.clone(), option.action.clone())?;
                        } else {
                            self.status = String::from("open with: no option selected");
                        }
                    }
                    KeyCode::Esc => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_plain_letter(&key, 'q')
                        || key_matches_plain_letter(&key, 'h') =>
                    {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ => {
                        self.clear_pending_count();
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                }
            }
            PendingAction::Rename {
                pane_id,
                original_name,
                mut buffer,
                mut cursor,
                mut mode,
            } => match mode {
                RenameMode::Insert => match key.code {
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
                },
                RenameMode::Normal if key_matches_shifted_letter(&key, 'A') => {
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
                }
                RenameMode::Normal => match key.code {
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
                    KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
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
                    _ => {
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
            },
            PendingAction::CreateEntry {
                pane_id,
                mut buffer,
                mut cursor,
                mut mode,
            } => match mode {
                RenameMode::Insert => match key.code {
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
                },
                RenameMode::Normal if key_matches_shifted_letter(&key, 'A') => {
                    cursor = buffer.chars().count();
                    mode = RenameMode::Insert;
                    self.pending_action = Some(PendingAction::CreateEntry {
                        pane_id,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = create_status_label("insert");
                }
                RenameMode::Normal => match key.code {
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
                    KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
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
                    _ => {
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
            },
            PendingAction::RegexRename {
                pane_id,
                pattern,
                replacement,
                mut selected,
                previews,
            } => {
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
                            selected =
                                (selected + self.take_count_or_one()).min(len.saturating_sub(1));
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
                            selected =
                                (selected + self.take_count_or_one()).min(len.saturating_sub(1));
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
                            selected =
                                (selected + self.take_panel_page_step()).min(len.saturating_sub(1));
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
                            selected =
                                (selected + self.take_large_move_step()).min(len.saturating_sub(1));
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
                    _ if key_matches_plain_letter(&key, 'q')
                        || key_matches_plain_letter(&key, 'h') =>
                    {
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
            }
            PendingAction::DiffMatrix(mut state) => {
                if state.search_active {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            state.search_active = false;
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        KeyCode::Backspace => {
                            state.search_query.pop();
                            state.refresh_filtered_indices();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        KeyCode::Char(c) => {
                            state.search_query.push(c);
                            state.refresh_filtered_indices();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ => {
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                    }
                } else if state.loading {
                    match key.code {
                        KeyCode::Esc => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ if key_matches_plain_letter(&key, 'q') => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ => {
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ if key_matches_plain_letter(&key, 'q') => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ if key_matches_plain_letter(&key, 'j') || key.code == KeyCode::Down => {
                            state.move_down();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'k') || key.code == KeyCode::Up => {
                            state.move_up();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'g') || key.code == KeyCode::Home => {
                            state.move_to_top();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_letter_any_case(&key, 'G') || key.code == KeyCode::End => {
                            state.move_to_bottom();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'f') => {
                            state.cycle_filter_mode();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key.code == KeyCode::Char('/') => {
                            state.search_active = true;
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'i') => {
                            state.git_ignore = !state.git_ignore;
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            let cancelled = Arc::new(AtomicBool::new(false));
                            self.diff_job_cancelled = Some(cancelled.clone());
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.diff_job_rx = Some(rx);
                            spawn_background_diff(
                                state.panel_roots.clone(),
                                state.git_ignore,
                                state.include_hidden,
                                cancelled,
                                tx,
                            );
                            state.loading = true;
                            state.rows.clear();
                            state.filtered_indices.clear();
                            self.status = format!(
                                "diff matrix: .gitignore rules {}",
                                if state.git_ignore {
                                    "enabled"
                                } else {
                                    "disabled (scanning target/build dirs)"
                                }
                            );
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key.code == KeyCode::Char('.') => {
                            state.include_hidden = !state.include_hidden;
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            let cancelled = Arc::new(AtomicBool::new(false));
                            self.diff_job_cancelled = Some(cancelled.clone());
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.diff_job_rx = Some(rx);
                            spawn_background_diff(
                                state.panel_roots.clone(),
                                state.git_ignore,
                                state.include_hidden,
                                cancelled,
                                tx,
                            );
                            state.loading = true;
                            state.rows.clear();
                            state.filtered_indices.clear();
                            self.status = format!(
                                "diff matrix: hidden files {}",
                                if state.include_hidden {
                                    "included"
                                } else {
                                    "excluded"
                                }
                            );
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'r') => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            let cancelled = Arc::new(AtomicBool::new(false));
                            self.diff_job_cancelled = Some(cancelled.clone());
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.diff_job_rx = Some(rx);
                            spawn_background_diff(
                                state.panel_roots.clone(),
                                state.git_ignore,
                                state.include_hidden,
                                cancelled,
                                tx,
                            );
                            state.loading = true;
                            state.rows.clear();
                            state.filtered_indices.clear();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        KeyCode::Enter => {
                            if let Some(row) = state.selected_row() {
                                if let Some(launch) =
                                    launch_content_diff_spec(&state.panel_roots, row)
                                {
                                    let detail =
                                        format!("{} {}", launch.program, launch.args.join(" "));
                                    let sources = state
                                        .panel_roots
                                        .iter()
                                        .map(|p| p.join(&row.relative_path).display().to_string())
                                        .collect();
                                    let task_id = self.push_task(
                                        self.focused_pane,
                                        "diff",
                                        format!("diff: {}", row.relative_path.display()),
                                        detail,
                                        sources,
                                        None,
                                    );
                                    self.pending_launch = Some(QueuedLaunch { task_id, launch });
                                    self.status =
                                        format!("opening diff for {}", row.relative_path.display());
                                } else if row.is_dir {
                                    self.status =
                                        format!("{} is a directory", row.relative_path.display());
                                }
                            }
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ => {
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                    }
                }
                if let Some(action) = self.pending_action.as_ref() {
                    self.status = self.status_for_pending_action(action)?;
                }
            }
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

    /// 在一般模式按下 `Esc` 時，優先處理 filter 的兩段式離開流程。
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
