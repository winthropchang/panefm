use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Instant;

use ignore::{WalkBuilder, WalkState};

use super::{ClipboardOperation, ClipboardState};
use crate::file_manager::archive::ExtractedArchive;
use crate::file_manager::entry::FileEntry;
use crate::file_manager::operation_history::OperationItem;
use crate::file_manager::pane::{DirectoryLoadProgress, PaneState, TransferProgress};

/// 大型檔案工作完成後送回主執行緒的事件。
///
/// worker 只處理檔案 I/O，不直接修改 [`App`]；操作歷史、panel reload 與狀態列都在
/// 主迴圈收到事件後更新，避免跨執行緒共享可變 UI 狀態。
#[derive(Debug)]
pub(crate) enum FileJobEvent {
    /// 背景貼上已建立第一層目標，主執行緒可立即刷新目的 panel，不必等待整批完成。
    DestinationVisible { target_dir: PathBuf },
    /// worker 定期回報累計 byte，主執行緒直接更新 task 面板與持久化歷史。
    Progress {
        task_id: usize,
        completed_bytes: u64,
        total_bytes: u64,
    },
    Paste {
        task_id: usize,
        clipboard: ClipboardState,
        overwrite: bool,
        result: PasteJobResult,
    },
    Compress {
        task_id: usize,
        pane_id: usize,
        entry_count: usize,
        first_name: String,
        result: io::Result<PathBuf>,
    },
    Extract {
        task_id: usize,
        pane_id: usize,
        result: io::Result<(Vec<ExtractedArchive>, usize)>,
    },
    Delete {
        task_id: usize,
        target_name: String,
        result: io::Result<Vec<String>>,
    },
}

/// 目錄大小 worker 傳回主執行緒的增量事件。
#[derive(Debug)]
pub(crate) enum DirectorySizeEvent {
    /// 單一直接子目錄目前已統計的 byte，以及該子樹是否已完成。
    Update {
        path: PathBuf,
        bytes: u64,
        complete: bool,
    },
    /// 目前 panel 啟動的整批直接子目錄都已完成或已取消。
    Done,
}

/// 保存單一 panel 的目錄大小背景工作。
///
/// `cwd` 用來拒絕切換目錄後晚到的舊結果；`cancelled` 讓新掃描取代舊掃描時，舊
/// worker 能在下一個檔案邊界停止，不會持續佔用磁碟或 SMB 連線。
#[derive(Debug)]
pub(crate) struct DirectorySizeJob {
    pub(crate) cwd: PathBuf,
    pub(crate) receiver: Receiver<DirectorySizeEvent>,
    pub(crate) cancelled: Arc<AtomicBool>,
}

/// 保存單一 panel 的目錄清單背景載入工作。
///
/// `cwd` 用來比對當前目錄；`cancelled` 讓新導航發生時，舊 worker 能立即在分塊邊界停止，
/// 避免背景磁碟 I/O 阻塞主事件迴圈。
#[derive(Debug)]
pub(crate) struct DirectoryLoadJob {
    pub(crate) cwd: PathBuf,
    pub(crate) receiver: Receiver<DirectoryLoadEvent>,
    pub(crate) cancelled: Arc<AtomicBool>,
}

/// 大型目錄背景讀取完成或分批串流送回主迴圈的資料。
#[derive(Debug)]
pub(crate) struct DirectoryLoadEvent {
    pub(crate) pane_id: usize,
    pub(crate) cwd: PathBuf,
    pub(crate) selected_path: Option<PathBuf>,
    pub(crate) result: io::Result<DirectoryLoadProgress>,
}

/// `ms` 背景掃描回報部分容量的最小間隔。
///
/// 200ms 可讓大型目錄的數字明顯持續前進，同時不會為每個檔案都傳送事件而
/// 壓垮 TUI 主執行緒。
pub(crate) const DIRECTORY_SIZE_UPDATE_INTERVAL_MS: u64 = 200;

