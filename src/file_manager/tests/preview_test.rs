use std::fs;
use std::io::{Cursor, Write};
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
/// 驗證大於 2 MiB 的非圖片檔案會顯示完整的「檔案詳細資訊卡片」，絕不顯示舊的 preview skipped 字樣。
///
/// 驗證內容：
/// 1. 建立超過 2 MiB 的大型文字檔案。
/// 2. 呼叫 `preview_file_content` 產生預覽行。
/// 3. 驗證預覽內容包含 path、type、size、modified、permissions、notice 與 hint。
/// 4. 驗證預覽內容絕不包含 "preview skipped"。
///
/// 保護目的：徹底解決先前大檔案只顯示無用 skipped 提示的問題，提供使用者完整有價值的檔案屬性資訊。
fn preview_large_file_shows_rich_details_card() {
    let dir = tempdir().expect("tempdir");
    let large_file = dir.path().join("large_log.txt");
    let content = "a".repeat(2 * 1024 * 1024 + 1024);
    fs::write(&large_file, content).expect("write large file");

    let mut cache = None;
    let lines = preview_file_content(&large_file, 20, 80, 24, &mut cache, &mut None);
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();

    assert!(text.iter().any(|l| l.starts_with("path: ")));
    assert!(text.iter().any(|l| l.contains("type: Plain Text Document")));
    assert!(text.iter().any(|l| l.starts_with("size: 2.00 MiB")));
    assert!(text.iter().any(|l| l.starts_with("modified: ")));
    assert!(text.iter().any(|l| l.starts_with("permissions: ")));
    assert!(
        text.iter()
            .any(|l| l.contains("notice: file size exceeds 2 MiB text preview limit"))
    );
    assert!(
        text.iter()
            .any(|l| l.contains("hint: press Enter to open with default application"))
    );

    // 嚴格保證絕不出現舊的簡陋提示
    assert!(
        !text.iter().any(|l| l.contains("preview skipped")),
        "不可再出現舊的 preview skipped 訊息"
    );
}

#[test]
/// 驗證超過先前 128 KiB 限制但小於 2 MiB 的程式碼檔案（如 200 KiB 原始碼）可正常呈現代碼預覽，不再被卡片阻擋。
/// 保護目的：解決典型專案大原始碼（如 4000 行之 pane.rs）無法預覽的問題。
fn preview_file_between_128k_and_2m_renders_code_preview() {
    let dir = tempdir().expect("tempdir");
    let large_code_file = dir.path().join("large_module.rs");
    let mut code = String::new();
    // 建立約 200 KiB 的 Rust 程式碼（約 5000 行）
    for i in 1..=5000 {
        code.push_str(&format!("fn helper_{i}() -> usize {{ {i} }}\n"));
    }
    assert!(code.len() > 128 * 1024, "測試檔案必須超過舊的 128 KiB 上限");
    assert!(code.len() < 2 * 1024 * 1024, "測試檔案必須在 2 MiB 限制內");
    fs::write(&large_code_file, &code).expect("write large code file");

    let mut cache = None;
    let lines = preview_file_content(&large_code_file, 20, 80, 24, &mut cache, &mut None);
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();

    // 驗證輸出的是程式碼帶行號，而非詳細屬性卡片
    assert!(
        !text
            .iter()
            .any(|l| l.starts_with("notice: file size exceeds"))
    );
    assert!(text.iter().any(|l| l.contains("fn helper_1()")));
    assert_eq!(lines.len(), 20);
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
/// 1. 建立目錄並加入子資料夾與子檔案。
/// 2. 呼叫 `preview_directory` 產生精進版「目錄偷窺」預覽（方案 A）。
/// 3. 驗證輸出 100% 滿版陳列子項目，資料夾帶 ，檔案帶  等特殊字元圖示與右側容量。
///
/// 保護目的：確保目錄偷窺預覽符合方案 A 之極簡直覺設計，不浪費畫面空間。
fn preview_directory_shows_metadata_and_contents() {
    let dir = tempdir().expect("tempdir");
    let sub = dir.path().join("subfolder");
    fs::create_dir(&sub).expect("create dir");
    fs::create_dir(sub.join("child_folder")).expect("create child dir");
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

    let (dirs, files) = quick_directory_counts(&sub);
    assert_eq!(dirs, 1);
    assert_eq!(files, 2);

    let title =
        format_directory_title("subfolder", dirs + files, dirs, files, Some(entry.modified));
    assert!(title.contains("subfolder"));
    assert!(title.contains("3 items (1 dirs, 2 files)"));

    let lines = preview_directory(&entry, 20, 60);
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();

    assert!(text.iter().any(|l| l.contains(" child_folder/")));
    assert!(text.iter().any(|l| l.contains(" file1.txt")));
    assert!(text.iter().any(|l| l.contains(" file2.txt")));
}

#[test]
/// 驗證空目錄能正確顯示 `[empty directory]` 提示標籤。
/// 保護目的：避免空目錄預覽時呈現空白無回應或拋出錯誤。
fn preview_directory_empty_shows_empty_tag() {
    let dir = tempdir().expect("tempdir");
    let empty_sub = dir.path().join("empty_folder");
    fs::create_dir(&empty_sub).expect("create empty dir");

    let entry = FileEntry {
        name: "empty_folder".to_string(),
        path: empty_sub,
        is_dir: true,
        size: 0,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::UNIX_EPOCH,
        created: std::time::SystemTime::UNIX_EPOCH,
        readonly: false,
        unix_mode: None,
    };

    let lines = preview_directory(&entry, 20, 60);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].to_string(), "[empty directory]");
}

