use std::{fs, path::Path};

use tempfile::tempdir;

use super::{
    ArchiveFormat, compress_entries_to_zip, compress_entries_to_zip_with_progress,
    default_extract_output_path, detect_archive_format, extract_entries_with_progress,
    validate_archive_before_extract,
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
        is_sparse_empty: false,
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
        is_sparse_empty: false,
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
        is_sparse_empty: false,
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

#[test]
/// 驗證 0 位元組空檔案在解壓前會被立即攔截，並回傳明確診斷訊息。
fn validate_archive_before_extract_rejects_empty_file() {
    let dir = tempdir().expect("tempdir");
    let empty_zip = dir.path().join("empty.zip");
    fs::File::create(&empty_zip).expect("create empty");

    let err = validate_archive_before_extract(&empty_zip, dir.path(), ArchiveFormat::Zip)
        .expect_err("should reject empty file");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("0 位元組 (空檔案)"));
}

#[test]
/// 驗證全為 0 的空洞檔案在解壓前會被攔截，並診斷出內容全為 0。
fn validate_archive_before_extract_rejects_all_zero_file() {
    let dir = tempdir().expect("tempdir");
    let zero_zip = dir.path().join("zeros.zip");
    fs::write(&zero_zip, vec![0u8; 1024]).expect("write zeros");

    let err = validate_archive_before_extract(&zero_zip, dir.path(), ArchiveFormat::Zip)
        .expect_err("should reject all zeros");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("內容全為 0"));
}

#[test]
/// 驗證缺少 PK 檔頭的無效 ZIP 檔案會被精確診斷為缺少 PK 標頭。
fn validate_archive_before_extract_rejects_invalid_zip_header() {
    let dir = tempdir().expect("tempdir");
    let bad_zip = dir.path().join("corrupt.zip");
    fs::write(&bad_zip, b"This is plain text pretending to be zip").expect("write bad zip");

    let err = validate_archive_before_extract(&bad_zip, dir.path(), ArchiveFormat::Zip)
        .expect_err("should reject invalid zip header");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("缺少 PK 標頭"));
}

#[test]
/// 驗證合法產生的 ZIP 檔案能順利通過解壓前預檢。
fn validate_archive_before_extract_accepts_valid_zip() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("doc.txt");
    fs::write(&source, "hello world").expect("write source");

    let entry = FileEntry {
        name: String::from("doc.txt"),
        path: source,
        is_dir: false,
        size: 11,
        is_sparse_empty: false,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let zip_path = compress_entries_to_zip(dir.path(), &[entry]).expect("compress");
    let result = validate_archive_before_extract(&zip_path, dir.path(), ArchiveFormat::Zip);
    assert!(result.is_ok());
}

#[cfg(unix)]
#[test]
/// 驗證 Unix 平台（APFS / ext4）上標稱大小大於 0 但實體區塊為 0 的稀疏檔案會被精確攔截。
fn validate_archive_before_extract_rejects_sparse_empty_file() {
    use std::os::unix::fs::MetadataExt;

    let dir = tempdir().expect("tempdir");
    let sparse_zip = dir.path().join("sparse.zip");
    let file = fs::File::create(&sparse_zip).expect("create file");
    file.set_len(10 * 1024 * 1024).expect("set len to 10MB");
    drop(file);

    let meta = fs::metadata(&sparse_zip).expect("metadata");
    if meta.blocks() == 0 {
        let err = validate_archive_before_extract(&sparse_zip, dir.path(), ArchiveFormat::Zip)
            .expect_err("should reject sparse empty file");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            err.to_string()
                .contains("未配置任何實體磁區 (0 bytes on disk)")
        );
    }
}