/// 背景 paste 完成的批次結果，包含成功項目及第一個失敗原因。
#[derive(Debug)]
pub(crate) struct PasteJobResult {
    pub(crate) history_items: Vec<OperationItem>,
    pub(crate) pasted_count: usize,
    pub(crate) failure: Option<PasteJobFailure>,
}

/// 記錄背景 paste 的失敗項目，供主執行緒顯示完整來源、目的與 OS error。
#[derive(Debug)]
pub(crate) struct PasteJobFailure {
    pub(crate) display_name: String,
    pub(crate) planned_target: PathBuf,
    pub(crate) error: io::Error,
}

/// 超過此大小的單批檔案工作一律放到背景，避免可感知的 TUI 停頓。
pub(crate) const BACKGROUND_FILE_JOB_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

/// 判斷 paste 是否應移出主執行緒。
///
/// 參數：`clipboard: &ClipboardState` 為來源批次；`target_dir: &Path` 為目的目錄。
/// 回傳：`bool`；來源包含目錄、目的地為網路磁碟，或來源檔案總大小達門檻時回傳
/// `true`。目錄一律在背景處理，避免為了判斷大小而在 UI thread 遞迴走訪內容。
pub(crate) fn paste_should_run_in_background(
    clipboard: &ClipboardState,
    target_dir: &Path,
) -> bool {
    is_probably_network_or_external_path(target_dir)
        || clipboard
            .entries
            .iter()
            .any(|entry| entry.source_path.is_dir())
        || clipboard
            .entries
            .iter()
            .filter_map(|entry| fs::metadata(&entry.source_path).ok())
            .map(|metadata| metadata.len())
            .try_fold(0u64, |total, size| total.checked_add(size))
            .is_none_or(|total| total >= BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
}

/// 判斷壓縮或解壓項目是否可能長時間占用 CPU／磁碟。
///
/// 參數：`entries: &[FileEntry]`，目前選取項目。
/// 回傳：`bool`；資料夾或總檔案大小達 8 MiB 時使用背景工作。
pub(crate) fn entries_should_run_in_background(entries: &[FileEntry]) -> bool {
    entries.iter().any(|entry| entry.is_dir)
        || entries
            .iter()
            .map(|entry| entry.size)
            .try_fold(0u64, |total, size| total.checked_add(size))
            .is_none_or(|total| total >= BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
}

/// 以跨平台保守規則辨識可能產生長延遲的網路或外接 volume。
///
/// 參數：`path: &Path`，要寫入的目標。
/// 回傳：`bool`；Windows UNC 與 macOS `/Volumes/...` 回傳 `true`。Windows 映射磁碟
/// 無法只靠路徑可靠辨識，但大型來源仍會由大小門檻轉入背景。
pub(crate) fn is_probably_network_or_external_path(path: &Path) -> bool {
    crate::file_manager::platform::is_network_path(path)
}

/// 在 worker 執行完整 paste 批次，不接觸 App 或任何可變 UI 狀態。
///
/// 參數：`clipboard: &ClipboardState` 為固定來源；`target_dir: &Path` 為目的目錄；
/// `overwrite: bool` 表示是否覆蓋。
/// 回傳：`PasteJobResult`，包含成功的 Undo 資料及第一個失敗項目。
pub(crate) fn perform_paste_job<F>(
    clipboard: &ClipboardState,
    target_dir: &Path,
    overwrite: bool,
    progress: &mut F,
) -> PasteJobResult
where
    F: FnMut(TransferProgress),
{
    let mut history_items = Vec::new();
    let mut pasted_count = 0usize;

    for entry in &clipboard.entries {
        if entry.source_path.parent() == Some(target_dir)
            && clipboard.operation == ClipboardOperation::Cut
        {
            continue;
        }

        let planned_target =
            PaneState::planned_paste_target_in_dir(&entry.source_path, target_dir, overwrite)
                .unwrap_or_else(|_| target_dir.join(&entry.display_name));
        let result = match clipboard.operation {
            ClipboardOperation::Copy => PaneState::copy_path_to_dir_with_history_progress(
                &entry.source_path,
                target_dir,
                overwrite,
                progress,
            ),
            ClipboardOperation::Cut => PaneState::move_path_to_dir_with_history_progress(
                &entry.source_path,
                target_dir,
                overwrite,
                progress,
            ),
        };
        match result {
            Ok(outcome) => {
                pasted_count += 1;
                history_items.push(OperationItem {
                    source_path: entry.source_path.clone(),
                    destination_path: outcome.target_path,
                    replaced_backup: outcome.backup_path,
                });
            }
            Err(error) => {
                return PasteJobResult {
                    history_items,
                    pasted_count,
                    failure: Some(PasteJobFailure {
                        display_name: entry.display_name.clone(),
                        planned_target,
                        error,
                    }),
                };
            }
        }
    }

    PasteJobResult {
        history_items,
        pasted_count,
        failure: None,
    }
}

/// 平行計算目前 panel 每個直接子目錄的真實內容大小。
///
/// 參數：
/// - `directories: Vec<PathBuf>`，目前 panel 的直接子目錄；每個路徑各自成為一列。
/// - `cancelled: &AtomicBool`，切換 linemode、目錄或重啟掃描時由主執行緒設為 `true`。
/// - `sender: &mpsc::Sender<DirectorySizeEvent>`，把部分值與最終值送回 TUI。
///
/// 回傳：`() `。工作數受 CPU 平行度限制；目錄少時會把額度用於同一棵大型
/// 子樹，目錄多時則會同時計算多列。無法讀取的項目會略過，且不追蹤 symlink，
/// 避免循環目錄與意外走訪其他 share。各列最多約每 200ms 回報一次，完成時一定回報
/// 精確終值。
pub(crate) fn scan_directory_sizes(
    directories: Vec<PathBuf>,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<DirectorySizeEvent>,
) {
    if directories.is_empty() {
        return;
    }

    let available_workers = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 12);
    let root_workers = available_workers.min(directories.len());
    let threads_per_root = (available_workers / root_workers).max(1);
    let next_root = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..root_workers {
            let directories = &directories;
            let next_root = &next_root;
            scope.spawn(move || {
                loop {
                    if cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let index = next_root.fetch_add(1, Ordering::Relaxed);
                    let Some(root) = directories.get(index) else {
                        return;
                    };
                    scan_one_directory_size(root, cancelled, sender, threads_per_root);
                }
            });
        }
    });
}

