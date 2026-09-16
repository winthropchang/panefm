use super::*;

impl App {
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
                _ if key_matches_plain_letter(&key, 'q') => {
                    self.cancel_global_search();
                }
                _ => {}
            }

            if !matches!(key.code, KeyCode::Esc) && !key_matches_plain_letter(&key, 'q') {
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
                _ if key_matches_plain_letter(&key, 'q') => {
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
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
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
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
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
            _ if key_matches_plain_letter(&key, 'q') => {
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
            _ if key_matches_plain_letter(&key, 'q') => {
                if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
                    pane.clear_filter();
                }
                self.status = String::from("normal mode");
            }
            _ => {
                self.filter = Some(filter);
            }
        }

        Ok(true)
    }
}
