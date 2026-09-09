use std::{fs, path::Path};

use tempfile::tempdir;

use super::{
    ArchiveFormat, compress_entries_to_zip, compress_entries_to_zip_with_progress,
    default_extract_output_path, detect_archive_format, extract_entries_with_progress,
};
use crate::file_manager::entry::FileEntry;

#[test]
/// 驗證副檔名辨識涵蓋 zip、tar、tar.gz 與 tgz，並拒絕一般檔案。
/// 保護目的：避免壓縮格式或輸出命名調整後，造成跨平台無法解壓、覆蓋既有資料或路徑不安全。
fn detect_archive_format_recognizes_supported_extensions() {
    assert_eq!(
        detect_archive_format(Path::new("demo.zip")),
        Some(ArchiveFormat::Zip)
    );
    assert_eq!(
        detect_archive_format(Path::new("demo.tar.gz")),
        Some(ArchiveFormat::TarGz)
    );
    assert_eq!(
        detect_archive_format(Path::new("demo.tar")),
        Some(ArchiveFormat::Tar)
    );
    assert_eq!(
        detect_archive_format(Path::new("demo.gz")),
        Some(ArchiveFormat::Gz)
    );
    assert_eq!(detect_archive_format(Path::new("demo.txt")), None);
}

#[test]
/// 驗證解壓目的地同名時會建立 copy 名稱，不覆蓋既有目錄。
/// 保護目的：避免壓縮格式或輸出命名調整後，造成跨平台無法解壓、覆蓋既有資料或路徑不安全。
fn default_extract_output_path_avoids_name_collisions() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha"), "existing").expect("write");

    let output = default_extract_output_path(
        dir.path(),
        &dir.path().join("alpha.zip"),
        ArchiveFormat::Zip,
    );

    assert_eq!(
        output.file_name().and_then(|name| name.to_str()),
        Some("alpha copy")
    );
}

#[test]
/// 驗證單一檔案壓縮會產生可讀取的預設 ZIP，並保存原始檔名。
/// 保護目的：避免壓縮格式或輸出命名調整後，造成跨平台無法解壓、覆蓋既有資料或路徑不安全。
fn compress_entries_to_zip_creates_archive_with_default_name() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("notes.txt");
    fs::write(&source, "hello archive").expect("file");

    let entry = FileEntry {
        name: String::from("notes.txt"),
        path: source,
        is_dir: false,
        size: 13,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let archive = compress_entries_to_zip(dir.path(), &[entry]).expect("compress");
    assert_eq!(
        archive.file_name().and_then(|name| name.to_str()),
        Some("notes.txt.zip")
    );
    assert!(archive.exists());
}

#[test]
/// 驗證背景壓縮與解壓會在實際搬移內容時持續回報 byte 數。
///
/// 保護目的：task 百分比不能只在完成時跳到 100%；若 archive 內部改回無 callback
/// 的 `io::copy`，這個測試會立即指出執行中進度已失效。
fn archive_operations_report_non_zero_progress() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("progress.txt");
    let content = vec![b'x'; 64 * 1024];
    fs::write(&source, &content).expect("source");
    let source_entry = FileEntry {
        name: String::from("progress.txt"),
        path: source,
        is_dir: false,
        size: content.len() as u64,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };
    let mut compressed_bytes = 0u64;
    let archive =
        compress_entries_to_zip_with_progress(dir.path(), &[source_entry], &mut |increment| {
            compressed_bytes += increment
        })
        .expect("compress with progress");
    let archive_size = fs::metadata(&archive).expect("archive metadata").len();
    let archive_entry = FileEntry {
        name: String::from("progress.txt.zip"),
        path: archive,
        is_dir: false,
        size: archive_size,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };
    let mut extracted_bytes = 0u64;

    extract_entries_with_progress(dir.path(), &[archive_entry], &mut |increment| {
        extracted_bytes += increment
    })
    .expect("extract with progress");

    assert_eq!(compressed_bytes, content.len() as u64);
    assert_eq!(extracted_bytes, content.len() as u64);
}
