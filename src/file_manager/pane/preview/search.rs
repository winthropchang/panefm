use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use super::super::{
    PaneState,
    preview_render::{build_search_preview_lines, preview_match_positions},
    types::SearchPreviewData,
};
use crate::file_manager::search::GlobalSearchEntry;
use crate::theme::Theme;

impl PaneState {
    /// 判斷目前 preview 是否正在套用搜尋條件。
    pub(crate) fn has_preview_search(&self) -> bool {
        self.preview_search_query.is_some()
    }

    /// 取得目前 preview 搜尋字串，供 UI 顯示狀態使用。
    pub(crate) fn preview_search_query(&self) -> Option<&str> {
        self.preview_search_query.as_deref()
    }

    /// 針對搜尋列表中的某個結果建立 preview，並自動跳到第一個命中位置。
    pub(crate) fn search_preview_for_entry(
        entry: &GlobalSearchEntry,
        viewport_height: usize,
        query: &str,
        requested_scroll: Option<usize>,
        current_match_line: Option<usize>,
        preview_focused: bool,
        theme: Theme,
    ) -> SearchPreviewData {
        let viewport_height = viewport_height.max(1);
        let match_positions = Self::search_preview_match_positions(&entry.path, query);
        let current_match_line = current_match_line
            .or(entry.match_line_number)
            .unwrap_or_else(|| match_positions.first().copied().unwrap_or(1));
        let effective_scroll = requested_scroll.unwrap_or(current_match_line);
        let visible_lines = build_search_preview_lines(
            &entry.path,
            viewport_height,
            query,
            current_match_line,
            theme,
        );
        let mut title = Self::preview_title_for_path(&entry.path, preview_focused, Some(query));
        if effective_scroll > 1 {
            title.push_str("  ^");
        }
        if match_positions
            .iter()
            .any(|line| *line > current_match_line)
        {
            title.push_str("  v");
        }
        SearchPreviewData {
            title,
            lines: visible_lines,
        }
    }

    /// 回傳指定檔案 preview 中所有命中的列位置，供搜尋 preview 導航使用。
    pub(crate) fn search_preview_match_positions(path: &Path, query: &str) -> Vec<usize> {
        let trimmed = query.trim().to_lowercase();
        if trimmed.is_empty() {
            return Vec::new();
        }

        let Ok(file) = File::open(path) else {
            return Vec::new();
        };
        let reader = BufReader::new(file);
        reader
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let Ok(line) = line else {
                    return None;
                };
                line.to_lowercase().contains(&trimmed).then_some(index + 1)
            })
            .collect()
    }

    /// 對 preview 內容套用搜尋條件，並跳到第一個命中的結果。
    pub(crate) fn set_preview_search_query(&mut self, query: &str) {
        let trimmed = query.trim();
        self.preview_search_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        if self.preview_search_query.is_some() {
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_current_match = None;
            self.jump_to_preview_match(true);
        } else {
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_current_match = None;
        }
    }

    /// 清除 preview 搜尋條件與其高亮狀態。
    pub(crate) fn clear_preview_search(&mut self) {
        self.preview_search_query = None;
        self.preview_current_match = None;
    }

    /// 跳到下一個 preview 搜尋命中結果，若已到底則循環回第一個。
    pub(crate) fn jump_to_next_preview_match(&mut self) -> bool {
        self.jump_to_preview_match(true)
    }

    /// 跳到上一個 preview 搜尋命中結果，若已到頂則循環回最後一個。
    pub(crate) fn jump_to_previous_preview_match(&mut self) -> bool {
        self.jump_to_preview_match(false)
    }

    /// 回傳目前 preview 搜尋命中的總數，供狀態列顯示。
    pub(crate) fn preview_match_count(&self) -> usize {
        let Some(query) = self.preview_search_query.as_deref() else {
            return 0;
        };
        let query = query.to_lowercase();
        preview_match_positions(&self.raw_preview_content_lines(), &query).len()
    }

    /// 依照方向跳到 preview 搜尋的下一個或上一個命中位置。
    pub(crate) fn jump_to_preview_match(&mut self, forward: bool) -> bool {
        let Some(query) = self.preview_search_query.as_deref() else {
            return false;
        };

        let query = query.to_lowercase();
        let matches = preview_match_positions(&self.raw_preview_content_lines(), &query);

        let target = if forward {
            self.preview_current_match
                .and_then(|current| current.checked_add(1))
                .filter(|next| *next < matches.len())
                .or(Some(0))
        } else {
            self.preview_current_match
                .and_then(|current| current.checked_sub(1))
                .or_else(|| matches.len().checked_sub(1))
        };
        let Some(target) = target else {
            return false;
        };
        let Some((line_index, _)) = matches.get(target).copied() else {
            return false;
        };

        self.preview_current_match = Some(target);
        self.preview_cursor = line_index;
        self.preview_scroll = line_index.min(self.max_preview_scroll());
        true
    }
}
