//! 目錄內容預覽、項目計數與圖示定義。

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use ratatui::text::Line;

use super::card::{format_preview_item_line, format_size_compact, format_system_time_short};
use crate::file_manager::entry::FileEntry;

/// 快速計算目錄內的子目錄與檔案數量（過濾 .DS_Store 與 .localized，最多計算 200 筆）。
pub fn quick_directory_counts(path: &Path) -> (usize, usize) {
    let Ok(read_dir) = fs::read_dir(path) else {
        return (0, 0);
    };
    let mut dirs = 0;
    let mut files = 0;
    for entry in read_dir.flatten().take(200) {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" || name == ".localized" {
            continue;
        }
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                dirs += 1;
            } else {
                files += 1;
            }
        }
    }
    (dirs, files)
}

/// 格式化目錄預覽頂部標題（方案 A：極簡單排）。
/// 範例：`Desktop  •  9 items (4 dirs, 5 files)  •  2026-09-09 22:47`
pub fn format_directory_title(
    name: &str,
    total: usize,
    dirs: usize,
    files: usize,
    mtime: Option<SystemTime>,
) -> String {
    let mut parts = Vec::new();
    parts.push(name.to_string());

    if total == 0 {
        parts.push("0 items".to_string());
    } else if dirs > 0 && files > 0 {
        parts.push(format!("{total} items ({dirs} dirs, {files} files)"));
    } else if dirs > 0 {
        parts.push(format!("{dirs} dirs"));
    } else {
        parts.push(format!("{files} files"));
    }

    if let Some(time) = mtime {
        let time_str = format_system_time_short(Some(time));
        if time_str != "unknown" {
            parts.push(time_str);
        }
    }

    parts.join("  •  ")
}

/// 依照名稱與是否為目錄，取得與檔案列表完全一致的 Nerd Font 特殊字元圖示。
pub fn preview_item_icon(name: &str, is_dir: bool) -> &'static str {
    if is_dir || name.ends_with('/') {
        return "";
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" => "",
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "tgz" | "zst" => "",
        "rs" | "toml" | "json" | "yaml" | "yml" | "py" | "js" | "ts" | "jsx" | "tsx" | "html"
        | "css" | "scss" | "c" | "cpp" | "h" | "hpp" | "go" | "java" | "sh" | "bash" | "zsh"
        | "sql" | "md" | "markdown" => "",
        "exe" | "com" | "bat" | "cmd" | "ps1" | "bin" => "",
        _ => "",
    }
}

/// 為資料夾產生精進版「目錄偷窺」預覽內容（方案 A：100% 滿版無冗餘，目錄置頂帶 ，檔案帶特殊字元圖示與大小）。
pub(crate) fn preview_directory(
    entry: &FileEntry,
    max_lines: usize,
    viewport_width: usize,
) -> Vec<Line<'static>> {
    let Ok(read_dir) = fs::read_dir(&entry.path) else {
        return vec![Line::from("unable to read directory contents")];
    };

    let mut dir_items = Vec::new();
    let mut file_items = Vec::new();

    for child in read_dir.flatten().take(200) {
        let name = child.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" || name == ".localized" {
            continue;
        }
        let is_dir = child.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        if is_dir {
            dir_items.push(name);
        } else {
            let size = child.metadata().map(|m| m.len()).unwrap_or(0);
            file_items.push((name, size));
        }
    }

    if dir_items.is_empty() && file_items.is_empty() {
        return vec![Line::from("[empty directory]")];
    }

    dir_items.sort_by_key(|a| a.to_lowercase());
    file_items.sort_by_key(|(a, _)| a.to_lowercase());

    let mut lines = Vec::new();
    for dir_name in dir_items {
        if lines.len() >= max_lines {
            break;
        }
        lines.push(format_preview_item_line(
            "",
            &format!("{dir_name}/"),
            None,
            viewport_width,
        ));
    }

    for (file_name, size) in file_items {
        if lines.len() >= max_lines {
            break;
        }
        let icon = preview_item_icon(&file_name, false);
        let size_str = format_size_compact(size);
        lines.push(format_preview_item_line(
            icon,
            &file_name,
            Some(&size_str),
            viewport_width,
        ));
    }

    lines
}
