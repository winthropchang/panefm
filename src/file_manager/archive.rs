//! ZIP、tar.gz 與 tar 的跨平台壓縮/解壓縮實作。
//!
//! 壓縮固定產生 ZIP 以確保 macOS/Windows 都能開啟；解壓可辨識多種格式。所有輸出
//! 路徑都必須經過 collision 與 path traversal 防護，不能直接信任 archive 內的名稱。

use std::{
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
};

use flate2::read::GzDecoder;
use tar::Archive as TarArchive;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::FileOptions};

use super::entry::FileEntry;

/// 描述目前支援辨識與解壓的壓縮檔格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveFormat {
    Zip,
    TarGz,
    Tar,
    Gz,
}

/// 記錄單次解壓後產生的結果，方便呼叫端更新狀態與游標位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedArchive {
    pub(crate) archive_path: PathBuf,
    pub(crate) output_path: PathBuf,
}

/// 將目前選取或標記的一批項目壓成單一 zip 檔。
///
/// 參數：
/// - `cwd: &Path`，目前 pane 所在目錄，壓縮檔會建立在這裡。
/// - `entries: &[FileEntry]`，要被壓縮的檔案或資料夾清單。
///
/// 回傳：`io::Result<PathBuf>`。
/// - 成功時回傳實際建立出的 zip 路徑。
/// - 失敗時回傳建立檔案、走訪目錄或寫入壓縮內容時的錯誤。
pub(crate) fn compress_entries_to_zip(cwd: &Path, entries: &[FileEntry]) -> io::Result<PathBuf> {
    compress_entries_to_zip_with_progress(cwd, entries, &mut |_| {})
}

/// 將選取項目壓成 ZIP，並以讀取來源內容的 byte 數回報背景進度。
///
/// 參數：`cwd`、`entries` 與 [`compress_entries_to_zip`] 相同；`progress` 接收本輪
/// 新讀取的 byte 數。
/// 回傳：`io::Result<PathBuf>`；成功時回傳完整關閉後的 ZIP 路徑。
pub(crate) fn compress_entries_to_zip_with_progress<F>(
    cwd: &Path,
    entries: &[FileEntry],
    progress: &mut F,
) -> io::Result<PathBuf>
where
    F: FnMut(u64),
{
    if entries.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no entries selected for compression",
        ));
    }

    let archive_name = default_archive_name(entries);
    let archive_path = unique_path_in_dir(cwd, &archive_name, false);
    let file = File::create(&archive_path)?;
    let buffered = BufWriter::with_capacity(1024 * 1024, file);
    let mut zip = ZipWriter::new(buffered);
    let file_options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));
    let dir_options = FileOptions::default().compression_method(CompressionMethod::Stored);
    let mut buffer = vec![0u8; 256 * 1024];

    for entry in entries {
        let relative_path = PathBuf::from(&entry.name);
        add_path_to_zip(
            &mut zip,
            &entry.path,
            &relative_path,
            entry.is_dir,
            file_options,
            dir_options,
            &mut buffer,
            progress,
        )?;
    }

    let mut buffered = zip.finish()?;
    buffered.flush()?;
    Ok(archive_path)
}

/// 解壓目前選取或標記的一批壓縮檔，並回傳實際展開出的輸出路徑。
///
/// 參數：
/// - `cwd: &Path`，目前 pane 所在目錄，解壓結果會建立在這裡。
/// - `entries: &[FileEntry]`，要被解壓的檔案清單。
///
/// 回傳：`io::Result<(Vec<ExtractedArchive>, usize)>`。
/// - 第一個值是成功解壓的結果列表。
/// - 第二個值是被略過的非支援項目數量。
pub(crate) fn extract_entries(
    cwd: &Path,
    entries: &[FileEntry],
) -> io::Result<(Vec<ExtractedArchive>, usize)> {
    extract_entries_with_progress(cwd, entries, &mut |_| {})
}

/// 解壓選取項目，並以寫入輸出內容的 byte 數回報背景進度。
///
/// 參數：`cwd`、`entries` 與 [`extract_entries`] 相同；`progress` 接收本輪寫入量。
/// 回傳：`io::Result<(Vec<ExtractedArchive>, usize)>`，包含成功結果與略過數量。
pub(crate) fn extract_entries_with_progress<F>(
    cwd: &Path,
    entries: &[FileEntry],
    progress: &mut F,
) -> io::Result<(Vec<ExtractedArchive>, usize)>
where
    F: FnMut(u64),
{
    let mut extracted = Vec::new();
    let mut skipped = 0usize;

    for entry in entries {
        if entry.is_dir {
            skipped += 1;
            continue;
        }

        let Some(format) = detect_archive_format(&entry.path) else {
            skipped += 1;
            continue;
        };

        let output_path = default_extract_output_path(cwd, &entry.path, format);
        match format {
            ArchiveFormat::Zip => extract_zip_archive(&entry.path, &output_path, progress)?,
            ArchiveFormat::TarGz => extract_tar_gz_archive(&entry.path, &output_path, progress)?,
            ArchiveFormat::Tar => extract_tar_archive(&entry.path, &output_path, progress)?,
            ArchiveFormat::Gz => extract_gz_file(&entry.path, &output_path, progress)?,
        }

        extracted.push(ExtractedArchive {
            archive_path: entry.path.clone(),
            output_path,
        });
    }

    Ok((extracted, skipped))
}

