//! 檔案詳細資訊卡片、檔案類型推測與格式化工具。

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use ratatui::text::Line;

/// 將千位數加上逗號分隔符號，提升大型數字易讀性。
pub fn format_number_with_commas(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut parts = Vec::new();
    while n > 0 {
        let remainder = n % 1000;
        n /= 1000;
        if n > 0 {
            parts.push(format!("{remainder:03}"));
        } else {
            parts.push(remainder.to_string());
        }
    }
    parts.reverse();
    parts.join(",")
}

/// 格式化檔案大小，同時附上以 1024 為基數的簡短單位與精確 byte 數。
pub fn format_size_with_bytes(size: u64) -> String {
    const K: f64 = 1_024.0;
    const M: f64 = K * 1_024.0;
    const G: f64 = M * 1_024.0;
    const T: f64 = G * 1_024.0;

    let size_f = size as f64;
    let short_str = if size_f >= T {
        format!("{:.2} TiB", size_f / T)
    } else if size_f >= G {
        format!("{:.2} GiB", size_f / G)
    } else if size_f >= M {
        format!("{:.2} MiB", size_f / M)
    } else if size_f >= K {
        format!("{:.1} KiB", size_f / K)
    } else {
        format!("{size} B")
    };

    let comma_bytes = format_number_with_commas(size);
    format!("{short_str} ({comma_bytes} bytes)")
}

/// 格式化檔案大小簡短表示（如 `13.85 MiB` 或 `512 B`）。
pub fn format_size_compact(size: u64) -> String {
    const K: f64 = 1_024.0;
    const M: f64 = K * 1_024.0;
    const G: f64 = M * 1_024.0;
    const T: f64 = G * 1_024.0;

    let size_f = size as f64;
    if size_f >= T {
        format!("{:.2} TiB", size_f / T)
    } else if size_f >= G {
        format!("{:.2} GiB", size_f / G)
    } else if size_f >= M {
        format!("{:.2} MiB", size_f / M)
    } else if size_f >= K {
        format!("{:.1} KiB", size_f / K)
    } else {
        format!("{size} B")
    }
}

/// 將 `SystemTime` 轉成簡短時間格式（`YYYY-MM-DD HH:MM`）。
pub(crate) fn format_system_time_short(time: Option<SystemTime>) -> String {
    match time {
        Some(time) => {
            let dt: DateTime<Local> = time.into();
            dt.format("%Y-%m-%d %H:%M").to_string()
        }
        None => "unknown".to_string(),
    }
}

/// 將 `SystemTime` 轉成使用者易讀的本地時間格式。
pub(crate) fn format_system_time(time: std::io::Result<SystemTime>) -> String {
    match time {
        Ok(time) => {
            let dt: DateTime<Local> = time.into();
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        }
        Err(_) => "unknown".to_string(),
    }
}

/// 格式化檔案權限字串（Unix 輸出例如 `-rw-r--r-- (0644)`，Windows 輸出存取狀態）。
#[cfg(unix)]
pub(crate) fn format_permissions(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    let is_dir = metadata.is_dir();
    let prefix = if is_dir { 'd' } else { '-' };
    let rwx = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    let mut chars = String::with_capacity(10);
    chars.push(prefix);
    for (mask, c) in rwx {
        chars.push(if mode & mask != 0 { c } else { '-' });
    }
    format!("{chars} ({:04o})", mode & 0o7777)
}

/// Windows 平台的檔案權限字串。
#[cfg(not(unix))]
pub(crate) fn format_permissions(metadata: &fs::Metadata) -> String {
    if metadata.permissions().readonly() {
        "read-only".to_string()
    } else {
        "read-write".to_string()
    }
}

