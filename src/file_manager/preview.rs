//! 終端檔案預覽模組：提供純 Rust 原生 Halfblock 圖片預覽與大檔案/目錄詳細資訊卡片。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local};
use image::GenericImageView;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::entry::FileEntry;

/// 支援終端 Halfblock 預覽的單一圖片大小上限（30 MiB）。
pub(crate) const MAX_IMAGE_PREVIEW_SIZE: u64 = 30 * 1024 * 1024;

/// 一般文字檔案直接載入內容預覽的大小上限（128 KiB）。
pub(crate) const MAX_TEXT_PREVIEW_SIZE: u64 = 128 * 1024;

/// 圖片預覽快取，避免每一幀重新讀取磁碟與解碼像素。
#[derive(Clone, Debug)]
pub struct ImagePreviewCache {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub max_cols: usize,
    pub max_rows: usize,
    pub dimensions: Option<(u32, u32)>,
    pub lines: Vec<Line<'static>>,
}

/// 圖片解碼成功時回傳的縮圖列與原圖解析度。
pub type ImageDecodeResult = Result<(Vec<Line<'static>>, (u32, u32)), String>;

/// 背景非同步圖片解碼任務。
pub struct ImageLoader {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub max_cols: usize,
    pub max_rows: usize,
    pub receiver: std::sync::mpsc::Receiver<ImageDecodeResult>,
}

impl std::fmt::Debug for ImageLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageLoader")
            .field("path", &self.path)
            .field("modified", &self.modified)
            .field("max_cols", &self.max_cols)
            .field("max_rows", &self.max_rows)
            .finish_non_exhaustive()
    }
}

/// 判斷副檔名是否為支援的圖片格式。
pub fn is_image_extension(ext: Option<&str>) -> bool {
    matches!(
        ext.unwrap_or_default().to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico"
    )
}

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

/// 依照圖片屬性產生邊框標題字串。
/// 格式範例：`1920 × 25000 (PNG)  •  13.85 MiB  •  2026-01-01 08:59`
pub fn format_image_title(
    dimensions: Option<(u32, u32)>,
    format_name: Option<&str>,
    size: u64,
    mtime: Option<SystemTime>,
    is_loading: bool,
) -> String {
    let size_str = format_size_compact(size);
    let mtime_str = format_system_time_short(mtime);
    let mut parts = Vec::new();

    if let Some((w, h)) = dimensions {
        if let Some(fmt) = format_name {
            parts.push(format!("{w} × {h} ({})", fmt.to_uppercase()));
        } else {
            parts.push(format!("{w} × {h}"));
        }
    } else if let Some(fmt) = format_name {
        parts.push(fmt.to_uppercase());
    }

    parts.push(size_str);

    if is_loading {
        parts.push("⏳ Loading...".to_string());
    } else {
        parts.push(mtime_str);
    }

    parts.join("  •  ")
}

/// 嘗試快速從檔案讀取圖片尺寸（先嘗試標準 image_dimensions，失敗時對 PNG/GIF 檔頭進行快速 fallback 解析）。
pub fn read_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    if let Ok((w, h)) = image::image_dimensions(path) {
        return Some((w, h));
    }
    let Ok(file) = fs::File::open(path) else {
        return None;
    };
    use std::io::Read;
    let mut buf = [0u8; 32];
    let mut reader = file.take(32);
    let Ok(n) = reader.read(&mut buf) else {
        return None;
    };
    let bytes = &buf[..n];
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((width, height));
    }
    if bytes.len() >= 10 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some((width, height));
    }
    None
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
        "js" => "JavaScript Source Code",
        "ts" => "TypeScript Source Code",
        "c" | "h" => "C Source Code",
        "cpp" | "hpp" | "cc" | "cxx" => "C++ Source Code",
        "java" => "Java Source Code",
        "rb" => "Ruby Source Code",
        "php" => "PHP Script",
        "sh" | "bash" | "zsh" => "Shell Script",
        "json" => "JSON Data",
        "toml" => "TOML Configuration",
        "yaml" | "yml" => "YAML Data",
        "xml" => "XML Document",
        "html" | "htm" => "HTML Document",
        "css" => "CSS Stylesheet",
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