/// 根據副檔名推斷目前檔案屬於哪一種壓縮格式。
pub(crate) fn detect_archive_format(path: &Path) -> Option<ArchiveFormat> {
    let lower = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveFormat::TarGz)
    } else if lower.ends_with(".zip") {
        Some(ArchiveFormat::Zip)
    } else if lower.ends_with(".tar") {
        Some(ArchiveFormat::Tar)
    } else if lower.ends_with(".gz") {
        Some(ArchiveFormat::Gz)
    } else {
        None
    }
}

/// 根據壓縮檔名稱決定預設的解壓輸出位置，並自動避開同名衝突。
pub(crate) fn default_extract_output_path(
    cwd: &Path,
    archive_path: &Path,
    format: ArchiveFormat,
) -> PathBuf {
    let file_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive");
    let base_name = archive_output_base_name(file_name, format);
    let candidate = cwd.join(base_name);
    unique_path_in_dir(
        cwd,
        candidate
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("archive"),
        !matches!(format, ArchiveFormat::Gz),
    )
}

/// 依目前選取內容建立適合拿來當 zip 檔名的預設名稱。
fn default_archive_name(entries: &[FileEntry]) -> String {
    if entries.len() == 1 {
        format!("{}.zip", entries[0].name)
    } else {
        String::from("archive.zip")
    }
}

/// 檢查副檔名是否為已壓縮格式，此類檔案使用 Stored 模式避免無效的 CPU 運算。
fn is_already_compressed_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "zip"
            | "gz"
            | "tgz"
            | "7z"
            | "rar"
            | "xz"
            | "zst"
            | "bz2"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "gif"
            | "mp4"
            | "mkv"
            | "mov"
            | "avi"
            | "mp3"
            | "flac"
            | "ogg"
            | "aac"
            | "pdf"
            | "docx"
            | "xlsx"
            | "pptx"
            | "apk"
            | "jar"
            | "war"
            | "wasm"
    )
}

/// 將指定路徑遞迴寫入 zip，保留目前 pane 目錄下看到的相對名稱。
fn add_path_to_zip<W, F>(
    zip: &mut ZipWriter<W>,
    source_path: &Path,
    relative_path: &Path,
    is_dir: bool,
    file_options: FileOptions,
    dir_options: FileOptions,
    buffer: &mut [u8],
    progress: &mut F,
) -> io::Result<()>
where
    W: io::Write + io::Seek,
    F: FnMut(u64),
{
    let archive_name = normalize_archive_path(relative_path);
    if is_dir {
        let dir_name = if archive_name.ends_with('/') {
            archive_name.clone()
        } else {
            format!("{archive_name}/")
        };
        zip.add_directory(dir_name, dir_options)?;
        for item in fs::read_dir(source_path)? {
            let item = item?;
            let item_is_dir = item.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            let next_relative = relative_path.join(item.file_name());
            add_path_to_zip(
                zip,
                &item.path(),
                &next_relative,
                item_is_dir,
                file_options,
                dir_options,
                buffer,
                progress,
            )?;
        }
    } else {
        let options = if is_already_compressed_file(source_path) {
            dir_options
        } else {
            file_options
        };
        zip.start_file(archive_name, options)?;
        let mut file = File::open(source_path)?;
        copy_with_progress(&mut file, zip, buffer, progress)?;
    }
    Ok(())
}

/// 解開 zip 壓縮檔到指定輸出目錄。
fn extract_zip_archive<F>(
    archive_path: &Path,
    output_dir: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64),
{
    fs::create_dir_all(output_dir)?;
    let file = BufReader::with_capacity(512 * 1024, File::open(archive_path)?);
    let mut archive = ZipArchive::new(file)?;
    let mut buffer = vec![0u8; 256 * 1024];

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(safe_name) = sanitize_archive_member_name(entry.name()) else {
            continue;
        };
        let target_path = output_dir.join(safe_name);

        if entry.is_dir() {
            fs::create_dir_all(&target_path)?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output_file =
            BufWriter::with_capacity(256 * 1024, File::create(&target_path)?);
        copy_with_progress(&mut entry, &mut output_file, &mut buffer, progress)?;
        output_file.flush()?;
    }

    Ok(())
}

/// 解開 tar.gz 壓縮檔到指定輸出目錄。
fn extract_tar_gz_archive<F>(
    archive_path: &Path,
    output_dir: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64),
{
    fs::create_dir_all(output_dir)?;
    let file = BufReader::with_capacity(512 * 1024, File::open(archive_path)?);
    let decoder = GzDecoder::new(file);
    let mut archive = TarArchive::new(decoder);
    unpack_tar_archive(&mut archive, output_dir, progress)
}

