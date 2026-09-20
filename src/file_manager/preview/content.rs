//! 檔案預覽總調度器，協調文字、程式碼語法、圖片 Halfblock、壓縮封裝與詳細資訊卡片。

use std::fs;
use std::path::Path;

use ratatui::text::Line;

use super::archive::{is_archive_file, preview_archive_content};
use super::card::format_file_details_preview;
use super::halfblock::{
    detect_legacy_image_summary, format_image_loading_preview, is_image_extension,
    render_image_halfblocks_with_dimensions,
};
use super::syntax::{highlight_code_preview, highlight_code_preview_slice};
use super::types::{ImageLoader, ImagePreviewCache};
use super::{MAX_IMAGE_PREVIEW_SIZE, MAX_TEXT_PREVIEW_SIZE};

/// 產生檔案指定行區間之預覽內容切片（用於跳頁、按 G 等快速捲動時避免全檔同步解析阻塞）。
pub(crate) fn preview_file_slice(
    path: &Path,
    start_line: usize,
    count: usize,
    total_lines: usize,
) -> Vec<Line<'static>> {
    let Ok(bytes) = fs::read(path) else {
        return vec![Line::from("unable to read file contents")];
    };
    match String::from_utf8(bytes) {
        Ok(contents) => {
            highlight_code_preview_slice(path, &contents, start_line, count, total_lines, None)
        }
        Err(_) => vec![Line::from("binary or non-utf8 file")],
    }
}

