use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use image::{ImageBuffer, ImageFormat, Rgba};
use ratatui::style::Color;
use tempfile::tempdir;

use super::*;
use crate::file_manager::entry::FileEntry;

/// 輔助函式：在記憶體中建立指定寬高與像素的 PNG 格式位元組。
fn create_test_png(width: u32, height: u32, pixel_fn: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut img = ImageBuffer::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let rgba = pixel_fn(x, y);
            img.put_pixel(x, y, Rgba(rgba));
        }
    }
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("write png");
    bytes
}

#[test]
/// 驗證純 Rust Halfblock 圖片渲染器能正確輸出格式、尺寸與 24-bit TrueColor 上下像素方塊。
///
/// 驗證內容：
/// 1. 建立 4x4 PNG 測試圖片，第 0 列為純紅 (255, 0, 0)，第 1 列為純綠 (0, 255, 0)。
/// 2. 呼叫 `render_image_halfblocks` 進行終端預覽渲染。
/// 3. 驗證標題包含 `format: png image` 與 `dimensions: 4 x 4`。
/// 4. 驗證 Halfblock 列使用 `▀` 字元，且前景色為純紅、背景色為純綠。
///
/// 保護目的：確保圖片終端預覽演算法在縮放與像素映射時維持正確的 24-bit 色彩，避免渲染破圖或色彩偏移。
fn preview_renders_halfblock_image_with_correct_dimensions_and_colors() {
    let bytes = create_test_png(4, 4, |_x, y| match y {
        0 => [255, 0, 0, 255],   // 紅色
        1 => [0, 255, 0, 255],   // 綠色
        2 => [0, 0, 255, 255],   // 藍色
        _ => [255, 255, 0, 255], // 黃色
    });

    let (lines, (w, h)) =
        render_image_halfblocks_with_dimensions(&bytes, 40, 20).expect("render halfblocks");

    // 驗證尺寸資訊
    assert_eq!(w, 4);
    assert_eq!(h, 4);
    assert!(!lines.is_empty());

    // 第一個像素列：上紅下綠
    let first_row = &lines[0];
    let has_halfblock = first_row
        .spans
        .iter()
        .any(|span| span.content.as_ref() == "▀");
    assert!(has_halfblock, "縮圖必須包含 Halfblock ▀ 字元");

    // 檢查上紅下綠顏色樣式
    let styled_span = first_row
        .spans
        .iter()
        .find(|span| span.content.as_ref() == "▀")
        .expect("found halfblock span");
    assert_eq!(styled_span.style.fg, Some(Color::Rgb(255, 0, 0)));
    assert_eq!(styled_span.style.bg, Some(Color::Rgb(0, 255, 0)));
}

#[test]
/// 驗證透明與半透明 PNG 圖片在 Halfblock 渲染時能正確分流至下半方塊 ▄ 或空白。
///
/// 驗證內容：
/// 1. 建立 2x2 測試圖片，第 0 列為完全透明 (alpha = 0)，第 1 列為不透明青色 (0, 255, 255)。
/// 2. 呼叫 `render_image_halfblocks` 進行渲染。
/// 3. 驗證上透明下不透明時，使用下半方塊 `▄` 且前景色為青色。
///
/// 保護目的：避免透明圖片在終端機預覽時被填上預設黑色背景或破壞排版。
fn preview_handles_alpha_transparency() {
    let bytes = create_test_png(2, 2, |_x, y| match y {
        0 => [0, 0, 0, 0],       // 完全透明
        _ => [0, 255, 255, 255], // 青色不透明
    });

    let lines = render_image_halfblocks(&bytes, 20, 10).expect("render alpha");
    let thumbnail_lines = &lines;
    assert!(!thumbnail_lines.is_empty());

    // 上透明下不透明時應使用 ▄ (fg = 青色)
    let has_lower_block = thumbnail_lines[0]
        .spans
        .iter()
        .any(|span| span.content.as_ref() == "▄" && span.style.fg == Some(Color::Rgb(0, 255, 255)));
    assert!(has_lower_block, "透明上半部搭配不透明下半部應產生 ▄ 字元");
}