/// 解開 tar 壓縮檔到指定輸出目錄。
fn extract_tar_archive<F>(
    archive_path: &Path,
    output_dir: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64),
{
    fs::create_dir_all(output_dir)?;
    let file = BufReader::with_capacity(512 * 1024, File::open(archive_path)?);
    let mut archive = TarArchive::new(file);
    unpack_tar_archive(&mut archive, output_dir, progress)
}

/// 解開單一 `.gz` 壓縮檔到指定輸出檔案。
fn extract_gz_file<F>(archive_path: &Path, output_file: &Path, progress: &mut F) -> io::Result<()>
where
    F: FnMut(u64),
{
    let file = BufReader::with_capacity(512 * 1024, File::open(archive_path)?);
    let mut decoder = GzDecoder::new(file);
    if let Some(parent) = output_file.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut buffer = vec![0u8; 256 * 1024];
    let mut output = BufWriter::with_capacity(256 * 1024, File::create(output_file)?);
    copy_with_progress(&mut decoder, &mut output, &mut buffer, progress)?;
    output.flush()?;
    Ok(())
}

/// 將 tar 類壓縮檔安全地解開到指定目錄，避免路徑穿越寫出到目標外。
fn unpack_tar_archive<R, F>(
    archive: &mut TarArchive<R>,
    output_dir: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    R: Read,
    F: FnMut(u64),
{
    let mut buffer = vec![0u8; 256 * 1024];
    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(path) = sanitize_archive_member_path(&entry.path()?) else {
            continue;
        };
        let target_path = output_dir.join(path);

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&target_path)?;
        } else {
            let mut output =
                BufWriter::with_capacity(256 * 1024, File::create(&target_path)?);
            copy_with_progress(&mut entry, &mut output, &mut buffer, progress)?;
            output.flush()?;
        }
    }
    Ok(())
}

/// 以固定 buffer 搬移資料並回報每輪完成量，避免 `io::copy` 無法觀察進度。
///
/// 參數：`reader`、`writer` 為來源與輸出；`progress` 接收每輪寫入 byte 數。
/// 回傳：`io::Result<u64>`，代表總寫入量。
fn copy_with_progress<R, W, F>(
    reader: &mut R,
    writer: &mut W,
    buffer: &mut [u8],
    progress: &mut F,
) -> io::Result<u64>
where
    R: Read,
    W: io::Write,
    F: FnMut(u64),
{
    let mut total = 0u64;
    loop {
        let read = reader.read(buffer)?;
        if read == 0 {
            break;
        }
        writer.write_all(&buffer[..read])?;
        total = total.saturating_add(read as u64);
        progress(read as u64);
    }
    Ok(total)
}

/// 把 zip 內部路徑轉成穩定的 `/` 形式，避免不同平台分隔符不一致。
fn normalize_archive_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => segment.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// 清理 zip 內部紀錄的字串路徑，只保留安全的相對內容。
fn sanitize_archive_member_name(name: &str) -> Option<PathBuf> {
    let raw = Path::new(name);
    sanitize_archive_member_path(raw)
}

/// 清理壓縮檔中的成員路徑，避免 `..` 或絕對路徑逃出輸出目錄。
fn sanitize_archive_member_path(path: &Path) -> Option<PathBuf> {
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => sanitized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if sanitized.as_os_str().is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

/// 根據原始壓縮檔名推導出預設輸出名稱。
fn archive_output_base_name(file_name: &str, format: ArchiveFormat) -> String {
    let lower = file_name.to_ascii_lowercase();
    match format {
        ArchiveFormat::Zip => strip_suffix_case_insensitive(file_name, &lower, ".zip")
            .unwrap_or_else(|| file_name.to_string()),
        ArchiveFormat::TarGz => strip_suffix_case_insensitive(file_name, &lower, ".tar.gz")
            .or_else(|| strip_suffix_case_insensitive(file_name, &lower, ".tgz"))
            .unwrap_or_else(|| file_name.to_string()),
        ArchiveFormat::Tar => strip_suffix_case_insensitive(file_name, &lower, ".tar")
            .unwrap_or_else(|| file_name.to_string()),
        ArchiveFormat::Gz => strip_suffix_case_insensitive(file_name, &lower, ".gz")
            .unwrap_or_else(|| file_name.to_string()),
    }
}

/// 用不分大小寫的方式移除檔名尾端副檔名。
fn strip_suffix_case_insensitive(original: &str, lower: &str, suffix: &str) -> Option<String> {
    lower
        .strip_suffix(suffix)
        .map(|trimmed| original[..trimmed.len()].to_string())
}

/// 在指定目錄中產生不與既有檔案衝突的路徑。
fn unique_path_in_dir(dir: &Path, file_name: &str, prefer_directory: bool) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(file_name);
    let (stem, extension) = if prefer_directory {
        (file_name.to_string(), None)
    } else {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(file_name)
            .to_string();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_string());
        (stem, extension)
    };

    let mut index = 1usize;
    loop {
        let name = if index == 1 {
            format!("{stem} copy")
        } else {
            format!("{stem} copy {index}")
        };
        let final_name = match &extension {
            Some(extension) => format!("{name}.{extension}"),
            None => name,
        };
        let next = dir.join(final_name);
        if !next.exists() {
            return next;
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
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
}