/// 產生檔案預覽行清單與檔案總行數（結合文字顯示、圖片 Halfblock 與大檔案/二進位詳細資訊卡片）。
pub(crate) fn preview_file_content_detailed(
    path: &Path,
    max_lines: usize,
    viewport_width: usize,
    viewport_height: usize,
    image_cache: &mut Option<ImagePreviewCache>,
    image_loader: &mut Option<ImageLoader>,
) -> (Vec<Line<'static>>, usize) {
    let Ok(metadata) = fs::metadata(path) else {
        return (vec![Line::from("unable to read metadata")], 1);
    };

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase());

    // 1. 圖片檔案分流
    if is_image_extension(extension.as_deref()) {
        if metadata.len() > MAX_IMAGE_PREVIEW_SIZE {
            let lines = format_file_details_preview(
                path,
                &metadata,
                Some("image exceeds 30 MiB preview limit"),
            );
            let total = lines.len();
            return (lines, total);
        }

        let mtime = metadata.modified().ok();
        let target_cols = viewport_width.max(20);
        let target_rows = viewport_height.max(4);

        // 檢查快取是否命中
        if let Some(cache) = image_cache
            && cache.path == path
            && cache.modified == mtime
            && cache.max_cols == target_cols
            && cache.max_rows == target_rows
        {
            let mut lines = cache.lines.clone();
            let total = lines.len();
            lines.truncate(max_lines);
            return (lines, total);
        }

        // 檢查背景非同步載入器
        if let Some(loader) = image_loader {
            if loader.path == path
                && loader.modified == mtime
                && loader.max_cols == target_cols
                && loader.max_rows == target_rows
            {
                match loader.receiver.try_recv() {
                    Ok(Ok((rendered, dims))) => {
                        *image_cache = Some(ImagePreviewCache {
                            path: path.to_path_buf(),
                            modified: mtime,
                            max_cols: target_cols,
                            max_rows: target_rows,
                            dimensions: Some(dims),
                            lines: rendered.clone(),
                        });
                        *image_loader = None;
                        let total = rendered.len();
                        let mut lines = rendered;
                        lines.truncate(max_lines);
                        return (lines, total);
                    }
                    Ok(Err(_)) => {
                        *image_loader = None;
                        let lines = format_file_details_preview(
                            path,
                            &metadata,
                            Some("image format unsupported or decode failed"),
                        );
                        let total = lines.len();
                        return (lines, total);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        let lines = format_image_loading_preview(max_lines);
                        let total = lines.len();
                        return (lines, total);
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        *image_loader = None;
                    }
                }
            } else {
                *image_loader = None;
            }
        }

        // 若檔案很小（<= 256 KiB），直接在當前執行緒快速秒解（耗時 < 2ms）
        let small_file_bytes = if metadata.len() <= 256 * 1024 {
            fs::read(path).ok()
        } else {
            None
        };
        if let Some(bytes) = small_file_bytes {
            if let Ok((rendered, dims)) =
                render_image_halfblocks_with_dimensions(&bytes, target_cols, target_rows)
            {
                *image_cache = Some(ImagePreviewCache {
                    path: path.to_path_buf(),
                    modified: mtime,
                    max_cols: target_cols,
                    max_rows: target_rows,
                    dimensions: Some(dims),
                    lines: rendered.clone(),
                });
                let total = rendered.len();
                let mut lines = rendered;
                lines.truncate(max_lines);
                return (lines, total);
            } else if let Some(summary) = detect_legacy_image_summary(&bytes, extension.as_deref())
            {
                let total = summary.len();
                let mut lines = summary;
                lines.truncate(max_lines);
                return (lines, total);
            }
        }

        // 大於 256 KiB 的圖片：啟動背景非同步解碼執行緒，讓 TUI 保持 0ms 完全流暢
        let (tx, rx) = std::sync::mpsc::channel();
        let path_buf = path.to_path_buf();
        std::thread::spawn(move || {
            let result = (|| -> Result<(Vec<Line<'static>>, (u32, u32)), String> {
                let bytes = fs::read(&path_buf).map_err(|e| e.to_string())?;
                render_image_halfblocks_with_dimensions(&bytes, target_cols, target_rows)
            })();
            let _ = tx.send(result);
        });

        *image_loader = Some(ImageLoader {
            path: path.to_path_buf(),
            modified: mtime,
            max_cols: target_cols,
            max_rows: target_rows,
            receiver: rx,
        });

        let lines = format_image_loading_preview(max_lines);
        let total = lines.len();
        return (lines, total);
    }

    // 2. 壓縮封裝檔案分流（免解壓內部檔案樹預覽）
    if let Some(kind) = is_archive_file(path) {
        if let Some(archive_lines) = preview_archive_content(path, kind, max_lines, viewport_width)
        {
            let total = archive_lines.len();
            return (archive_lines, total);
        }

        // 壓縮檔無法正常解析內部結構：給出精確診斷，避免誤導成純文字大小限制
        #[cfg(unix)]
        let is_sparse = {
            use std::os::unix::fs::MetadataExt;
            metadata.is_file() && metadata.len() > 0 && metadata.blocks() == 0
        };
        #[cfg(not(unix))]
        let is_sparse = false;

        let notice = if is_sparse {
            "warning: archive has 0 blocks allocated on disk (transfer incomplete or dataless placeholder)"
        } else if metadata.len() == 0 {
            "archive file is empty (0 bytes)"
        } else {
            "unable to read archive contents (corrupted, incomplete, or unsupported format)"
        };

        let lines = format_file_details_preview(path, &metadata, Some(notice));
        let total = lines.len();
        return (lines, total);
    }

    // 3. 非圖片大檔案：超過 2 MiB 時顯示結構化詳細資訊卡片
    if metadata.len() > MAX_TEXT_PREVIEW_SIZE {
        let lines = format_file_details_preview(
            path,
            &metadata,
            Some("file size exceeds 2 MiB text preview limit"),
        );
        let total = lines.len();
        return (lines, total);
    }

    // 4. 小於等於 2 MiB 的檔案：嘗試讀取並以語法高亮預覽
    let Ok(bytes) = fs::read(path) else {
        let lines =
            format_file_details_preview(path, &metadata, Some("unable to read file contents"));
        let total = lines.len();
        return (lines, total);
    };

    match String::from_utf8(bytes) {
        Ok(contents) => {
            let total_lines = contents.lines().count().max(1);
            let lines = highlight_code_preview(path, &contents, max_lines, None);
            (lines, total_lines)
        }
        Err(_) => {
            let lines =
                format_file_details_preview(path, &metadata, Some("binary or non-utf8 file"));
            let total = lines.len();
            (lines, total)
        }
    }
}

/// 產生檔案預覽行清單（結合文字顯示、圖片 Halfblock 與大檔案/二進位詳細資訊卡片）。
#[allow(dead_code)]
pub(crate) fn preview_file_content(
    path: &Path,
    max_lines: usize,
    viewport_width: usize,
    viewport_height: usize,
    image_cache: &mut Option<ImagePreviewCache>,
    image_loader: &mut Option<ImageLoader>,
) -> Vec<Line<'static>> {
    preview_file_content_detailed(
        path,
        max_lines,
        viewport_width,
        viewport_height,
        image_cache,
        image_loader,
    )
    .0
}
