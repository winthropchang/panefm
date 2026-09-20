use std::{
    fs, io,
    path::Path,
    sync::atomic::{AtomicBool, Ordering as AtomicOrdering},
    thread,
    time::SystemTime,
};

use super::{
    sort::sort_file_entries,
    types::{DirectoryLoadProgress, SortMode},
};
use crate::file_manager::{entry::FileEntry, undo_backup::is_internal_temporary_name};

/// 快速統計指定路徑的檔案大小或資料夾完整容量。
///
/// 參數：`path: &Path`，要統計的來源。
/// 回傳：`io::Result<u64>`；資料夾會遞迴加總，無法讀取時回傳原始 I/O 錯誤。
#[allow(dead_code)]
pub(crate) fn path_content_size(path: &Path) -> io::Result<u64> {
    let metadata = fs::metadata(path)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    dir_content_size(path, &mut total)?;
    Ok(total)
}

fn dir_content_size(dir: &Path, total: &mut u64) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                let _ = dir_content_size(&entry.path(), total);
            } else if ft.is_file()
                && let Ok(meta) = entry.metadata()
            {
                *total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(())
}

/// 快速讀取目錄：在背景多執行緒讀取 metadata 並完成自然排序，直接送出 100% 正確排序的完整清單，徹底避免畫面列表跳動。
pub(crate) fn stream_dir_entries_with_cancellation<F>(
    path: &Path,
    sort_mode: SortMode,
    random_seed: u64,
    cancelled: &AtomicBool,
    mut on_progress: F,
) -> io::Result<()>
where
    F: FnMut(DirectoryLoadProgress) -> bool,
{
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    let read_dir = fs::read_dir(path)?;
    let mut items = Vec::new();
    let mut first_batch = Vec::new();
    let mut sent_first_chunk = false;

    for dir_entry_result in read_dir {
        if cancelled.load(AtomicOrdering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "directory load cancelled",
            ));
        }

        let item = match dir_entry_result {
            Ok(item) => item,
            Err(_) => continue,
        };
        let name = item.file_name().to_string_lossy().into_owned();
        if is_internal_temporary_name(&name) {
            continue;
        }
        let file_type = item.file_type().ok();
        let is_dir = file_type.as_ref().map(|t| t.is_dir()).unwrap_or(false);
        let entry_path = item.path();

        if !sent_first_chunk {
            first_batch.push(FileEntry {
                name,
                path: entry_path,
                is_dir,
                size: 0,
                is_sparse_empty: false,
                directory_size: None,
                directory_size_complete: false,
                modified: SystemTime::UNIX_EPOCH,
                created: SystemTime::UNIX_EPOCH,
                readonly: false,
                unix_mode: None,
            });
            if first_batch.len() >= 128 {
                sort_file_entries(&mut first_batch, sort_mode, random_seed);
                let chunk = std::mem::take(&mut first_batch);
                if !on_progress(DirectoryLoadProgress::Batch {
                    entries: chunk,
                    is_first_chunk: true,
                }) {
                    return Ok(());
                }
                sent_first_chunk = true;
            }
        }

        items.push(item);
    }

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    // 第一批暫存清單只負責讓大型目錄立刻可見；Complete 則一定要在背景補齊
    // metadata。不能因為目前採 natural 排序就省略 metadata，因為使用者可能在進入
    // 目錄前已啟用 `ms`，或進入後才切換 size/permissions/mtime/btime。舊實作會讓
    // 這些背景載入的項目永久保留 size = 0，而直接建立的新 panel 卻有正確資料，造成
    // 同一路徑的兩個 panel 顯示不一致。這裡仍在 worker 執行緒平行讀取，不會阻塞 TUI。
    let mut final_entries = read_metadata_for_dir_entries(items, cancelled)?;

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    // 在背景執行緒進行自然排序，避免在主 UI 執行緒排序數萬筆檔案造成卡頓
    sort_file_entries(&mut final_entries, sort_mode, random_seed);

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    let _ = on_progress(DirectoryLoadProgress::Complete(final_entries));
    Ok(())
}

/// 讀取指定目錄，並整理成可顯示的檔案項目清單。
pub(crate) fn read_dir_entries(path: &Path) -> io::Result<Vec<FileEntry>> {
    read_dir_entries_with_cancellation(path, &AtomicBool::new(false))
}

