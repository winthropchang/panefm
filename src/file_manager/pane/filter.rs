use super::{
    PaneState,
    sort::{is_hidden_name, sort_file_entries},
    types::{FilterCache, FilterMode},
};
use crate::file_manager::fuzzy::fuzzy_matched_indices;

impl PaneState {
    pub(crate) fn set_filter_query(&mut self, query: &str, mode: FilterMode) {
        let trimmed = query.trim();
        self.filter_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        self.filter_mode = mode;
        self.refresh_visible_entries();
    }

    /// 清除目前的 filter，恢復成顯示全部項目。
    pub(crate) fn clear_filter(&mut self) {
        self.filter_query = None;
        self.refresh_visible_entries();
    }

    /// 將 pane 切換到指定路徑所在的目錄，並把游標聚焦到該項目。
    ///
    /// 參數：
    /// - `path: &Path`，要在列表中顯示並選中的目標路徑。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表 pane 已切到正確目錄並聚焦項目。
    /// - 失敗時代表重新載入目錄內容時發生 I/O 錯誤。
    pub(crate) fn has_active_filter(&self) -> bool {
        self.filter_query.is_some()
    }

    /// 切換目前 pane 是否顯示隱藏檔。
    pub(crate) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh_visible_entries();
    }

    /// 直接設定目前 pane 是否顯示隱藏檔，而不是做切換。
    ///
    /// 參數：
    /// - `show_hidden: bool`，`true` 表示顯示隱藏檔，`false` 表示隱藏。
    ///
    /// 回傳：`()`
    pub(crate) fn set_show_hidden(&mut self, show_hidden: bool) {
        self.show_hidden = show_hidden;
        self.refresh_visible_entries();
    }

    /// 切換到下一個排序模式，並立即重排目前列表。
    pub(crate) fn refresh_visible_entries(&mut self) {
        let previous_selected_path = self.selected_entry().map(|entry| entry.path.clone());
        self.visible_indices = match &self.filter_query {
            Some(query) => {
                let is_fuzzy = matches!(self.filter_mode, FilterMode::Fuzzy);
                let candidates = self.filter_candidates(query, is_fuzzy);
                if is_fuzzy {
                    fuzzy_matched_indices(&candidates, query, |index| {
                        self.entries[*index].name.clone().into()
                    })
                    .into_iter()
                    .map(|matched_index| candidates[matched_index])
                    .collect()
                } else {
                    candidates
                        .into_iter()
                        .filter(|index| normal_filter_matches(&self.entries[*index].name, query))
                        .collect()
                }
            }
            None => {
                self.filter_cache = None;
                self.base_visible_candidates()
            }
        };

        if let Some(query) = &self.filter_query {
            let is_fuzzy = matches!(self.filter_mode, FilterMode::Fuzzy);
            self.filter_cache = Some(FilterCache {
                query: query.clone(),
                is_fuzzy,
                show_hidden: self.show_hidden,
                entry_revision: self.entry_revision,
                matched_indices: self.visible_indices.clone(),
            });
        }

        if self.visible_indices.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
        } else {
            self.selected = self
                .selected
                .min(self.visible_indices.len().saturating_sub(1));
            self.list_state.select(Some(self.selected));
        }

        let current_selected_path = self.selected_entry().map(|entry| entry.path.clone());
        if previous_selected_path != current_selected_path {
            self.preview_scroll = 0;
            self.preview_search_query = None;
            self.preview_current_match = None;
        } else {
            self.clamp_preview_scroll();
        }
    }

    /// 依照目前排序模式重排完整項目列表。
    pub(crate) fn sort_entries(&mut self) {
        sort_file_entries(&mut self.entries, self.sort_mode, self.random_seed);
        self.bump_entry_revision();
    }

    fn base_visible_candidates(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.show_hidden || !is_hidden_name(&entry.name))
            .map(|(index, _)| index)
            .collect()
    }

    fn filter_candidates(&self, query: &str, is_fuzzy: bool) -> Vec<usize> {
        if let Some(cache) = &self.filter_cache {
            let is_incremental_narrowing =
                query.starts_with(&cache.query) && query.len() > cache.query.len();
            if is_incremental_narrowing
                && cache.is_fuzzy == is_fuzzy
                && cache.show_hidden == self.show_hidden
                && cache.entry_revision == self.entry_revision
            {
                return cache.matched_indices.clone();
            }
        }
        self.base_visible_candidates()
    }

    pub(crate) fn bump_entry_revision(&mut self) {
        self.entry_revision = self.entry_revision.wrapping_add(1);
        self.filter_cache = None;
    }
}

/// 一般 filter 以空白拆成多個詞，每個詞都必須是檔名的一段連續文字。
/// ASCII 使用零配置的大小寫不敏感比較；非 ASCII 則保留 Unicode 大小寫語意。
fn normal_filter_matches(name: &str, query: &str) -> bool {
    query
        .split_whitespace()
        .all(|term| contains_case_insensitive(name, term))
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_ascii() {
        let needle = needle.as_bytes();
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|candidate| candidate.eq_ignore_ascii_case(needle));
    }

    haystack.to_lowercase().contains(&needle.to_lowercase())
}