/// 使用受限制的平行 walker 計算單一直接子目錄。
///
/// 參數：`root` 是列表中的直接子目錄；`cancelled` 是取消旗標；`sender` 將部分與
/// 最終 byte 傳回 TUI；`worker_threads` 是這棵子樹可使用的最大執行緒數。
///
/// 回傳：`() `。每個一般檔案只累加 `metadata.len()`；目錄本身與 symbolic link 不計入，
/// 因此結果代表內容的 logical bytes，不是檔案系統的磁碟配置空間。
pub(crate) fn scan_one_directory_size(
    root: &Path,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<DirectorySizeEvent>,
    worker_threads: usize,
) {
    let total_bytes = AtomicU64::new(0);
    let last_update_ms = AtomicU64::new(0);
    let started_at = Instant::now();
    let mut walker = WalkBuilder::new(root);
    walker
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .ignore(false)
        .follow_links(false)
        .threads(worker_threads.max(1));

    walker.build_parallel().run(|| {
        Box::new(|result| {
            if cancelled.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            let Some(file_type) = entry.file_type() else {
                return WalkState::Continue;
            };
            if !file_type.is_file() {
                return WalkState::Continue;
            }
            let Ok(metadata) = entry.metadata() else {
                return WalkState::Continue;
            };
            let bytes = total_bytes
                .fetch_add(metadata.len(), Ordering::Relaxed)
                .saturating_add(metadata.len());
            let elapsed_ms = started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let previous_update = last_update_ms.load(Ordering::Relaxed);
            if should_report_directory_size(elapsed_ms, previous_update)
                && last_update_ms
                    .compare_exchange(
                        previous_update,
                        elapsed_ms,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
            {
                let _ = sender.send(DirectorySizeEvent::Update {
                    path: root.to_path_buf(),
                    bytes,
                    complete: false,
                });
            }
            WalkState::Continue
        })
    });

    if !cancelled.load(Ordering::Relaxed) {
        let _ = sender.send(DirectorySizeEvent::Update {
            path: root.to_path_buf(),
            bytes: total_bytes.load(Ordering::Relaxed),
            complete: true,
        });
    }
}

/// 判斷目錄容量部分結果是否已到下一次回報時間。
///
/// 參數：`elapsed_ms: u64` 是掃描啟動後的毫秒數；`previous_update_ms: u64` 是上次回報時間。
/// 回傳：`bool`；間隔達 200ms 時為 `true`，否則為 `false`。
pub(crate) fn should_report_directory_size(elapsed_ms: u64, previous_update_ms: u64) -> bool {
    elapsed_ms.saturating_sub(previous_update_ms) >= DIRECTORY_SIZE_UPDATE_INTERVAL_MS
}

/// byte 進度發生變化時送出事件；呼叫端負責以時間節流，避免大量 channel 訊息。
///
/// 參數：`sender` 為 worker event channel；`task_id` 為工作編號；`completed_bytes` 與
/// `total_bytes` 為累計量；`last_progress` 保存上次送出的 byte 組合。
/// 回傳：`() `；channel 已關閉時安靜停止回報，不影響檔案工作的錯誤處理。
pub(crate) fn send_progress_if_changed(
    sender: &mpsc::Sender<FileJobEvent>,
    task_id: usize,
    completed_bytes: u64,
    total_bytes: u64,
    last_progress: &mut Option<(u64, u64)>,
) {
    let progress = (completed_bytes, total_bytes.max(completed_bytes));
    if *last_progress == Some(progress) {
        return;
    }
    *last_progress = Some(progress);
    let _ = sender.send(FileJobEvent::Progress {
        task_id,
        completed_bytes: progress.0,
        total_bytes: progress.1,
    });
}

pub(crate) fn ensure_path_writable(path: &Path) {
    if let Ok(mut perms) = fs::metadata(path).map(|m| m.permissions())
        && perms.readonly()
    {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = fs::set_permissions(path, perms);
    }
}

/// 移除單一檔案或符號連結，遇到權限受阻時自動嘗試解除唯讀後重試，並回傳釋放的 byte 數。
pub(crate) fn remove_file_or_symlink_with_retry(path: &Path) -> io::Result<u64> {
    let size = fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0);
    if let Err(err) = fs::remove_file(path) {
        if err.kind() == io::ErrorKind::PermissionDenied {
            ensure_path_writable(path);
            if let Some(parent) = path.parent() {
                ensure_path_writable(parent);
            }
            fs::remove_file(path)?;
            return Ok(size);
        }
        if err.kind() != io::ErrorKind::NotFound {
            return Err(err);
        }
    }
    Ok(size)
}

