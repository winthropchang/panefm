use std::path::Path;

use super::super::PaneState;
use crate::file_manager::entry::FileEntry;

impl PaneState {
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
}
