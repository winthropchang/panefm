//! 純 Rust 原生 Halfblock 圖片預覽、24-bit TrueColor ANSI 渲染與圖片尺寸讀取。

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use image::GenericImageView;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::card::{format_size_compact, format_system_time_short};

/// 判斷副檔名是否為支援的圖片格式。
pub fn is_image_extension(ext: Option<&str>) -> bool {
    matches!(
        ext.unwrap_or_default().to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico"
    )
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

/// 針對部分測試中只寫入檔頭 bytes 的 mock 檔案提供降級解析。
pub(crate) fn detect_legacy_image_summary(
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
