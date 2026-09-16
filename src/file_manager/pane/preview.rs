use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use ratatui::text::Line;

use super::{
    PaneState,
    preview_render::{
        build_search_preview_lines, highlight_cursor_line, highlight_preview_matches,
        preview_match_positions,
    },
    types::SearchPreviewData,
};
use crate::file_manager::{entry::FileEntry, search::GlobalSearchEntry};
use crate::theme::Theme;

impl PaneState {
    pub(crate) fn is_preview_open(&self) -> bool {
        self.preview_open
    }

    /// 明確設定目前 panel 的雙欄即時預覽開啟狀態。
    pub(crate) fn set_preview_open(&mut self, open: bool) {
        self.preview_open = open;
        if !open {
            self.preview_focused = false;
        }
        self.preview_active = self.preview_open && self.preview_focused;
    }

    /// 切換目前 panel 的雙欄即時預覽開啟狀態。
    pub(crate) fn toggle_preview_open(&mut self) -> bool {
        self.preview_open = !self.preview_open;
        if self.preview_open {
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_diff_mode = false;
            self.preview_search_query = None;
            self.preview_current_match = None;
            self.preview_focused = true;
        } else {
            self.preview_focused = false;
            self.preview_diff_mode = false;
        }
        self.preview_active = self.preview_open && self.preview_focused;
        self.preview_open
    }

    /// 切換預覽的 VCS Diff 差異模式。
    /// 若預覽未開啟，則同時開啟預覽並切換至 diff 模式。
    /// 若預覽已開啟且處於 diff 模式，則切回全文模式。
    /// 若預覽已開啟但處於全文模式，則切換至 diff 模式。
    /// 回傳切換後是否處於 diff 模式。
    pub(crate) fn toggle_preview_diff_mode(&mut self) -> bool {
        if !self.preview_open {
            self.preview_open = true;
            self.preview_diff_mode = true;
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_search_query = None;
            self.preview_current_match = None;
            self.preview_focused = true;
        } else {
            self.preview_diff_mode = !self.preview_diff_mode;
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_search_query = None;
            self.preview_current_match = None;
            self.preview_focused = true;
        }
        self.preview_active = self.preview_open && self.preview_focused;
        self.preview_diff_mode
    }

    /// 判斷當前焦點是否正處於右側預覽（可進行滾動、跳頁與預覽搜尋）。
    pub(crate) fn is_preview_focused(&self) -> bool {
        self.preview_open && self.preview_focused
    }

    /// 設定焦點是否切入右側預覽視窗。
    pub(crate) fn set_preview_focused(&mut self, focused: bool) {
        if self.preview_open {
            self.preview_focused = focused;
        } else {
            self.preview_focused = false;
        }
        self.preview_active = self.preview_open && self.preview_focused;
    }

    /// 判斷目前 panel 是否正在接收預覽操作按鍵（等同於 is_preview_focused）。
    pub(crate) fn is_preview_active(&self) -> bool {
        self.preview_open && self.preview_focused
    }

    /// 明確設定目前 panel 的 preview 操作狀態。
    pub(crate) fn set_preview_active(&mut self, active: bool) {
        self.preview_open = active;
        self.preview_focused = active;
        self.preview_active = active;
    }

    /// 切換目前 panel 的 preview 顯示狀態，並回傳切換後的結果。
    #[allow(dead_code)]
    pub(crate) fn toggle_preview_active(&mut self) -> bool {
        self.toggle_preview_open()
    }

    /// 依照目前選取項目產生預覽區要顯示的文字行。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    /// - `max_lines: usize`，預覽區最多顯示的行數。
    ///
    /// 回傳：`Vec<Line<'static>>`，可直接交給 `ratatui` 的 Paragraph 渲染。
    pub(crate) fn preview_lines(&self, max_lines: usize, theme: Theme) -> Vec<Line<'static>> {
        let max_lines = max_lines.max(1);
        let Some(entry) = self.selected_entry() else {
            return vec![Line::from("empty directory")];
        };

        if self.preview_diff_mode {
            return self.preview_diff_lines(entry, max_lines, theme);
        }

        if self.preview_search_query.is_some() {
            return self
                .preview_content_lines(theme)
                .into_iter()
                .skip(self.preview_scroll)
                .take(max_lines)
                .collect();
        }