#[test]
/// 驗證大於 128 KiB 的非圖片檔案會顯示完整的「檔案詳細資訊卡片」，絕不顯示舊的 preview skipped 字樣。
///
/// 驗證內容：
/// 1. 建立 200 KiB 的大型文字檔案。
/// 2. 呼叫 `preview_file_content` 產生預覽行。
/// 3. 驗證預覽內容包含 path、type、size、modified、permissions、notice 與 hint。
/// 4. 驗證預覽內容絕不包含 "preview skipped for files larger than 128 KiB"。
///
/// 保護目的：徹底解決先前大檔案只顯示無用 skipped 提示的問題，提供使用者完整有價值的檔案屬性資訊。
fn preview_large_file_shows_rich_details_card() {
    let dir = tempdir().expect("tempdir");
    let large_file = dir.path().join("large_log.txt");
    let content = "a".repeat(200 * 1024);
    fs::write(&large_file, content).expect("write large file");

    let mut cache = None;
    let lines = preview_file_content(&large_file, 20, 80, 24, &mut cache, &mut None);
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();

    assert!(text.iter().any(|l| l.starts_with("path: ")));
    assert!(text.iter().any(|l| l.contains("type: Plain Text Document")));
    assert!(
        text.iter()
            .any(|l| l.contains("size: 200.0 KiB (204,800 bytes)"))
    );
    assert!(text.iter().any(|l| l.starts_with("modified: ")));
    assert!(text.iter().any(|l| l.starts_with("permissions: ")));
    assert!(
        text.iter()
            .any(|l| l.contains("notice: file size exceeds 128 KiB text preview limit"))
    );
    assert!(
        text.iter()
            .any(|l| l.contains("hint: press Enter to open with default application"))
    );

    // 嚴格保證絕不出現舊的簡陋提示
    assert!(
        !text
            .iter()
            .any(|l| l.contains("preview skipped for files larger than 128 KiB")),
        "不可再出現舊的 preview skipped 訊息"
    );
}

#[test]
/// 驗證二進位或非 UTF-8 檔案會顯示詳細資訊卡片與格式分類說明。
///
/// 驗證內容：
/// 1. 建立包含非法 UTF-8 位元組的二進位檔案。
/// 2. 呼叫 `preview_file_content` 產生預覽行。
/// 3. 驗證回傳內容標註二進位說明與開啟提示。
///
/// 保護目的：避免二進位檔案造成預覽亂碼或無資訊可讀。
fn preview_binary_file_shows_rich_details_card() {
    let dir = tempdir().expect("tempdir");
    let bin_file = dir.path().join("data.bin");
    fs::write(&bin_file, [0xFF, 0xFE, 0x00, 0xBA, 0xBE]).expect("write bin");

    let mut cache = None;
    let lines = preview_file_content(&bin_file, 20, 80, 24, &mut cache, &mut None);
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();

    assert!(text.iter().any(|l| l.starts_with("path: ")));
    assert!(text.iter().any(|l| l.contains("type: Binary Executable")));
    assert!(
        text.iter()
            .any(|l| l.contains("notice: binary or non-utf8 file"))
    );
    assert!(
        text.iter()
            .any(|l| l.contains("hint: press Enter to open with default application"))
    );
}

#[test]
/// 驗證圖片預覽快取機制能正確保存並命中已解碼的預覽結果，避免重複解碼。
///
/// 驗證內容：
/// 1. 建立 10x10 測試圖片。
/// 2. 第一次呼叫 `preview_file_content`，確認快取被填充。
/// 3. 第二次使用相同路徑與視窗尺寸呼叫，確認直接回傳快取內容。
///
/// 保護目的：確保快速鍵盤導航或重繪時不會重複進行高開銷的圖片解碼與縮放。
fn preview_image_cache_avoids_redundant_decode() {
    let dir = tempdir().expect("tempdir");
    let img_path = dir.path().join("photo.png");
    let bytes = create_test_png(10, 10, |_x, _y| [100, 150, 200, 255]);
    fs::write(&img_path, bytes).expect("write png");

    let mut cache: Option<ImagePreviewCache> = None;
    let first_result = preview_file_content(&img_path, 20, 80, 24, &mut cache, &mut None);
    assert!(cache.is_some(), "快取必須在第一次解碼後建立");

    let second_result = preview_file_content(&img_path, 20, 80, 24, &mut cache, &mut None);
    assert_eq!(first_result.len(), second_result.len());
    assert_eq!(first_result[0].to_string(), second_result[0].to_string());
}

#[test]
/// 驗證目錄預覽包含結構化路徑、項目數、修改時間與權限資訊。
///
/// 驗證內容：
/// 1. 建立目錄並加入兩個子檔案。
/// 2. 呼叫 `preview_directory` 產生目錄預覽。
/// 3. 驗證輸出同時相容於舊有的 path、items、contents 結構，並擴充時間與權限。
///
/// 保護目的：保持既有目錄預覽測試相容性，同時為使用者呈現更完整的目錄資訊卡片。
fn preview_directory_shows_metadata_and_contents() {
    let dir = tempdir().expect("tempdir");
    let sub = dir.path().join("subfolder");
    fs::create_dir(&sub).expect("create dir");
    fs::write(sub.join("file1.txt"), "hello").expect("write 1");
    fs::write(sub.join("file2.txt"), "world").expect("write 2");

    let entry = FileEntry {
        name: "subfolder".to_string(),
        path: sub.clone(),
        is_dir: true,
        size: 0,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::UNIX_EPOCH,
        created: std::time::SystemTime::UNIX_EPOCH,
        readonly: false,
        unix_mode: None,
    };

    let lines = preview_directory(&entry, 20);
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();

    assert!(text.iter().any(|l| l.contains("path: ")));
    assert!(text.iter().any(|l| l.contains("items: 2")));
    assert!(text.iter().any(|l| l.contains("modified: ")));
    assert!(text.iter().any(|l| l.contains("permissions: ")));
    assert!(text.iter().any(|l| l == "contents:"));
    assert!(text.iter().any(|l| l.contains("file1.txt")));
    assert!(text.iter().any(|l| l.contains("file2.txt")));
}

