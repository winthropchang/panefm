use super::super::PaneState;
use crate::theme::Theme;

impl PaneState {
    /// 更新 preview 區目前實際可顯示的欄位寬度與高度。
    pub(crate) fn set_preview_viewport_size(&mut self, width: usize, height: usize) {
        self.preview_viewport_width = width.max(10);
        self.set_preview_viewport_height(height);
    }

    /// 更新 preview 區目前實際可顯示的列數，供捲動行為使用。
    pub(crate) fn set_preview_viewport_height(&mut self, height: usize) {
        self.preview_viewport_height = height.max(1);
        if self.preview_scroll > 0 || self.preview_search_query.is_some() {
            self.clamp_preview_scroll();
        }
    }

    /// 回傳目前 preview 內容的總行數。
    pub(crate) fn preview_total_lines(&self) -> usize {
        let Some(entry) = self.selected_entry() else {
            return 0;
        };

        if self.preview_diff_mode {
            let default_theme = Theme::default_theme();
            return self
                .get_or_load_preview_diff_lines(entry, &default_theme)
                .len();
        }

        let ext = entry.path.extension().and_then(|e| e.to_str());
        let is_image = !entry.is_dir && crate::file_manager::preview::is_image_extension(ext);
        if is_image {
            if let Ok(guard) = self.preview_image_cache.lock()
                && let Some(c) = guard.as_ref()
                && c.path == entry.path
            {
                return c.lines.len();
            }
            return 0;
        }

        if let Ok(guard) = self.preview_content_cache.lock()
            && let Some(cache) = guard.get(
                &entry.path,
                Some(entry.modified),
                self.preview_viewport_width,
            )
        {
            cache.total_lines
        } else {
            let _ = self.raw_preview_content_lines_limited(40);
            if let Ok(guard) = self.preview_content_cache.lock()
                && let Some(cache) = guard.get(
                    &entry.path,
                    Some(entry.modified),
                    self.preview_viewport_width,
                )
            {
                cache.total_lines
            } else {
                1
            }
        }
    }

    /// 確保 preview 游標落在可視區域內；若超出則捲動 preview_scroll。
    pub(crate) fn ensure_preview_cursor_visible(&mut self) {
        let total = self.preview_total_lines();
        if total == 0 {
            self.preview_cursor = 0;
            self.preview_scroll = 0;
            return;
        }
        self.preview_cursor = self.preview_cursor.min(total.saturating_sub(1));
        let viewport_height = self.preview_viewport_height.max(1);
        if self.preview_cursor < self.preview_scroll {
            self.preview_scroll = self.preview_cursor;
        } else if self.preview_cursor >= self.preview_scroll + viewport_height {
            self.preview_scroll = self.preview_cursor.saturating_sub(viewport_height - 1);
        }
        self.clamp_preview_scroll();
    }

    /// 將 preview 游標向下移動指定列數（如像文字編輯器般移動游標，僅在游標超出可視範圍時捲動）。
    pub(crate) fn move_preview_cursor_down(&mut self, lines: usize) {
        let total = self.preview_total_lines();
        if total == 0 {
            self.preview_cursor = 0;
            self.preview_scroll = 0;
            return;
        }
        self.preview_cursor = (self.preview_cursor + lines).min(total.saturating_sub(1));
        self.ensure_preview_cursor_visible();
    }

    /// 將 preview 游標向上移動指定列數（如像文字編輯器般移動游標，僅在游標超出可視範圍時捲動）。
    pub(crate) fn move_preview_cursor_up(&mut self, lines: usize) {
        self.preview_cursor = self.preview_cursor.saturating_sub(lines);
        self.ensure_preview_cursor_visible();
    }

    /// 將 preview 向下捲動指定列數。
    pub(crate) fn scroll_preview_down(&mut self, lines: usize) {
        let max_scroll = self.max_preview_scroll();
        let total = self.preview_total_lines();
        self.preview_scroll = (self.preview_scroll + lines).min(max_scroll);
        self.preview_cursor = (self.preview_cursor + lines).min(total.saturating_sub(1));
        self.ensure_preview_cursor_visible();
    }

    /// 將 preview 向上捲動指定列數。
    pub(crate) fn scroll_preview_up(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(lines);
        self.preview_cursor = self.preview_cursor.saturating_sub(lines);
        self.ensure_preview_cursor_visible();
    }

    /// 將 preview 捲到最上方。
    pub(crate) fn scroll_preview_top(&mut self) {
        self.preview_scroll = 0;
        self.preview_cursor = 0;
    }

    /// 將 preview 捲到最下方。
    pub(crate) fn scroll_preview_bottom(&mut self) {
        self.preview_scroll = self.max_preview_scroll();
        let total = self.preview_total_lines();
        self.preview_cursor = total.saturating_sub(1);
        self.ensure_preview_cursor_visible();
    }

    /// 將 preview 游標跳至指定行號（1-indexed）。
    pub(crate) fn move_preview_cursor_to_line(&mut self, line_1_indexed: usize) {
        let total = self.preview_total_lines();
        if total == 0 {
            self.preview_cursor = 0;
            self.preview_scroll = 0;
            return;
        }
        let target_0_indexed = line_1_indexed
            .saturating_sub(1)
            .min(total.saturating_sub(1));
        self.preview_cursor = target_0_indexed;
        self.ensure_preview_cursor_visible();
    }

    /// 依照目前 viewport 高度向下翻半頁。
    #[allow(dead_code)]
    pub(crate) fn page_preview_down(&mut self) {
        let step = (self.preview_viewport_height / 2).max(1);
        self.scroll_preview_down(step);
    }

    /// 依照目前 viewport 高度向上翻半頁。
    pub(crate) fn page_preview_up(&mut self) {
        let step = (self.preview_viewport_height / 2).max(1);
        self.scroll_preview_up(step);
    }

    /// 依照目前 preview viewport 高度向下翻一整頁。
    pub(crate) fn full_page_preview_down(&mut self) {
        let step = self.preview_viewport_height.max(1);
        self.scroll_preview_down(step);
    }

    /// 依照目前 preview viewport 高度向上翻一整頁。
    pub(crate) fn full_page_preview_up(&mut self) {
        let step = self.preview_viewport_height.max(1);
        self.scroll_preview_up(step);
    }

    /// 判斷目前 preview 是否已經有捲動位置。
    pub(crate) fn has_preview_scroll(&self) -> bool {
        self.preview_scroll > 0
    }

    /// 判斷目前 preview 是否還有更多內容可以往下捲動。
    pub(crate) fn preview_has_more_below(&self) -> bool {
        if self.preview_search_query.is_some() {
            return self.preview_scroll < self.max_preview_scroll();
        }

        let viewport_height = self.preview_viewport_height.max(1);
        let total = self.preview_total_lines();
        self.preview_scroll + viewport_height < total
    }

    /// 回傳完整 preview 內容最多可以向下捲到哪一列。
    pub(crate) fn max_preview_scroll(&self) -> usize {
        self.preview_total_lines()
            .saturating_sub(self.preview_viewport_height.max(1))
    }

    /// 當列表或 viewport 發生變化時，把 preview 捲動位置與游標壓回合法範圍。
    pub(crate) fn clamp_preview_scroll(&mut self) {
        self.preview_scroll = self.preview_scroll.min(self.max_preview_scroll());
        let total = self.preview_total_lines();
        if total > 0 {
            self.preview_cursor = self.preview_cursor.min(total.saturating_sub(1));
        } else {
            self.preview_cursor = 0;
        }
    }
}