        let ext = entry.path.extension().and_then(|e| e.to_str());
        let is_image = !entry.is_dir && crate::file_manager::preview::is_image_extension(ext);
        let is_archive =
            !entry.is_dir && crate::file_manager::preview::is_archive_file(&entry.path).is_some();

        // 目錄、圖片、壓縮包維持原有快速分流
        if entry.is_dir || is_image || is_archive {
            let needed_lines = self.preview_scroll + max_lines;
            let lines = self
                .raw_preview_content_lines_limited(needed_lines)
                .into_iter()
                .skip(self.preview_scroll)
                .take(max_lines)
                .collect();
            if is_image {
                return lines;
            }
            return self.apply_preview_cursor_highlight(lines, theme);
        }

        let start_line = self.preview_scroll;
        let end_line = start_line + max_lines;

        // 檢查快取
        let result_lines = if let Ok(guard) = self.preview_content_cache.lock()
            && let Some(cache) = guard.get(
                &entry.path,
                Some(entry.modified),
                self.preview_viewport_width,
            ) {
            // 若快取已涵蓋可見區間，或全文已經由背景執行緒解析完畢
            if cache.lines.len() >= end_line || cache.is_complete {
                cache
                    .lines
                    .iter()
                    .skip(start_line)
                    .take(max_lines)
                    .cloned()
                    .collect()
            } else {
                // 快取存在但目標區間超出目前快取行數（例如剛打開檔案即按下 G 跳至第 5000 行）：
                // 立即以 < 1ms 切片渲染可見行，絕不阻塞主 UI 執行緒！
                crate::file_manager::preview::preview_file_slice(
                    &entry.path,
                    start_line,
                    max_lines,
                    cache.total_lines,
                )
            }
        } else {
            // 快取尚未建立（首次開啟）：先載入首屏 40 行，並在背景啟動全文高亮
            let initial_lines = self.raw_preview_content_lines_limited(end_line.min(40));
            if let Ok(guard) = self.preview_content_cache.lock()
                && let Some(cache) = guard.get(
                    &entry.path,
                    Some(entry.modified),
                    self.preview_viewport_width,
                )
            {
                if start_line < initial_lines.len() {
                    initial_lines
                        .into_iter()
                        .skip(start_line)
                        .take(max_lines)
                        .collect()
                } else {
                    crate::file_manager::preview::preview_file_slice(
                        &entry.path,
                        start_line,
                        max_lines,
                        cache.total_lines,
                    )
                }
            } else {
                initial_lines
                    .into_iter()
                    .skip(start_line)
                    .take(max_lines)
                    .collect()
            }
        };

