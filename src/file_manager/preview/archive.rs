//! 壓縮檔案格式識別、內部條目清單預覽與壓縮統計。

use std::fs;
use std::path::Path;

use ratatui::text::Line;

use super::card::{format_preview_item_line, format_size_compact};
use super::directory::preview_item_icon;

/// 判斷檔案是否為支援內部預覽之壓縮檔案。
pub fn is_archive_file(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some("tar.gz")
    } else if name.ends_with(".zip") {
        Some("zip")
    } else if name.ends_with(".tar") {
        Some("tar")
    } else {
        None
    }
}

/// 快速計算壓縮包條目數與總未壓縮大小（最多掃描 300 筆）。
pub fn quick_archive_counts(path: &Path, kind: &str) -> Option<(usize, u64)> {
    let file = fs::File::open(path).ok()?;
    if kind == "zip" {
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let count = archive.len();
        let mut total: u64 = 0;
        for i in 0..count.min(300) {
            if let Ok(entry) = archive.by_index(i) {
                total = total.saturating_add(entry.size());
            }
        }
        Some((count, total))
    } else if kind == "tar.gz" {
        let gz = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(gz);
        let entries = tar.entries().ok()?;
        let mut count = 0;
        let mut total: u64 = 0;
        for entry in entries.take(300).flatten() {
            count += 1;
            total = total.saturating_add(entry.size());
        }
        Some((count, total))
    } else if kind == "tar" {
        let mut tar = tar::Archive::new(file);
        let entries = tar.entries().ok()?;
        let mut count = 0;
        let mut total: u64 = 0;
        for entry in entries.take(300).flatten() {
            count += 1;
            total = total.saturating_add(entry.size());
        }
        Some((count, total))
    } else {
        None
    }
}

/// 格式化壓縮檔預覽標題。
/// 範例：`bundle.zip  •  48 entries  •  1.42 MiB uncompressed`
pub fn format_archive_title(name: &str, entry_count: usize, total_uncompressed: u64) -> String {
    let size_str = format_size_compact(total_uncompressed);
    format!("{name}  •  {entry_count} entries  •  {size_str} uncompressed")
}

/// 預覽壓縮包內部條目清單（支援 .zip、.tar.gz、.tgz、.tar）。
pub fn preview_archive_content(
    path: &Path,
    kind: &str,
    max_lines: usize,
    viewport_width: usize,
) -> Option<Vec<Line<'static>>> {
    let file = fs::File::open(path).ok()?;
    let mut items = Vec::new();

    if kind == "zip" {
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let len = archive.len();
        for i in 0..len.min(300) {
            if let Ok(entry) = archive.by_index(i) {
                let name = entry.name().to_string();
                let size = entry.size();
                let is_dir = entry.is_dir() || name.ends_with('/');
                items.push((name, is_dir, size));
            }
        }
    } else if kind == "tar.gz" {
        let gz = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(gz);
        let entries = tar.entries().ok()?;
        for entry in entries.take(300).flatten() {
            let name = entry
                .path()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let size = entry.size();
            let is_dir = entry.header().entry_type().is_dir() || name.ends_with('/');
            items.push((name, is_dir, size));
        }
    } else if kind == "tar" {
        let mut tar = tar::Archive::new(file);
        let entries = tar.entries().ok()?;
        for entry in entries.take(300).flatten() {
            let name = entry
                .path()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let size = entry.size();
            let is_dir = entry.header().entry_type().is_dir() || name.ends_with('/');
            items.push((name, is_dir, size));
        }
    } else {
        return None;
    }

    if items.is_empty() {
        return Some(vec![Line::from("[empty archive]")]);
    }

    let mut lines = Vec::new();
    for (name, is_dir, size) in items.into_iter().take(max_lines) {
        let icon = preview_item_icon(&name, is_dir);
        if is_dir {
            let display_name = if name.ends_with('/') {
                name
            } else {
                format!("{name}/")
            };
            lines.push(format_preview_item_line(
                icon,
                &display_name,
                None,
                viewport_width,
            ));
        } else {
            let size_str = format_size_compact(size);
            lines.push(format_preview_item_line(
                icon,
                &name,
                Some(&size_str),
                viewport_width,
            ));
        }
    }

    Some(lines)
}