#[test]
/// 驗證 ZIP 壓縮檔能免解壓直接解析內部目錄結構、檔案名稱與未壓縮大小。
/// 保護目的：保障壓縮檔內部預覽能高速讀取 central directory，不浪費磁碟空間解壓縮。
fn preview_zip_archive_shows_internal_entries() {
    let dir = tempdir().expect("tempdir");
    let zip_path = dir.path().join("test.zip");

    {
        let file = fs::File::create(&zip_path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.add_directory("nested/", options).expect("zip dir");
        zip.start_file("nested/hello.rs", options)
            .expect("zip file 1");
        zip.write_all(b"fn main() {}").expect("write zip 1");
        zip.start_file("readme.md", options).expect("zip file 2");
        zip.write_all(b"# Test Archive").expect("write zip 2");
        zip.finish().expect("finish zip");
    }

    assert_eq!(is_archive_file(&zip_path), Some("zip"));
    let (count, uncompressed) =
        quick_archive_counts(&zip_path, "zip").expect("quick archive counts");
    assert_eq!(count, 3);
    assert!(uncompressed > 0);

    let title = format_archive_title("test.zip", count, uncompressed);
    assert!(title.contains("test.zip"));
    assert!(title.contains("3 entries"));

    let lines = preview_archive_content(&zip_path, "zip", 20, 60).expect("preview archive");
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    assert!(text.iter().any(|l| l.contains(" nested/")));
    assert!(text.iter().any(|l| l.contains(" nested/hello.rs")));
    assert!(text.iter().any(|l| l.contains(" readme.md")));
}

#[test]
/// 驗證 TAR.GZ 壓縮檔能透過 Gzip 串流解析 tar headers 並預覽內部檔案。
/// 保護目的：確保 tar.gz 與 tgz 檔案格式正確支援免解壓檢視。
fn preview_tar_gz_archive_shows_internal_entries() {
    let dir = tempdir().expect("tempdir");
    let tar_gz_path = dir.path().join("archive.tar.gz");

    {
        let file = fs::File::create(&tar_gz_path).expect("create tar.gz");
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);

        let mut header = tar::Header::new_gnu();
        header.set_size(12);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "inner.txt", b"hello world\n".as_slice())
            .expect("append tar");
        tar.finish().expect("finish tar");
    }

    assert_eq!(is_archive_file(&tar_gz_path), Some("tar.gz"));
    let (count, uncompressed) = quick_archive_counts(&tar_gz_path, "tar.gz").expect("counts");
    assert_eq!(count, 1);
    assert_eq!(uncompressed, 12);

    let lines = preview_archive_content(&tar_gz_path, "tar.gz", 20, 60).expect("preview tar.gz");
    let text = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
    assert!(text.iter().any(|l| l.contains(" inner.txt")));
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