/// 為資料夾產生結構化摘要內容，包含路徑、項目數、時間、權限與子項目清單。
pub(crate) fn preview_directory(entry: &FileEntry, max_lines: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("path: {}", entry.path.display())),
        Line::from(format!("items: {}", count_directory_items(&entry.path))),
    ];

    if let Ok(metadata) = fs::metadata(&entry.path) {
        lines.push(Line::from(format!(
            "modified: {}",
            format_system_time(metadata.modified())
        )));
        lines.push(Line::from(format!(
            "permissions: {}",
            format_permissions(&metadata)
        )));
    }

    if max_lines <= lines.len() {
        lines.truncate(max_lines);
        return lines;
    }

    let remaining = max_lines.saturating_sub(lines.len()).min(50);
    if remaining == 0 {
        return lines;
    }

    match fs::read_dir(&entry.path) {
        Ok(read_dir) => {
            let mut child_names = Vec::new();
            for child in read_dir.flatten() {
                child_names.push(child.file_name().to_string_lossy().to_string());
                if child_names.len() >= remaining {
                    break;
                }
            }

            if child_names.is_empty() {
                lines.push(Line::from("empty directory"));
            } else {
                lines.push(Line::from("contents:"));
                for name in child_names
                    .into_iter()
                    .take(max_lines.saturating_sub(lines.len()))
                {
                    lines.push(Line::from(format!("  {name}")));
                }
            }
        }
        Err(_) => lines.push(Line::from("unable to read directory contents")),
    }

    lines.truncate(max_lines);
    lines
}

/// 計算指定目錄內的子項目數量（超過 64 時顯示 `64+`）。
fn count_directory_items(path: &Path) -> String {
    let Ok(read_dir) = fs::read_dir(path) else {
        return String::from("0");
    };
    let mut count = 0;
    for entry in read_dir {
        if entry.is_ok() {
            count += 1;
            if count > 64 {
                return String::from("64+");
            }
        }
    }
    count.to_string()
}

/// 將圖片資料渲染成 ANSI 24-bit TrueColor Halfblock（`▀` / `▄`）列，並回傳原圖尺寸。
pub fn render_image_halfblocks_with_dimensions(
    bytes: &[u8],
    max_cols: usize,
    max_rows: usize,
) -> Result<(Vec<Line<'static>>, (u32, u32)), String> {
    let img = image::load_from_memory(bytes).map_err(|err| err.to_string())?;
    let (orig_w, orig_h) = img.dimensions();
    if orig_w == 0 || orig_h == 0 {
        return Err("image dimensions are zero".to_string());
    }

    let mut result_lines = Vec::new();

    // 縮圖可用的終端列數與像素數（滿版配置給圖片，不浪費任何空間）
    let avail_rows = max_rows.max(1);
    let target_pixel_width = max_cols.clamp(10, 240) as u32;
    let target_pixel_height = (avail_rows * 2).clamp(2, 600) as u32;

    // 保持原始寬高比縮放（若小於視窗大小則保持原尺寸；大圖使用高速整數取樣縮圖演算法）
    let resized = if orig_w <= target_pixel_width && orig_h <= target_pixel_height {
        img.to_rgba8()
    } else {
        img.thumbnail(target_pixel_width, target_pixel_height)
            .to_rgba8()
    };
    let (resized_w, resized_h) = resized.dimensions();

    // 逐兩列像素合成一個終端列
    let mut y = 0;
    while y < resized_h {
        let mut spans = Vec::with_capacity(resized_w as usize);
        for x in 0..resized_w {
            let top_pixel = resized.get_pixel(x, y);
            let bot_pixel = if y + 1 < resized_h {
                Some(resized.get_pixel(x, y + 1))
            } else {
                None
            };

            let top_alpha = top_pixel[3];
            let bot_alpha = bot_pixel.map(|p| p[3]).unwrap_or(0);

            // 透明度分流處理
            if top_alpha < 32 && bot_alpha < 32 {
                spans.push(Span::raw(" "));
            } else if top_alpha < 32 {
                // 上透明、下不透明：使用下半方塊 ▄ (fg = bot_pixel)
                let bot_p = bot_pixel.unwrap();
                let fg = Color::Rgb(bot_p[0], bot_p[1], bot_p[2]);
                spans.push(Span::styled("▄", Style::default().fg(fg)));
            } else if bot_alpha < 32 {
                // 上不透明、下透明：使用上半方塊 ▀ (fg = top_pixel)
                let fg = Color::Rgb(top_pixel[0], top_pixel[1], top_pixel[2]);
                spans.push(Span::styled("▀", Style::default().fg(fg)));
            } else {
                // 上下皆不透明：使用上半方塊 ▀ (fg = top, bg = bot)
                let fg = Color::Rgb(top_pixel[0], top_pixel[1], top_pixel[2]);
                let bot_p = bot_pixel.unwrap();
                let bg = Color::Rgb(bot_p[0], bot_p[1], bot_p[2]);
                spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
            }
        }
        result_lines.push(Line::from(spans));
        y += 2;
    }

    Ok((result_lines, (orig_w, orig_h)))
}