#[cfg(unix)]
#[test]
fn compress_directory_with_broken_symlink_does_not_fail() {
    let dir = tempdir().expect("tempdir");
    let folder = dir.path().join("my_repo");
    fs::create_dir(&folder).expect("create dir");
    fs::write(folder.join("valid.txt"), "hello").expect("write file");
    std::os::unix::fs::symlink(folder.join("nonexistent_file"), folder.join("broken_link"))
        .expect("symlink");

    let entry = FileEntry {
        name: String::from("my_repo"),
        path: folder,
        is_dir: true,
        size: 5,
        is_sparse_empty: false,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let zip_path = compress_entries_to_zip(dir.path(), &[entry]).expect("compress should succeed");
    assert!(zip_path.exists());

    let zip_entry = FileEntry {
        name: String::from("my_repo.zip"),
        path: zip_path.clone(),
        is_dir: false,
        size: fs::metadata(&zip_path).expect("meta").len(),
        is_sparse_empty: false,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let extract_dir = dir.path().join("extracted");
    fs::create_dir(&extract_dir).expect("create extract dir");
    let (_, skipped) = super::extract_entries(&extract_dir, &[zip_entry]).expect("extract");
    assert_eq!(skipped, 0);
    let files = walkdir_simple(&extract_dir);
    assert!(
        files
            .iter()
            .any(|f| f.file_name().unwrap_or_default() == "broken_link"),
        "broken_link should exist in extract_dir: {files:?}"
    );
    assert!(
        files
            .iter()
            .any(|f| f.file_name().unwrap_or_default() == "valid.txt"),
        "valid.txt should exist in extract_dir: {files:?}"
    );
}

#[allow(dead_code)]
fn walkdir_simple(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            results.push(path.clone());
            if let Ok(meta) = fs::symlink_metadata(&path)
                && meta.is_dir()
            {
                results.extend(walkdir_simple(&path));
            }
        }
    }
    results
}

