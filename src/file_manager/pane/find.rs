use super::PaneState;

impl PaneState {
    pub(crate) fn has_list_find(&self) -> bool {
        self.list_find_query.is_some()
    }

    /// 取得目前列表內 find-next 的查詢文字。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Option<&str>`。
    /// - `Some(query)` 代表目前有啟用中的查詢。
    /// - `None` 代表目前沒有查詢。
    pub(crate) fn list_find_query(&self) -> Option<&str> {
        self.list_find_query.as_deref()
    }

    /// 設定列表內 find-next 的查詢文字。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要更新搜尋條件的 pane。
    /// - `query: &str`，新的查詢字串；空白字串會視為清除。
    ///
    /// 回傳：`()`
    pub(crate) fn set_list_find_query(&mut self, query: &str) {
        let trimmed = query.trim();
        self.list_find_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_lowercase())
        };
    }

    /// 清除目前列表中的 find-next 結果。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被清除搜尋狀態的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn clear_list_find(&mut self) {
        self.list_find_query = None;
    }

    /// 回傳目前可見列表中所有符合 find-next 的索引位置。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Vec<usize>`。
    /// - 每個值都是目前可見列表中的索引位置，而不是 `entries` 的原始索引。
    pub(crate) fn list_find_match_indices(&self) -> Vec<usize> {
        let Some(query) = self.list_find_query.as_deref() else {
            return Vec::new();
        };

        self.visible_entries()
            .into_iter()
            .enumerate()
            .filter_map(|(visible_index, entry)| {
                entry
                    .name
                    .to_lowercase()
                    .contains(query)
                    .then_some(visible_index)
            })
            .collect()
    }

    /// 回傳目前選取項目在 find-next 命中結果中的順位與總數。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Option<(usize, usize)>`。
    /// - `Some((current, total))` 代表目前選取列就是命中結果之一，且 `current` 為 1-based。
    /// - `None` 代表目前沒有命中結果，或目前游標不在命中項目上。
    pub(crate) fn list_find_match_position(&self) -> Option<(usize, usize)> {
        let matches = self.list_find_match_indices();
        let total = matches.len();
        if total == 0 {
            return None;
        }

        matches
            .iter()
            .position(|index| *index == self.selected)
            .map(|position| (position + 1, total))
    }

    /// 把列表游標移到下一個 find-next 命中項目，找不到時會循環回第一個。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要移動游標的 pane。
    ///
    /// 回傳：`bool`。
    /// - `true` 代表成功跳到某個命中項目。
    /// - `false` 代表目前沒有任何命中。
    pub(crate) fn jump_to_next_list_find_match(&mut self) -> bool {
        let matches = self.list_find_match_indices();
        let Some(target) = matches
            .iter()
            .copied()
            .find(|index| *index > self.selected)
            .or_else(|| matches.first().copied())
        else {
            return false;
        };

        self.selected = target;
        self.list_state.select(Some(target));
        self.preview_scroll = 0;
        true
    }

    /// 把列表游標移到上一個 find-next 命中項目，找不到時會循環回最後一個。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要移動游標的 pane。
    ///
    /// 回傳：`bool`。
    /// - `true` 代表成功跳到某個命中項目。
    /// - `false` 代表目前沒有任何命中。
    pub(crate) fn jump_to_previous_list_find_match(&mut self) -> bool {
        let matches = self.list_find_match_indices();
        let Some(target) = matches
            .iter()
            .rev()
            .copied()
            .find(|index| *index < self.selected)
            .or_else(|| matches.last().copied())
        else {
            return false;
        };

        self.selected = target;
        self.list_state.select(Some(target));
        self.preview_scroll = 0;
        true
    }
}