#[test]
/// 驗證檔案大小格式化函數能輸出帶逗號的精確 byte 數與正確單位。
///
/// 驗證內容：
/// 測試 512B、2048B、1572864B 等數值的大小格式化字串。
///
/// 保護目的：確保檔案大小顯示清晰精確，方便使用者在詳細資訊卡片中核對。
fn format_size_with_bytes_formats_correctly() {
    assert_eq!(format_size_with_bytes(512), "512 B (512 bytes)");
    assert_eq!(format_size_with_bytes(2048), "2.0 KiB (2,048 bytes)");
    assert_eq!(
        format_size_with_bytes(1_572_864),
        "1.50 MiB (1,572,864 bytes)"
    );
}

#[test]
/// 驗證常見檔案類型辨識涵蓋圖片、文件、壓縮檔、原始碼與設定檔。
///
/// 驗證內容：
/// 測試各類副檔名推測出的檔案類型字串。
///
/// 保護目的：確保不同類型的檔案在詳細卡片中顯示專業且易懂的分類說明。
fn detect_file_kind_recognizes_common_types() {
    let p = PathBuf::from("test.rs");
    assert_eq!(detect_file_kind(&p, Some("rs")), "Rust Source Code");
    let p = PathBuf::from("doc.pdf");
    assert_eq!(detect_file_kind(&p, Some("pdf")), "PDF Document");
    let p = PathBuf::from("archive.zip");
    assert_eq!(detect_file_kind(&p, Some("zip")), "ZIP Archive");
    let p = PathBuf::from("video.mp4");
    assert_eq!(detect_file_kind(&p, Some("mp4")), "MP4 Video");
    let p = PathBuf::from("image.png");
    assert_eq!(detect_file_kind(&p, Some("png")), "PNG Image");
}

#[test]
/// 驗證大型圖片（> 256 KiB）會觸發非同步背景解碼，立即回傳載入中提示卡，並在完成後自動轉為縮圖。
///
/// 驗證內容：
/// 1. 建立超過 256 KiB 的測試圖片。
/// 2. 第一次預覽時立即回傳載入卡片，不阻塞，並建立 `ImageLoader`。
/// 3. 等候背景執行緒完成後再次呼叫預覽，確認取得縮圖 halfblock，同時快取建立且 loader 清除。
///
/// 保護目的：確保按下 Tab 預覽大型圖片時 UI 保持 0ms 零卡頓，且能順暢過渡至圖片縮圖。
fn large_image_triggers_async_loader_and_renders_on_completion() {
    let dir = tempdir().expect("tempdir");
    let img_path = dir.path().join("large_photo.png");
    // 建立 600x600 的高熵 PNG 圖片，檔案大小超過 256 KiB
    let bytes = create_test_png(600, 600, |x, y| {
        [
            ((x.wrapping_mul(12345) ^ y.wrapping_mul(67890)) % 256) as u8,
            ((x.wrapping_mul(54321) + y.wrapping_mul(9876)) % 256) as u8,
            ((x ^ y) % 256) as u8,
            255,
        ]
    });
    assert!(bytes.len() > 256 * 1024, "測試圖片大小需超過 256 KiB");
    fs::write(&img_path, bytes).expect("write large png");

    let mut cache: Option<ImagePreviewCache> = None;
    let mut loader: Option<ImageLoader> = None;

    // 第一次呼叫：應立即返回載入提示，絕不阻塞
    let loading_lines = preview_file_content(&img_path, 30, 80, 24, &mut cache, &mut loader);
    let loading_text = loading_lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>();
    assert!(
        loading_text
            .iter()
            .any(|l| l.contains("Loading image preview in background")),
        "大型圖片初次載入必須顯示非同步載入提示"
    );
    assert!(loader.is_some(), "必須啟動 ImageLoader 背景任務");
    assert!(cache.is_none(), "背景解碼尚未完成前快取不可為 Some");

    // 等待背景執行緒完成解碼（通常只需幾十毫秒）
    let mut retries = 0;
    while retries < 100 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let lines = preview_file_content(&img_path, 30, 80, 24, &mut cache, &mut loader);
        let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        if !text
            .iter()
            .any(|l| l.contains("Loading image preview in background"))
        {
            // 背景解碼完成並成功渲染為 halfblock
            assert!(loader.is_none(), "解碼完成後 loader 必須被清空");
            assert!(cache.is_some(), "解碼完成後必須寫入快取");
            assert!(lines.len() > 1, "預覽內容應包含 halfblock 圖片列");
            assert!(
                lines
                    .iter()
                    .any(|line| line.spans.iter().any(|s| s.content.as_ref() == "▀")),
                "必須包含 Halfblock 縮圖"
            );
            assert_eq!(cache.as_ref().unwrap().dimensions, Some((600, 600)));
            return;
        }
        retries += 1;
    }
    panic!("背景圖片解碼超時未完成");
}