#[test]
/// 驗證壓縮包含無權限子目錄或中途消失的檔案時，能優雅略過並成功完成整體壓縮。
/// 保護目的：避免大型專案目錄（如 .git 或 node_modules）內部的暫存檔或權限造成整批失敗。
fn compress_directory_with_unreadable_or_missing_files_does_not_fail() {
    let dir = tempdir().expect("tempdir");
    let folder = dir.path().join("mixed_repo");
    fs::create_dir(&folder).expect("create dir");
    fs::write(folder.join("regular.txt"), "regular content").expect("write");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let restricted = folder.join("restricted_dir");
        fs::create_dir(&restricted).expect("create restricted");
        fs::write(restricted.join("secret.txt"), "secret").expect("write secret");
        fs::set_permissions(&restricted, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    }

    let entry = FileEntry {
        name: String::from("mixed_repo"),
        path: folder.clone(),
        is_dir: true,
        size: 15,
        is_sparse_empty: false,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let zip_path = compress_entries_to_zip(dir.path(), &[entry]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let restricted = folder.join("restricted_dir");
        let _ = fs::set_permissions(&restricted, fs::Permissions::from_mode(0o755));
    }

    let zip_path = zip_path.expect("compress should succeed despite unreadable items");
    assert!(zip_path.exists());
}

#[test]
/// 驗證 Git pack 與 loose objects 能被 is_already_compressed_file 正確辨識為已壓縮，
/// 避免在大型 Git 倉庫壓縮時因重複 Deflate 造成 CPU 飆高與長時間卡頓。
fn is_already_compressed_file_recognizes_git_pack_and_loose_objects() {
    assert!(super::is_already_compressed_file(Path::new(
        "repo/.git/objects/pack/pack-1234567890abcdef.pack"
    )));
    assert!(super::is_already_compressed_file(Path::new(
        "repo/.git/objects/pack/pack-1234567890abcdef.idx"
    )));
    assert!(super::is_already_compressed_file(Path::new(
        "repo/.git/objects/4b/825dc642cb6eb9a060e54bf8d69288fbee4904"
    )));
    assert!(super::is_already_compressed_file(Path::new("archive.whl")));
    assert!(super::is_already_compressed_file(Path::new(
        "package.crate"
    )));
    assert!(super::is_already_compressed_file(Path::new("font.woff2")));
    assert!(!super::is_already_compressed_file(Path::new("main.rs")));
    assert!(!super::is_already_compressed_file(Path::new("notes.txt")));
}

#[test]
/// 驗證壓縮包含 Git pack 與一般檔案的目錄時，pack 檔案會以 Stored 模式寫入，一般檔案以 Deflated 寫入。
fn compress_directory_with_git_pack_uses_stored_mode() {
    let dir = tempdir().expect("tempdir");
    let repo_dir = dir.path().join("my_repo");
    let git_pack_dir = repo_dir.join(".git").join("objects").join("pack");
    fs::create_dir_all(&git_pack_dir).expect("create pack dir");
    fs::write(git_pack_dir.join("test.pack"), b"fake_pack_content").expect("write pack");
    fs::write(
        repo_dir.join("code.rs"),
        b"fn main() { println!(\"hello\"); }",
    )
    .expect("write code");

    let entry = FileEntry {
        name: String::from("my_repo"),
        path: repo_dir,
        is_dir: true,
        size: 100,
        is_sparse_empty: false,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let zip_path = compress_entries_to_zip(dir.path(), &[entry]).expect("compress");
    let file = fs::File::open(&zip_path).expect("open zip");
    let mut archive = zip::ZipArchive::new(file).expect("zip archive");

    {
        let pack_entry = archive
            .by_name("my_repo/.git/objects/pack/test.pack")
            .expect("find pack in zip");
        assert_eq!(pack_entry.compression(), zip::CompressionMethod::Stored);
    }

    {
        let code_entry = archive
            .by_name("my_repo/code.rs")
            .expect("find code in zip");
        assert_eq!(code_entry.compression(), zip::CompressionMethod::Deflated);
    }
}

#[test]
/// 驗證壓縮未預先掃描大小的目錄時，calculate_entries_uncompressed_bytes 能正確走訪並加總真實未壓縮大小，
/// 避免僅使用目錄 inode 大小（如 192 bytes）導致任務進度過早顯示為 100%。
fn background_compress_directory_calculates_uncompressed_size_accurately() {
    let dir = tempdir().expect("tempdir");
    let test_folder = dir.path().join("my_project");
    fs::create_dir_all(test_folder.join("sub")).expect("create dir");
    fs::write(test_folder.join("sub").join("data.txt"), vec![b'a'; 10_000]).expect("write file");
    fs::write(test_folder.join("sub").join("code.rs"), vec![b'b'; 5_000]).expect("write file");

    let entry = FileEntry {
        name: String::from("my_project"),
        path: test_folder,
        is_dir: true,
        size: 192, // 模擬 APFS 目錄 inode 僅有 192 bytes
        is_sparse_empty: false,
        directory_size: None, // 未預先掃描
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let total = crate::file_manager::app::file_ops::archive::calculate_entries_uncompressed_bytes(
        &[entry],
        192,
    );
    assert_eq!(total, 15_000);
}

#[test]
#[cfg(unix)]
/// 驗證當目錄中存在未配置實體區塊的檔案 (0 bytes on disk / UF_DATALESS 雲端檔案) 時，
/// 壓縮能夠優雅處理而不引發核心讀取阻塞，並順利產出合法 ZIP。
fn compress_directory_with_dataless_or_zero_block_file_does_not_hang() {
    let dir = tempdir().expect("tempdir");
    let test_folder = dir.path().join("cloud_folder");
    fs::create_dir_all(&test_folder).expect("create dir");

    let sparse_file_path = test_folder.join("cloud_placeholder.bin");
    let file = fs::File::create(&sparse_file_path).expect("create file");
    file.set_len(10 * 1024 * 1024).expect("set_len to 10MB");
    drop(file);

    fs::write(test_folder.join("normal.txt"), b"regular content").expect("write regular");

    let entry = FileEntry {
        name: String::from("cloud_folder"),
        path: test_folder,
        is_dir: true,
        size: 192,
        is_sparse_empty: false,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::now(),
        created: std::time::SystemTime::now(),
        readonly: false,
        unix_mode: None,
    };

    let zip_path = compress_entries_to_zip(dir.path(), &[entry]).expect("compress");
    assert!(zip_path.exists());
    let zip_file = fs::File::open(&zip_path).expect("open zip");
    let mut archive = zip::ZipArchive::new(zip_file).expect("zip archive");
    assert!(archive.by_name("cloud_folder/normal.txt").is_ok());
    assert!(
        archive
            .by_name("cloud_folder/cloud_placeholder.bin")
            .is_ok()
    );
}