/// 讀取指定目錄並支援即時取消，整理成可顯示的檔案項目清單。
pub(crate) fn read_dir_entries_with_cancellation(
    path: &Path,
    cancelled: &AtomicBool,
) -> io::Result<Vec<FileEntry>> {
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }
    let items = fs::read_dir(path)?
        .filter_map(Result::ok)
        .filter(|entry| !is_internal_temporary_name(&entry.file_name().to_string_lossy()))
        .collect::<Vec<_>>();
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }
    read_metadata_for_dir_entries(items, cancelled)
}

/// 讀取指定項目清單的完整 metadata，並支援中途取消。
fn read_metadata_for_dir_entries(
    items: Vec<fs::DirEntry>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<FileEntry>> {
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8)
        .min(items.len().max(1));

    // 小目錄直接處理可避免建立 thread 的成本；大型目錄則把 metadata 系統呼叫分散到
    // 有上限的 worker。這段仍需等清單完成才能排序，但不再讓數萬筆 metadata 串行阻塞。
    if items.len() < 512 || worker_count == 1 {
        let mut entries = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            if index % 64 == 0 && cancelled.load(AtomicOrdering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "directory load cancelled",
                ));
            }
            entries.push(file_entry_from_dir_entry(item));
        }
        return Ok(entries);
    }

    let chunk_size = items.len().div_ceil(worker_count);
    let mut chunks = items.into_iter();
    let results = thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..worker_count {
            let chunk = chunks.by_ref().take(chunk_size).collect::<Vec<_>>();
            if chunk.is_empty() {
                break;
            }
            workers.push(scope.spawn(move || {
                let mut chunk_entries = Vec::with_capacity(chunk.len());
                for (index, item) in chunk.into_iter().enumerate() {
                    if index % 64 == 0 && cancelled.load(AtomicOrdering::Relaxed) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "directory load cancelled",
                        ));
                    }
                    chunk_entries.push(file_entry_from_dir_entry(item));
                }
                Ok(chunk_entries)
            }));
        }

        workers
            .into_iter()
            .map(|worker| {
                worker.join().map_err(|_| {
                    io::Error::other("directory metadata worker terminated unexpectedly")
                })?
            })
            .collect::<io::Result<Vec<_>>>()
    })?;

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    Ok(results.into_iter().flatten().collect())
}

/// 將單一 `DirEntry` 轉成 PaneFM 列表資料。
fn file_entry_from_dir_entry(item: fs::DirEntry) -> FileEntry {
    let name = item.file_name().to_string_lossy().into_owned();
    let entry_path = item.path();
    let file_type = item.file_type().ok();
    let metadata = item
        .metadata()
        .or_else(|_| fs::symlink_metadata(&entry_path))
        .ok();
    let is_dir = file_type
        .as_ref()
        .map(|t| t.is_dir())
        .or_else(|| metadata.as_ref().map(|m| m.is_dir()))
        .unwrap_or(false);

    let (size, is_sparse_empty, modified, created, readonly, unix_mode) =
        if let Some(meta) = metadata {
            (
                meta.len(),
                is_metadata_sparse_empty(&meta),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                meta.created().unwrap_or(SystemTime::UNIX_EPOCH),
                meta.permissions().readonly(),
                read_unix_mode(&meta),
            )
        } else {
            (
                0,
                false,
                SystemTime::UNIX_EPOCH,
                SystemTime::UNIX_EPOCH,
                false,
                None,
            )
        };

    FileEntry {
        name,
        path: entry_path,
        is_dir,
        size,
        is_sparse_empty,
        directory_size: None,
        directory_size_complete: false,
        modified,
        created,
        readonly,
        unix_mode,
    }
}

/// 判斷檔案是否為實體未配置區塊（0 bytes on disk）的稀疏空洞或未完成檔案。
#[cfg(unix)]
fn is_metadata_sparse_empty(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    metadata.is_file() && metadata.len() > 0 && metadata.blocks() == 0
}

#[cfg(not(unix))]
fn is_metadata_sparse_empty(_: &fs::Metadata) -> bool {
    false
}

/// 讀取目前平台可提供的 Unix 權限位元，供 linemode permissions 顯示。
#[cfg(unix)]
fn read_unix_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    Some(metadata.mode())
}

/// 在非 Unix 平台上，目前沒有標準庫可直接讀完整 rwx 權限，因此回傳 `None`。
#[cfg(not(unix))]
fn read_unix_mode(_: &fs::Metadata) -> Option<u32> {
    None
}