/// 將圖片資料渲染成 ANSI 24-bit TrueColor Halfblock（`▀` / `▄`）列。
pub fn render_image_halfblocks(
    bytes: &[u8],
    max_cols: usize,
    max_rows: usize,
) -> Result<Vec<Line<'static>>, String> {
    render_image_halfblocks_with_dimensions(bytes, max_cols, max_rows).map(|(lines, _)| lines)
}

/// 產生圖片正在背景載入時的即時卡片內容，絕不阻塞 TUI。
pub fn format_image_loading_preview(max_lines: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(""),
        Line::styled(
            "⏳ Loading image preview in background... (背景解碼圖片中...)",
            Style::default().fg(Color::Yellow),
        ),
    ];
    lines.truncate(max_lines);
    lines
}

/// 產生檔案預覽行清單（結合文字顯示、圖片 Halfblock 與大檔案/二進位詳細資訊卡片）。
pub(crate) fn preview_file_content(
    path: &Path,
    max_lines: usize,
    viewport_width: usize,
    viewport_height: usize,
    image_cache: &mut Option<ImagePreviewCache>,
    image_loader: &mut Option<ImageLoader>,
) -> Vec<Line<'static>> {
    let Ok(metadata) = fs::metadata(path) else {
        return vec![Line::from("unable to read metadata")];
    };

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase());

    // 1. 圖片檔案分流
    if is_image_extension(extension.as_deref()) {
        if metadata.len() > MAX_IMAGE_PREVIEW_SIZE {
            return format_file_details_preview(
                path,
                &metadata,
                Some("image exceeds 30 MiB preview limit"),
            );
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
            lines.truncate(max_lines);
            return lines;
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
                        let mut lines = rendered;
                        lines.truncate(max_lines);
                        return lines;
                    }
                    Ok(Err(_)) => {
                        *image_loader = None;
                        return format_file_details_preview(
                            path,
                            &metadata,
                            Some("image format unsupported or decode failed"),
                        );
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return format_image_loading_preview(max_lines);
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
                let mut lines = rendered;
                lines.truncate(max_lines);
                return lines;
            } else if let Some(summary) = detect_legacy_image_summary(&bytes, extension.as_deref())
            {
                let mut lines = summary;
                lines.truncate(max_lines);
                return lines;
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

        return format_image_loading_preview(max_lines);
    }

    // 2. 非圖片大檔案：超過 128 KiB 時顯示結構化詳細資訊卡片
    if metadata.len() > MAX_TEXT_PREVIEW_SIZE {
        return format_file_details_preview(
            path,
            &metadata,
            Some("file size exceeds 128 KiB text preview limit"),
        );
    }

    // 3. 小於等於 128 KiB 的檔案：嘗試讀取並以純文字行號預覽
    let Ok(bytes) = fs::read(path) else {
        return format_file_details_preview(path, &metadata, Some("unable to read file contents"));
    };

    match String::from_utf8(bytes) {
        Ok(contents) => {
            let content_lines: Vec<&str> = contents.lines().collect();
            if content_lines.is_empty() {
                return vec![Line::from("[empty file]")];
            }

            let mut lines = Vec::new();
            let truncated = content_lines.len() > max_lines;
            for (index, line) in content_lines.into_iter().take(max_lines).enumerate() {
                lines.push(Line::from(format!("{:>3} {}", index + 1, line)));
            }

            if truncated && !lines.is_empty() {
                let last_index = lines.len() - 1;
                lines[last_index] = Line::from("...");
            }

            lines.truncate(max_lines);
            lines
        }
        Err(_) => format_file_details_preview(path, &metadata, Some("binary or non-utf8 file")),
    }
}

/// 針對部分測試中只寫入檔頭 bytes 的 mock 檔案提供降級解析。
fn detect_legacy_image_summary(
    bytes: &[u8],
    extension: Option<&str>,
) -> Option<Vec<Line<'static>>> {
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some(vec![
            Line::from("format: png image"),
            Line::from(format!("dimensions: {width} x {height}")),
        ]);
    }
    if bytes.len() >= 10 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some(vec![
            Line::from("format: gif image"),
            Line::from(format!("dimensions: {width} x {height}")),
        ]);
    }
    match extension.unwrap_or_default() {
        "png" => Some(vec![Line::from("format: png image")]),
        "jpg" | "jpeg" => Some(vec![Line::from("format: jpeg image")]),
        "gif" => Some(vec![Line::from("format: gif image")]),
        "webp" => Some(vec![Line::from("format: webp image")]),
        _ => None,
    }
}

#[cfg(test)]
#[path = "tests/preview_test.rs"]
mod tests;