        self.prefetch_adjacent_previews();
        self.apply_preview_cursor_highlight(result_lines, theme)
    }

    /// 取得或載入當前選取檔案的 VCS Diff 行清單（帶快取保護）。
    fn get_or_load_preview_diff_lines(
        &self,
        entry: &FileEntry,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        use ratatui::text::Span;

        if entry.is_dir {
            return vec![Line::from(Span::styled(
                "[Directories do not support diff preview - press Ctrl+d for preview]",
                theme.muted_style(),
            ))];
        }

        if let Ok(guard) = self.preview_diff_cache.lock()
            && let Some((cached_path, cached_mtime, lines)) = guard.as_ref()
            && cached_path == &entry.path
            && *cached_mtime == Some(entry.modified)
        {
            return lines.clone();
        }

        let diff_output = crate::file_manager::vcs::query_vcs_file_diff(&entry.path);
        let lines = match diff_output {
            Some(diff) if !diff.trim().is_empty() => {
                crate::file_manager::vcs::format_diff_lines(&diff, theme)
            }
            _ => {
                vec![Line::from(Span::styled(
                    "[No VCS modifications detected in Git/SVN]",
                    theme.muted_style(),
                ))]
            }
        };

        if let Ok(mut guard) = self.preview_diff_cache.lock() {
            *guard = Some((entry.path.clone(), Some(entry.modified), lines.clone()));
        }

        lines
    }

    /// 取得當前選取檔案的 VCS Diff 行清單，支援快取、搜尋高亮、捲動切片與游標所在行高亮。
    fn preview_diff_lines(
        &self,
        entry: &FileEntry,
        max_lines: usize,
        theme: Theme,
    ) -> Vec<Line<'static>> {
        let all_lines = self.get_or_load_preview_diff_lines(entry, &theme);
        let all_lines = if let Some(query) = self.preview_search_query.as_deref() {
            highlight_preview_matches(all_lines, query, theme, self.preview_current_match)
        } else {
            all_lines
        };
        let visible_lines: Vec<Line<'static>> = all_lines
            .into_iter()
            .skip(self.preview_scroll)
            .take(max_lines)
            .collect();

        if self.preview_search_query.is_some() {
            visible_lines
        } else {
            self.apply_preview_cursor_highlight(visible_lines, theme)
        }
    }

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

    /// 判斷目前 preview 是否正在套用搜尋條件。
    pub(crate) fn has_preview_search(&self) -> bool {
        self.preview_search_query.is_some()
    }

    /// 取得目前 preview 搜尋字串，供 UI 顯示狀態使用。
    pub(crate) fn preview_search_query(&self) -> Option<&str> {
        self.preview_search_query.as_deref()
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

    /// 為目前可見預覽行套用游標所在行的高亮（背景色與行號強調）。
    fn apply_preview_cursor_highlight(
        &self,
        lines: Vec<Line<'static>>,
        theme: Theme,
    ) -> Vec<Line<'static>> {
        let start_line = self.preview_scroll;
        let cursor_line = self.preview_cursor;

        lines
            .into_iter()
            .enumerate()
            .map(|(offset, line)| {
                let global_idx = start_line + offset;
                if global_idx == cursor_line {
                    highlight_cursor_line(line, theme)
                } else {
                    line
                }
            })
            .collect()
    }

    /// 產生目前選取項目的完整 preview 內容，供捲動切片與上下界計算使用。
    fn preview_content_lines(&self, theme: Theme) -> Vec<Line<'static>> {
        let lines = if self.preview_diff_mode {
            let Some(entry) = self.selected_entry() else {
                return Vec::new();
            };
            self.get_or_load_preview_diff_lines(entry, &theme)
        } else {
            self.raw_preview_content_lines()
        };
        if let Some(query) = self.preview_search_query.as_deref() {
            highlight_preview_matches(lines, query, theme, self.preview_current_match)
        } else {
            lines
        }
    }

    /// 產生目前選取項目的完整 preview 原始內容，不套用任何搜尋高亮。
    fn raw_preview_content_lines(&self) -> Vec<Line<'static>> {
        if self.preview_diff_mode {
            let Some(entry) = self.selected_entry() else {
                return Vec::new();
            };
            return self.get_or_load_preview_diff_lines(entry, &Theme::default_theme());
        }
        self.raw_preview_content_lines_limited(usize::MAX)
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

    /// 依照指定路徑建立 preview 區塊標題。
    pub(crate) fn preview_title_for_path(
        path: &Path,
        preview_focused: bool,
        query: Option<&str>,
    ) -> String {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let mut title = format!("Preview: {name}");
        if preview_focused {
            title.push_str("  [preview]");
        }
        if let Some(query) = query.filter(|query| !query.trim().is_empty()) {
            title.push_str(&format!("  [/{query}]"));
        }
        title
    }

    /// 依照目前選取的 entry 與 pane 狀態建立 preview 標題。
    /// 若為目錄或壓縮檔案，直接在邊框顯示單排精煉資訊。
    /// 若為圖片檔案，直接在邊框顯示單排精煉資訊：`1920 × 25000 (PNG)  •  13.85 MiB  •  2026-01-01 08:59`。
    pub(crate) fn preview_title_for_entry(&self, entry: &FileEntry) -> String {
        if self.preview_diff_mode {
            let mut title = format!("Preview [diff]: {}  (press Ctrl+d for full)", entry.name);
            if self.is_preview_focused() {
                title.push_str("  [preview]");
            }
            if let Some(query) = self.preview_search_query() {
                title.push_str(&format!("  [/{query}]"));
            }
            if self.has_preview_scroll() {
                title.push_str("  ^");
            }
            if self.preview_has_more_below() {
                title.push_str("  v");
            }
            return title;
        }

        if entry.is_dir {
            let (dir_count, file_count) =
                crate::file_manager::preview::quick_directory_counts(&entry.path);
            let total = dir_count + file_count;
            let mut title = crate::file_manager::preview::format_directory_title(
                &entry.name,
                total,
                dir_count,
                file_count,
                Some(entry.modified),
            );
            if self.has_preview_scroll() {
                title.push_str("  ^");
            }
            if self.preview_has_more_below() {
                title.push_str("  v");
            }
            return title;
        }

        if let Some(kind) = crate::file_manager::preview::is_archive_file(&entry.path)
            && let Some((count, uncompressed)) =
                crate::file_manager::preview::quick_archive_counts(&entry.path, kind)
        {
            let mut title = crate::file_manager::preview::format_archive_title(
                &entry.name,
                count,
                uncompressed,
            );
            if self.has_preview_scroll() {
                title.push_str("  ^");
            }
            if self.preview_has_more_below() {
                title.push_str("  v");
            }
            return title;
        }

        let ext = entry.path.extension().and_then(|e| e.to_str());
        if !entry.is_dir && crate::file_manager::preview::is_image_extension(ext) {
            let dimensions = if let Ok(guard) = self.preview_image_cache.lock() {
                guard.as_ref().and_then(|c| {
                    if c.path == entry.path {
                        c.dimensions
                    } else {
                        None
                    }
                })
            } else {
                None
            }
            .or_else(|| crate::file_manager::preview::read_image_dimensions(&entry.path));

            let is_loading = if let Ok(guard) = self.preview_image_loader.lock() {
                guard
                    .as_ref()
                    .map(|l| l.path == entry.path)
                    .unwrap_or(false)
            } else {
                false
            };

            let mut title = crate::file_manager::preview::format_image_title(
                dimensions,
                ext,
                entry.size,
                Some(entry.modified),
                is_loading,
            );

            if self.has_preview_scroll() {
                title.push_str("  ^");
            }
            if self.preview_has_more_below() {
                title.push_str("  v");
            }
            return title;
        }

        let mut title = format!("Preview: {}", entry.name);
        title.push_str("  [preview]");
        if let Some(query) = self.preview_search_query() {
            title.push_str(&format!("  [/{query}]"));
        }
        if self.has_preview_scroll() {
            title.push_str("  ^");
        }
        if self.preview_has_more_below() {
            title.push_str("  v");
        }
        title
    }

    /// 產生目前選取項目的 preview 原始內容，並限制最多只建立指定行數。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    /// - `max_lines: usize`，最多建立的 preview 行數。
    ///
    /// 回傳：`Vec<Line<'static>>`，未套用搜尋高亮的 preview 原始內容。
    fn raw_preview_content_lines_limited(&self, max_lines: usize) -> Vec<Line<'static>> {
        let Some(entry) = self.selected_entry() else {
            return vec![Line::from("empty directory")];
        };

        let ext = entry.path.extension().and_then(|e| e.to_str());
        let is_image = !entry.is_dir && crate::file_manager::preview::is_image_extension(ext);

        if !is_image
            && let Ok(guard) = self.preview_content_cache.lock()
            && let Some(cache) = guard.get(
                &entry.path,
                Some(entry.modified),
                self.preview_viewport_width,
            )
            && (cache.lines.len() >= max_lines || cache.is_complete)
        {
            let mut lines = cache.lines.clone();
            lines.truncate(max_lines);
            return lines;
        }

        let (lines, total_lines) = if entry.is_dir {
            let lines = crate::file_manager::preview::preview_directory(
                entry,
                usize::MAX,
                self.preview_viewport_width,
            );
            let count = lines.len();
            (lines, count)
        } else {
            let mut fallback_cache = None;
            let mut cache_guard = self.preview_image_cache.lock().ok();
            let cache_ref = cache_guard.as_deref_mut().unwrap_or(&mut fallback_cache);

            let mut fallback_loader = None;
            let mut loader_guard = self.preview_image_loader.lock().ok();
            let loader_ref = loader_guard.as_deref_mut().unwrap_or(&mut fallback_loader);

            let request_lines = if is_image {
                max_lines
            } else {
                max_lines.max(40)
            };
            crate::file_manager::preview::preview_file_content_detailed(
                &entry.path,
                request_lines,
                self.preview_viewport_width,
                self.preview_viewport_height,
                cache_ref,
                loader_ref,
            )
        };

        if !is_image {
            let is_complete = lines.len() >= total_lines;
            if let Ok(mut guard) = self.preview_content_cache.lock() {
                guard.put_current(crate::file_manager::preview::PreviewContentCache {
                    path: entry.path.clone(),
                    modified: Some(entry.modified),
                    viewport_width: self.preview_viewport_width,
                    total_lines,
                    lines: lines.clone(),
                    is_complete,
                });
            }

            if !is_complete {
                let bg_path = entry.path.clone();
                let bg_modified = Some(entry.modified);
                let bg_cache = self.preview_content_cache.clone();
                std::thread::spawn(move || {
                    if let Ok(bytes) = std::fs::read(&bg_path)
                        && let Ok(contents) = String::from_utf8(bytes)
                    {
                        let full_lines = crate::file_manager::preview::highlight_code_preview(
                            &bg_path,
                            &contents,
                            usize::MAX,
                            None,
                        );
                        if let Ok(mut guard) = bg_cache.lock() {
                            guard.update_complete(&bg_path, bg_modified, full_lines);
                        }
                    }
                });
            }
        }

        let mut output = lines;
        output.truncate(max_lines);
        output
    }

    /// 投機性預熱鄰近項目的預覽內容（selected + 1 與 selected - 1）。
    /// 在背景執行緒中預先完成讀檔與語法高亮，使 j/k 切換時達到 0ms 快取命中。
    pub(crate) fn prefetch_adjacent_previews(&self) {
        self.prefetch_current_and_adjacent_previews(false);
    }

    /// 投機性預熱當前選取項目與鄰近項目的預覽內容。
    ///
    /// 參數：
    /// - `include_current: bool`：若為 true，將當前游標停駐的項目也納入預熱（用於清單模式）。
    pub(crate) fn prefetch_current_and_adjacent_previews(&self, include_current: bool) {
        let current_index = self.selected;
        let viewport_width = self.preview_viewport_width;
        let visible_count = self.visible_indices.len();
        if visible_count == 0 {
            return;
        }

        let mut candidate_indices = Vec::new();
        if include_current {
            candidate_indices.push(current_index);
        }
        if current_index + 1 < visible_count {
            candidate_indices.push(current_index + 1);
        }
        if current_index > 0 {
            candidate_indices.push(current_index - 1);
        }

        let mut to_prefetch = Vec::new();
        if let Ok(mut guard) = self.preview_content_cache.lock() {
            for idx in candidate_indices {
                if let Some(&entry_index) = self.visible_indices.get(idx)
                    && let Some(entry) = self.entries.get(entry_index)
                    && !entry.is_dir
                {
                    let ext = entry.path.extension().and_then(|e| e.to_str());
                    if !crate::file_manager::preview::is_image_extension(ext)
                        && !guard.is_in_flight_or_cached(
                            &entry.path,
                            Some(entry.modified),
                            viewport_width,
                        )
                    {
                        guard.mark_in_flight(entry.path.clone());
                        to_prefetch.push((entry.path.clone(), entry.modified));
                    }
                }
            }
        }

        if !to_prefetch.is_empty() {
            let bg_cache = self.preview_content_cache.clone();
            std::thread::spawn(move || {
                for (path, modified) in to_prefetch {
                    let mut loaded = false;
                    if let Ok(metadata) = std::fs::metadata(&path)
                        && metadata.len() <= crate::file_manager::preview::MAX_TEXT_PREVIEW_SIZE
                        && let Ok(bytes) = std::fs::read(&path)
                        && let Ok(contents) = String::from_utf8(bytes)
                    {
                        let total_lines = contents.lines().count().max(1);
                        let is_complete = total_lines <= 2000;
                        let max_lines = if is_complete { total_lines } else { 200 };
                        let lines = crate::file_manager::preview::highlight_code_preview(
                            &path, &contents, max_lines, None,
                        );
                        if let Ok(mut guard) = bg_cache.lock() {
                            guard.put(crate::file_manager::preview::PreviewContentCache {
                                path: path.clone(),
                                modified: Some(modified),
                                viewport_width,
                                total_lines,
                                lines,
                                is_complete,
                            });
                            loaded = true;
                        }
                    }
                    if !loaded && let Ok(mut guard) = bg_cache.lock() {
                        guard.clear_in_flight(&path);
                    }
                }
            });
        }
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
    fn jump_to_preview_match(&mut self, forward: bool) -> bool {
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