#[test]
/// 驗證 Rust 原始碼在代碼預覽時能正確產生語法高亮 Span 與暗色行號。
/// 保護目的：確保程式碼預覽告別單色黑白文字，呈現層次豐富的關鍵字、字串與行號色彩。
fn preview_code_renders_syntax_highlighted_spans_for_rust() {
    let dir = tempdir().expect("tempdir");
    let rust_file = dir.path().join("demo.rs");
    let code = "fn main() {\n    let greeting = \"hello world\";\n}\n";
    fs::write(&rust_file, code).expect("write rust file");

    let lines = highlight_code_preview(&rust_file, code, 10, None);
    assert_eq!(lines.len(), 3);

    // 驗證第 1 行包含行號與語法著色 Span
    let line1 = &lines[0];
    assert!(line1.spans.len() >= 2, "第 1 行應至少包含行號與關鍵字 Span");

    // 行號 Span 驗證：前景色應為暗灰 (110, 115, 128)
    let line_num_span = &line1.spans[0];
    assert_eq!(line_num_span.content.as_ref(), "  1 ");
    assert_eq!(
        line_num_span.style.fg,
        Some(Color::Rgb(110, 115, 128)),
        "行號必須使用低飽和度暗灰色"
    );

    // 關鍵字 "fn" 應該被著色（前景色為 RGB 色彩，非預設空樣式）
    let has_colored_kw = line1
        .spans
        .iter()
        .any(|span| span.content.as_ref().contains("fn") && span.style.fg.is_some());
    assert!(has_colored_kw, "Rust 關鍵字 fn 必須帶有語法高亮色彩");

    // 驗證第 2 行中的字串字面量有高亮
    let line2 = &lines[1];
    let has_colored_str = line2
        .spans
        .iter()
        .any(|span| span.content.as_ref().contains("hello world") && span.style.fg.is_some());
    assert!(has_colored_str, "字串字面量必須帶有語法高亮色彩");

    // 驗證轉為純文字時維持正確格式
    assert_eq!(line1.to_string(), "  1 fn main() {");
    assert_eq!(line2.to_string(), "  2     let greeting = \"hello world\";");
}

#[test]
/// 驗證 JSON 檔案在代碼預覽時能正確識別語法並為鍵值與數值著色。
/// 保護目的：確保常見設定檔（JSON/TOML）具備良好的可讀性。
fn preview_code_renders_syntax_highlighted_spans_for_json() {
    let dir = tempdir().expect("tempdir");
    let json_file = dir.path().join("config.json");
    let json_content = "{\n  \"port\": 8080,\n  \"enabled\": true\n}\n";
    fs::write(&json_file, json_content).expect("write json file");

    let lines = highlight_code_preview(&json_file, json_content, 10, None);
    assert_eq!(lines.len(), 4);

    let line2 = &lines[1];
    let has_colored_num = line2
        .spans
        .iter()
        .any(|span| span.content.as_ref().contains("8080") && span.style.fg.is_some());
    assert!(has_colored_num, "JSON 數值 8080 必須具備高亮色彩");
}

#[test]
/// 驗證未支援的副檔名或純文字檔案能安全回退至暗色行號與原文字串，不發生崩潰。
/// 保護目的：確保語法高亮在面對任意未知檔案時具備 100% 穩定性。
fn preview_code_renders_plain_text_with_safe_fallback() {
    let dir = tempdir().expect("tempdir");
    let txt_file = dir.path().join("unknown.custom_ext_123");
    let content = "alpha\nbeta\n";
    fs::write(&txt_file, content).expect("write txt file");

    let lines = highlight_code_preview(&txt_file, content, 10, None);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].to_string(), "  1 alpha");
    assert_eq!(lines[1].to_string(), "  2 beta");
}

