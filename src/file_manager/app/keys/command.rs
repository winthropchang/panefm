use super::*;

impl App {
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
            RenameMode::Insert => {
                if key_matches_ctrl_letter(key, 'v') {
                    if let Some(text) = read_text_from_system_clipboard() {
                        let text = sanitize_pasted_text(&text);
                        if !text.is_empty() {
                            insert_str(buffer, &mut self.text_input_cursor, &text);
                            return TextEditResult::Changed;
                        }
                    }
                    return TextEditResult::Consumed;
                }
                match key.code {
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
                }
            }
            RenameMode::Normal => {
                if key_matches_shifted_letter(key, 'A') {
                    self.text_input_cursor = buffer.chars().count();
                    self.text_input_mode = RenameMode::Insert;
                    return TextEditResult::Consumed;
                }
                if key_matches_plain_letter(key, 'p') {
                    if let Some(text) = read_text_from_system_clipboard() {
                        let text = sanitize_pasted_text(&text);
                        if !text.is_empty() {
                            self.text_input_cursor =
                                move_cursor_right(buffer, self.text_input_cursor);
                            insert_str(buffer, &mut self.text_input_cursor, &text);
                            self.text_input_cursor =
                                normal_cursor(buffer, self.text_input_cursor.saturating_sub(1));
                            return TextEditResult::Changed;
                        }
                    }
                    return TextEditResult::Consumed;
                }
                if key_matches_shifted_letter(key, 'P') {
                    if let Some(text) = read_text_from_system_clipboard() {
                        let text = sanitize_pasted_text(&text);
                        if !text.is_empty() {
                            insert_str(buffer, &mut self.text_input_cursor, &text);
                            self.text_input_cursor =
                                normal_cursor(buffer, self.text_input_cursor.saturating_sub(1));
                            return TextEditResult::Changed;
                        }
                    }
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
                    _ if key_matches_plain_letter(key, 'q') => {
                        return TextEditResult::PassThrough;
                    }
                    _ => {}
                }
                TextEditResult::Consumed
            }
        }
    }
}