const DELETE_WORKERS: usize = 8;

/// 移除目錄，遇到暫時性檔案鎖定（如 SMB 句柄延遲釋放）或 macOS 自動產生的系統隱藏檔（如 `.DS_Store`、`._*`）
/// 時，自動清理殘留隱藏檔並配合短暫退避重試，若最終仍無法移除則回傳錯誤。
pub(crate) fn remove_dir_with_retry(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }

    ensure_path_writable(path);
    if fs::remove_dir(path).is_ok() {
        return Ok(());
    }

    let mut last_error = None;
    // 退避間隔：15ms, 30ms, 60ms, 120ms
    let backoff_delays = [
        std::time::Duration::from_millis(15),
        std::time::Duration::from_millis(30),
        std::time::Duration::from_millis(60),
        std::time::Duration::from_millis(120),
    ];

    for delay in backoff_delays {
        if !path.exists() {
            return Ok(());
        }

        thread::sleep(delay);
        ensure_path_writable(path);

        // 清理可能在刪除過程中被 macOS Finder 或其他程式新生成的殘留隱藏檔
        if let Ok(rd) = fs::read_dir(path) {
            for entry in rd.flatten() {
                let entry_path = entry.path();
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() && !ft.is_symlink() {
                        let _ = fs::remove_dir_all(&entry_path);
                    } else {
                        let _ = remove_file_or_symlink_with_retry(&entry_path);
                    }
                } else {
                    let _ = remove_file_or_symlink_with_retry(&entry_path);
                }
            }
        }

        ensure_path_writable(path);
        if fs::remove_dir(path).is_ok() {
            return Ok(());
        }

        // 嘗試以 remove_dir_all 作為保底
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_error = Some(err);
            }
        }

        if !path.exists() {
            return Ok(());
        }
    }

    if path.exists() {
        ensure_path_writable(path);
        if fs::remove_dir(path).is_err() {
            if let Err(all_err) = fs::remove_dir_all(path) {
                return Err(last_error.unwrap_or(all_err));
            }
        }
    }

    if path.exists() {
        return Err(last_error.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("failed to remove directory '{}': directory still exists", path.display()),
            )
        }));
    }

    Ok(())
}

