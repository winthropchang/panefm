//! 終端圖片預覽與檔案資訊黑箱整合測試。
//!
//! 依據 `DEVELOPMENT_GUIDELINES.md` 規範，黑箱整合測試獨立放置於 `tests/` 目錄，
//! 驗證：
//! 1. 純 Rust Halfblock 縮圖輸出符合終端 ANSI TrueColor 規範。
//! 2. 副檔名與檔案類型推測邏輯覆蓋常見檔案格式。
//! 3. 檔案大小同時支援以 1024 為基數的單位與精確 byte 數。

use std::io::Cursor;
use std::path::Path;

use image::{ImageBuffer, ImageFormat, Rgba};
use panefm::file_manager::preview::{
    detect_file_kind, format_number_with_commas, format_size_with_bytes, is_image_extension,
    render_image_halfblocks,
};

#[test]
/// 驗證真實 PNG 圖片可成功轉換為包含 `▀` 的 Halfblock 終端預覽行。
///
/// 驗證內容：
/// 1. 產生 16x16 的漸層圖片。
/// 2. 呼叫 `render_image_halfblocks` 進行渲染。
/// 3. 驗證產生的 Line 集合包含格式、尺寸資訊以及 `▀` 字元。
///
/// 保護目的：確保圖片預覽模組能作為公開 API 穩定被外部呼叫者或整合測試引用。
fn integration_image_preview_generates_valid_halfblock_lines() {
    let mut img = ImageBuffer::new(16, 16);
    for y in 0..16 {
        for x in 0..16 {
            let r = (x * 16) as u8;
            let g = (y * 16) as u8;
            img.put_pixel(x, y, Rgba([r, g, 128, 255]));
        }
    }
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("write png");

    let result = render_image_halfblocks(&bytes, 60, 20).expect("render halfblocks");
    assert!(!result.is_empty());
    let has_halfblock = result
        .iter()
        .any(|line| line.spans.iter().any(|span| span.content.as_ref() == "▀"));
    assert!(has_halfblock, "預覽行必須包含 Halfblock ▀ 字元");

    use panefm::file_manager::preview::format_image_title;
    let title = format_image_title(Some((1920, 1080)), Some("png"), 14_523_688, None, false);
    assert_eq!(title, "1920 × 1080 (PNG)  •  13.85 MiB  •  unknown");
}

#[test]
/// 驗證副檔名推測檔案類別函數涵蓋圖片、文件、壓縮檔、原始碼與設定檔。
///
/// 驗證內容：
/// 測試各類副檔名推測出的檔案類型字串。
///
/// 保護目的：避免型態標記重構後破壞檔案詳細資訊卡片上的類型名稱。
fn integration_file_kind_detection() {
    assert_eq!(
        detect_file_kind(Path::new("main.rs"), Some("rs")),
        "Rust Source Code"
    );
    assert_eq!(
        detect_file_kind(Path::new("photo.jpeg"), Some("jpeg")),
        "JPEG Image"
    );
    assert_eq!(
        detect_file_kind(Path::new("image.webp"), Some("webp")),
        "WebP Image"
    );
    assert_eq!(
        detect_file_kind(Path::new("icon.ico"), Some("ico")),
        "ICO Icon"
    );
    assert_eq!(
        detect_file_kind(Path::new("paper.pdf"), Some("pdf")),
        "PDF Document"
    );
    assert_eq!(
        detect_file_kind(Path::new("bundle.zip"), Some("zip")),
        "ZIP Archive"
    );
    assert_eq!(
        detect_file_kind(Path::new("movie.mp4"), Some("mp4")),
        "MP4 Video"
    );
}

#[test]
/// 驗證千位數逗號分隔與檔案大小格式化函數。
///
/// 驗證內容：
/// 測試數字逗號格式化與容量單位轉換。
///
/// 保護目的：確保使用者在詳細資訊卡片中看到的數值整齊易讀。
fn integration_size_formatting_with_bytes() {
    assert_eq!(format_number_with_commas(0), "0");
    assert_eq!(format_number_with_commas(999), "999");
    assert_eq!(format_number_with_commas(1000), "1,000");
    assert_eq!(format_number_with_commas(12345678), "12,345,678");

    assert_eq!(format_size_with_bytes(0), "0 B (0 bytes)");
    assert_eq!(format_size_with_bytes(1024), "1.0 KiB (1,024 bytes)");
    assert_eq!(
        format_size_with_bytes(10 * 1024 * 1024),
        "10.00 MiB (10,485,760 bytes)"
    );
}