/// 根據路徑副檔名推測常見檔案的類型說明。
pub fn detect_file_kind(path: &Path, extension: Option<&str>) -> &'static str {
    if path.is_dir() {
        return "Directory";
    }
    let ext = extension.unwrap_or_default().to_ascii_lowercase();
    match ext.as_str() {
        // 圖片
        "png" => "PNG Image",
        "jpg" | "jpeg" => "JPEG Image",
        "gif" => "GIF Image",
        "webp" => "WebP Image",
        "bmp" => "BMP Image",
        "ico" => "ICO Icon",
        "svg" => "SVG Vector Image",
        "tiff" | "tif" => "TIFF Image",
        // 文件
        "pdf" => "PDF Document",
        "doc" | "docx" => "Microsoft Word Document",
        "xls" | "xlsx" => "Microsoft Excel Spreadsheet",
        "ppt" | "pptx" => "Microsoft PowerPoint Presentation",
        "epub" => "EPUB eBook",
        // 壓縮封裝檔
        "zip" => "ZIP Archive",
        "tar" => "Tar Archive",
        "gz" | "tgz" => "Gzip Compressed Archive",
        "bz2" | "tbz2" => "Bzip2 Compressed Archive",
        "xz" | "txz" => "XZ Compressed Archive",
        "7z" => "7-Zip Archive",
        "rar" => "RAR Archive",
        "zst" => "Zstandard Compressed Archive",
        // 執行檔與函式庫
        "exe" => "Windows Executable",
        "dll" => "Windows Dynamic Link Library",
        "so" => "Shared Object Library",
        "dylib" => "macOS Dynamic Library",
        "bin" => "Binary Executable",
        "dmg" => "Apple Disk Image",
        "iso" => "ISO Disk Image",
        "wasm" => "WebAssembly Binary",
        // 影音
        "mp4" | "m4v" => "MP4 Video",
        "mkv" => "Matroska Video",
        "mov" => "QuickTime Video",
        "avi" => "AVI Video",
        "webm" => "WebM Video",
        "mp3" => "MP3 Audio",
        "flac" => "FLAC Audio",
        "wav" => "WAV Audio",
        "aac" => "AAC Audio",
        "ogg" => "Ogg Vorbis Audio",
        "m4a" => "M4A Audio",
        // 原始碼與設定檔
        "rs" => "Rust Source Code",
        "go" => "Go Source Code",
        "py" => "Python Source Code",
        "js" | "mjs" | "cjs" => "JavaScript Source Code",
        "ts" | "mts" | "cts" => "TypeScript Source Code",
        "jsx" => "React JSX Source Code",
        "tsx" => "React TSX Source Code",
        "c" | "h" => "C Source Code",
        "cpp" | "hpp" | "cc" | "cxx" => "C++ Source Code",
        "java" => "Java Source Code",
        "rb" => "Ruby Source Code",
        "php" => "PHP Script",
        "sh" | "bash" | "zsh" => "Shell Script",
        "ps1" | "psm1" | "psd1" => "PowerShell Script",
        "bat" | "cmd" => "Batch Script",
        "json" => "JSON Data",
        "toml" => "TOML Configuration",
        "yaml" | "yml" => "YAML Data",
        "xml" => "XML Document",
        "html" | "htm" => "HTML Document",
        "css" | "scss" | "sass" | "less" => "CSS Stylesheet",
        "md" | "markdown" => "Markdown Document",
        "txt" => "Plain Text Document",
        "log" => "Log File",
        "sql" => "SQL Script",
        "db" | "sqlite" | "sqlite3" => "SQLite Database",
        _ => "Binary or Data File",
    }
}

/// 為不能以純文字預覽的檔案產生結構化的「檔案詳細資訊卡片」。
pub(crate) fn format_file_details_preview(
    path: &Path,
    metadata: &fs::Metadata,
    notice: Option<&str>,
) -> Vec<Line<'static>> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase());

    let mut lines = vec![
        Line::from(format!("path: {}", path.display())),
        Line::from(format!(
            "type: {}",
            detect_file_kind(path, extension.as_deref())
        )),
        Line::from(format!("size: {}", format_size_with_bytes(metadata.len()))),
        Line::from(format!(
            "modified: {}",
            format_system_time(metadata.modified())
        )),
        Line::from(format!("permissions: {}", format_permissions(metadata))),
    ];

    if let Ok(created) = metadata.created() {
        lines.push(Line::from(format!(
            "created: {}",
            format_system_time(Ok(created))
        )));
    }

    if let Some(msg) = notice {
        lines.push(Line::from(format!("notice: {msg}")));
    }

    lines.push(Line::from(
        "hint: press Enter to open with default application",
    ));
    lines
}

/// 在預覽區將檔名靠左、大小靠右格式化，若寬度不足則維持最少 2 個空格。
pub(crate) fn format_preview_item_line(
    icon: &str,
    name: &str,
    size_str: Option<&str>,
    viewport_width: usize,
) -> Line<'static> {
    let prefix = format!("{icon} {name}");
    if let Some(size) = size_str {
        let prefix_width = unicode_width::UnicodeWidthStr::width(prefix.as_str());
        let size_width = size.len();
        let target_width = viewport_width.max(20);
        if target_width > prefix_width + size_width + 2 {
            let padding = target_width - prefix_width - size_width - 1;
            Line::from(format!("{prefix}{}{size}", " ".repeat(padding)))
        } else {
            Line::from(format!("{prefix}  {size}"))
        }
    } else {
        Line::from(prefix)
    }
}