/// 高速遞迴刪除子目錄或檔案，遇到唯讀權限受阻時自動嘗試排除。
pub(crate) fn remove_dir_all_fast_recursive<F>(path: &Path, on_progress: &mut F) -> io::Result<()>
where
    F: FnMut(u64),
{
    ensure_path_writable(path);
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut child_error = None;
    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        if let Ok(file_type) = entry.file_type() {
            if file_type.is_dir() && !file_type.is_symlink() {
                if let Err(err) = remove_dir_all_fast_recursive(&entry_path, on_progress) {
                    if child_error.is_none() {
                        child_error = Some(err);
                    }
                }
            } else {
                match remove_file_or_symlink_with_retry(&entry_path) {
                    Ok(size) => on_progress(size),
                    Err(err) => {
                        if child_error.is_none() {
                            child_error = Some(err);
                        }
                    }
                }
            }
        } else {
            match remove_file_or_symlink_with_retry(&entry_path) {
                Ok(size) => on_progress(size),
                Err(err) => {
                    if child_error.is_none() {
                        child_error = Some(err);
                    }
                }
            }
        }
    }
    if let Some(err) = child_error {
        let _ = remove_dir_with_retry(path);
        return Err(err);
    }
    remove_dir_with_retry(path)
}

/// 多執行緒平行刪除目錄，大幅提高 NVMe/SSD 與檔案系統的 unlink 吞吐量並回報 byte 進度。
pub(crate) fn remove_dir_all_parallel_with_progress<F>(
    path: &Path,
    on_progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64) + Send,
{
    ensure_path_writable(path);
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let entries: Vec<PathBuf> = read_dir.flatten().map(|e| e.path()).collect();
    if entries.is_empty() {
        return remove_dir_with_retry(path);
    }

    let worker_error = std::sync::Mutex::new(None::<io::Error>);

    if entries.len() <= 4 {
        for child in &entries {
            if child.is_dir() && !child.is_symlink() {
                if let Err(err) = remove_dir_all_fast_recursive(child, on_progress) {
                    let mut guard = worker_error.lock().unwrap();
                    if guard.is_none() {
                        *guard = Some(err);
                    }
                }
            } else {
                match remove_file_or_symlink_with_retry(child) {
                    Ok(size) => on_progress(size),
                    Err(err) => {
                        let mut guard = worker_error.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some(err);
                        }
                    }
                }
            }
        }
    } else {
        let chunk_size = entries.len().div_ceil(DELETE_WORKERS);
        let progress_mutex = std::sync::Mutex::new(on_progress);
        let w_err_ref = &worker_error;
        thread::scope(|scope| {
            for chunk in entries.chunks(chunk_size) {
                let chunk = chunk.to_vec();
                let p_mutex = &progress_mutex;
                scope.spawn(move || {
                    let mut local_bytes = 0u64;
                    let mut local_progress = |increment: u64| {
                        local_bytes = local_bytes.saturating_add(increment);
                        if local_bytes >= 1024 * 512 {
                            if let Ok(mut guard) = p_mutex.lock() {
                                guard(local_bytes);
                            }
                            local_bytes = 0;
                        }
                    };
                    for child in chunk {
                        if child.is_dir() && !child.is_symlink() {
                            if let Err(err) = remove_dir_all_fast_recursive(&child, &mut local_progress) {
                                if let Ok(mut guard) = w_err_ref.lock()
                                    && guard.is_none()
                                {
                                    *guard = Some(err);
                                }
                            }
                        } else {
                            match remove_file_or_symlink_with_retry(&child) {
                                Ok(size) => local_progress(size),
                                Err(err) => {
                                    if let Ok(mut guard) = w_err_ref.lock()
                                        && guard.is_none()
                                    {
                                        *guard = Some(err);
                                    }
                                }
                            }
                        }
                    }
                    if local_bytes > 0
                        && let Ok(mut guard) = p_mutex.lock()
                    {
                        guard(local_bytes);
                    }
                });
            }
        });
    }

    let dir_res = remove_dir_with_retry(path);
    if let Ok(guard) = worker_error.into_inner()
        && let Some(err) = guard
    {
        return Err(err);
    }
    dir_res
}

