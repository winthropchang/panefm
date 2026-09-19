use super::super::PaneState;

impl PaneState {
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
}