#[test]
/// 驗證 TOML 設定檔（如 Cargo.toml）在代碼預覽時能正確產生語法高亮 Span。
/// 保護目的：確保 TOML 標題、鍵名、字串、數值、布林值與註解皆有對應的高亮著色。
fn preview_code_renders_syntax_highlighted_spans_for_toml() {
    let dir = tempdir().expect("tempdir");
    let toml_file = dir.path().join("Cargo.toml");
    let toml_content =
        "[package]\nname = \"panefm\"\nversion = \"0.1.12\"\nenabled = true\n# comment\n";
    fs::write(&toml_file, toml_content).expect("write toml file");

    let lines = highlight_code_preview(&toml_file, toml_content, 10, None);
    assert_eq!(lines.len(), 5);

    // 第 1 行：[package] 區段標題高亮
    let line1 = &lines[0];
    let has_header = line1
        .spans
        .iter()
        .any(|s| s.content.as_ref() == "package" && s.style.fg.is_some());
    assert!(has_header, "TOML 區段標題 [package] 必須帶有色彩");

    // 第 2 行：name = "panefm" 鍵與字串高亮
    let line2 = &lines[1];
    let has_key = line2
        .spans
        .iter()
        .any(|s| s.content.as_ref() == "name" && s.style.fg.is_some());
    let has_str = line2
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("panefm") && s.style.fg.is_some());
    assert!(has_key, "TOML 鍵名必須高亮");
    assert!(has_str, "TOML 字串必須高亮");

    // 第 4 行：enabled = true 布林值高亮
    let line4 = &lines[3];
    let has_bool = line4
        .spans
        .iter()
        .any(|s| s.content.as_ref() == "true" && s.style.fg.is_some());
    assert!(has_bool, "TOML 布林值必須高亮");

    // 第 5 行：# comment 註解高亮
    let line5 = &lines[4];
    let has_comment = line5
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("# comment") && s.style.fg.is_some());
    assert!(has_comment, "TOML 註解必須高亮");
}

#[test]
/// 驗證單行 TOML 分詞高亮函式針對各種標記（鍵、等號、字串、數值、內嵌表、陣列）的解析表現。
fn test_highlight_toml_line_tokens() {
    // 陣列表標題
    let spans = highlight_toml_line("[[bin]]");
    assert_eq!(spans.len(), 3);
    assert_eq!(spans[0].content.as_ref(), "[[");
    assert_eq!(spans[1].content.as_ref(), "bin");
    assert_eq!(spans[2].content.as_ref(), "]]");

    // 純註解行
    let spans = highlight_toml_line("# a comment");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].content.as_ref(), "# a comment");

    // 內嵌結構行
    let spans = highlight_toml_line("dependencies = { serde = \"1.0\", ratatui = \"0.29\" }");
    let content: String = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(
        content,
        "dependencies = { serde = \"1.0\", ratatui = \"0.29\" }"
    );
    let has_serde_key = spans
        .iter()
        .any(|s| s.content.as_ref() == "serde" && s.style.fg.is_some());
    assert!(has_serde_key, "內嵌表鍵名 serde 應高亮");
}

#[test]
/// 驗證 highlight_code_preview_slice 能正確對指定區間渲染正確行號與語法色彩。
fn test_highlight_code_preview_slice() {
    let code = "fn alpha() {}\nfn beta() {}\nfn gamma() {}\nfn delta() {}\n";
    let lines = highlight_code_preview_slice(Path::new("test.rs"), code, 2, 2, 100, None);
    assert_eq!(lines.len(), 2);
    // 第 1 條輸出應該對應第 3 行 (gamma)
    let line1 = &lines[0];
    let line1_str = line1.to_string();
    assert!(
        line1_str.contains("  3 fn gamma()"),
        "第 3 行應正確顯示行號與函式內容: {line1_str}"
    );
    let has_fn_kw = line1
        .spans
        .iter()
        .any(|s| s.content.as_ref().contains("fn") && s.style.fg.is_some());
    assert!(has_fn_kw, "切片渲染中關鍵字 fn 應有色彩");

    // 第 2 條輸出應該對應第 4 行 (delta)
    let line2 = &lines[1];
    let line2_str = line2.to_string();
    assert!(
        line2_str.contains("  4 fn delta()"),
        "第 4 行應正確顯示行號與函式內容: {line2_str}"
    );
}

#[test]
/// 驗證 preview_file_slice 能從磁碟檔案直接提取特定區間並輸出切片。
fn test_preview_file_slice_from_disk() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("config.toml");
    fs::write(
        &file_path,
        "name = \"app\"\nversion = \"1.0.0\"\nactive = true\n",
    )
    .expect("write toml");

    let lines = preview_file_slice(&file_path, 1, 2, 3);
    assert_eq!(lines.len(), 2);
    let line1 = &lines[0];
    let line1_str = line1.to_string();
    assert!(line1_str.contains(" 2 version = \"1.0.0\""));
}