#[test]
/// 驗證圖片副檔名檢驗函數不區分大小寫並正確辨識支援的圖片格式。
///
/// 驗證內容：
/// 測試大小寫 PNG、JPG、JPEG、WEBP、GIF、BMP、ICO 以及非圖片副檔名。
///
/// 保護目的：避免因為副檔名大小寫差異導致預覽分流失效。
fn integration_image_extension_matching() {
    assert!(is_image_extension(Some("png")));
    assert!(is_image_extension(Some("PNG")));
    assert!(is_image_extension(Some("jpg")));
    assert!(is_image_extension(Some("Jpeg")));
    assert!(is_image_extension(Some("webp")));
    assert!(is_image_extension(Some("gif")));
    assert!(is_image_extension(Some("bmp")));
    assert!(is_image_extension(Some("ico")));

    assert!(!is_image_extension(Some("txt")));
    assert!(!is_image_extension(Some("rs")));
    assert!(!is_image_extension(Some("pdf")));
    assert!(!is_image_extension(None));
}

#[test]
/// 驗證目錄偷窺預覽（方案 A）格式化函數能正確產生單排標題與區分目錄及檔案。
///
/// 驗證內容：
/// 1. 測試 `format_directory_title` 產生簡潔邊框標題。
/// 2. 測試 `is_archive_file` 能識別 zip 與 tar.gz 檔案。
/// 3. 測試 `format_archive_title` 格式化壓縮檔標題。
///
/// 保護目的：確保預覽標題與壓縮檔辨識之公開 API 結構穩定。
fn integration_directory_and_archive_preview_helpers() {
    use panefm::file_manager::preview::{
        format_archive_title, format_directory_title, is_archive_file,
    };

    let title = format_directory_title("Desktop", 9, 4, 5, None);
    assert_eq!(title, "Desktop  •  9 items (4 dirs, 5 files)");

    let empty_title = format_directory_title("Empty", 0, 0, 0, None);
    assert_eq!(empty_title, "Empty  •  0 items");

    assert_eq!(is_archive_file(Path::new("project.zip")), Some("zip"));
    assert_eq!(is_archive_file(Path::new("archive.tar.gz")), Some("tar.gz"));
    assert_eq!(is_archive_file(Path::new("archive.tgz")), Some("tar.gz"));
    assert_eq!(is_archive_file(Path::new("backup.tar")), Some("tar"));
    assert_eq!(is_archive_file(Path::new("document.pdf")), None);

    let arc_title = format_archive_title("bundle.zip", 48, 1_488_977);
    assert_eq!(
        arc_title,
        "bundle.zip  •  48 entries  •  1.42 MiB uncompressed"
    );
}

#[test]
/// 驗證外部呼叫者可直接呼叫 `highlight_code_preview` 為各類語言產生語法著色與暗色行號。
/// 保護目的：確保語法高亮模組作為公開 API 的簽章與行為保持長久穩定。
fn integration_code_preview_syntax_highlighting() {
    use panefm::file_manager::preview::highlight_code_preview;

    let path = Path::new("main.py");
    let python_code = "def hello():\n    print('world')\n";
    let lines = highlight_code_preview(path, python_code, 10, None);

    assert_eq!(lines.len(), 2);
    assert!(lines[0].spans.len() >= 2);
    assert_eq!(lines[0].spans[0].content.as_ref(), "  1 ");
    assert!(lines[0].spans[0].style.fg.is_some());
    assert_eq!(lines[0].to_string(), "  1 def hello():");
    assert_eq!(lines[1].to_string(), "  2     print('world')");
}

#[test]
/// 驗證 TOML 檔案（如 Cargo.toml、Config.toml）透過公開 API 可享有客製化語法高亮著色。
/// 保護目的：確保 TOML 專用解析器在整合情境下運作正常。
fn integration_toml_code_preview_syntax_highlighting() {
    use panefm::file_manager::preview::highlight_code_preview;

    let path = Path::new("Cargo.toml");
    let toml_code = "[package]\nname = \"panefm\"\nversion = \"0.1.12\"\n";
    let lines = highlight_code_preview(path, toml_code, 10, None);

    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].to_string(), "  1 [package]");
    assert!(
        lines[0]
            .spans
            .iter()
            .any(|s| s.content.as_ref() == "package" && s.style.fg.is_some())
    );
    assert!(
        lines[1]
            .spans
            .iter()
            .any(|s| s.content.as_ref() == "name" && s.style.fg.is_some())
    );
}