/// 建立 paste 成功後的狀態文字，讓同步與背景流程使用相同規則。
///
/// 參數：`operation` 為 copy/cut；`overwrite` 表示覆蓋；`count` 為成功數量。
/// 回傳：`String`，可直接顯示於狀態列並寫入 task detail。
pub(crate) fn paste_success_status(
    operation: ClipboardOperation,
    overwrite: bool,
    count: usize,
) -> String {
    match operation {
        ClipboardOperation::Copy if overwrite && count == 1 => {
            String::from("pasted copy with overwrite: 1 item")
        }
        ClipboardOperation::Copy if overwrite => {
            format!("pasted copy with overwrite: {count} items")
        }
        ClipboardOperation::Copy if count == 1 => String::from("pasted copy: 1 item"),
        ClipboardOperation::Copy => format!("pasted copy: {count} items"),
        ClipboardOperation::Cut if overwrite && count == 1 => {
            String::from("moved with overwrite: 1 item")
        }
        ClipboardOperation::Cut if overwrite => format!("moved with overwrite: {count} items"),
        ClipboardOperation::Cut if count == 1 => String::from("moved: 1 item"),
        ClipboardOperation::Cut => format!("moved: {count} items"),
    }
}

/// 建立貼上失敗時供 status area 顯示的完整診斷訊息。
///
/// 第一行只描述失敗的來源項目，讓使用者能快速辨識是哪一筆操作；第二行保留完整
/// destination 與作業系統錯誤。UI 會依終端寬度自動換行，因此 UNC/SMB 長路徑即使
/// 需要三行以上也不會遺失尾端最重要的 OS error。
///
/// 參數：
/// - `source_name: &str`，貼上來源的顯示名稱。
/// - `destination: &Path`，本次操作實際預計使用的完整目標路徑。
/// - `error: &io::Error`，底層檔案系統或作業系統回傳的原始錯誤。
///
/// 回傳：`String`，含明確換行及完整診斷資訊的狀態文字。
pub(crate) fn paste_failure_status(
    source_name: &str,
    destination: &Path,
    error: &io::Error,
) -> String {
    format!(
        "paste failed for {source_name}\ndestination: {} | OS error: {error}",
        destination.display()
    )
}
