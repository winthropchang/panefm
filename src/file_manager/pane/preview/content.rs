use ratatui::text::{Line, Span};

use super::super::{
    PaneState,
    preview_render::{highlight_cursor_line, highlight_preview_matches},
};
use crate::file_manager::entry::FileEntry;
use crate::theme::Theme;

impl PaneState {
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
    pub(crate) fn get_or_load_preview_diff_lines(
        &self,
        entry: &FileEntry,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
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
    pub(crate) fn preview_diff_lines(
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

    /// 為目前可見預覽行套用游標所在行的高亮（背景色與行號強調）。
    pub(crate) fn apply_preview_cursor_highlight(
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
    pub(crate) fn preview_content_lines(&self, theme: Theme) -> Vec<Line<'static>> {
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
    pub(crate) fn raw_preview_content_lines(&self) -> Vec<Line<'static>> {
        if self.preview_diff_mode {
            let Some(entry) = self.selected_entry() else {
                return Vec::new();
            };
            return self.get_or_load_preview_diff_lines(entry, &Theme::default_theme());
        }
        self.raw_preview_content_lines_limited(usize::MAX)
    }

    /// 產生目前選取項目的 preview 原始內容，並限制最多只建立指定行數。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    /// - `max_lines: usize`，最多建立的 preview 行數。
    ///
    /// 回傳：`Vec<Line<'static>>`，未套用搜尋高亮的 preview 原始內容。
    pub(crate) fn raw_preview_content_lines_limited(&self, max_lines: usize) -> Vec<Line<'static>> {
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
}
