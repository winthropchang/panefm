//! 終端 Bracketed Paste 事件（滑鼠右鍵或終端貼上字串）的分派處理。

use anyhow::Result;

use super::super::*;

impl App {
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
