use std::{
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering as AtomicOrdering},
        mpsc,
    },
    thread,
    time::Duration,
};

use super::ops::{move_path_with_fallback, remove_existing_target};
use crate::file_manager::{
    cow::{clone_file_cow, is_cow_unsupported_error},
    platform::is_network_path,
    undo_backup::create_unique_undo_backup_path,
};

/// 產生同一程序內不重複的暫存檔序號，避免同時複製多個項目時互相覆蓋。
static TRANSFER_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 單檔達到這個大小後，才值得建立額外 thread 輪詢目的檔大小。
pub(crate) const PROGRESSIVE_NATIVE_COPY_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

/// SMB 或其他檔案系統不支援平台原生 copy 時，分塊傳輸使用的 buffer 大小。
const STREAM_COPY_BUFFER_BYTES: usize = 1024 * 1024;

pub(crate) fn copy_file_and_verify(source_path: &Path, staged_path: &Path) -> io::Result<()> {
    match clone_file_cow(source_path, staged_path) {
        Ok(()) => Ok(()),
        Err(error) if is_cow_unsupported_error(&error) => {
            if is_network_path(source_path) || is_network_path(staged_path) {
                return copy_file_streaming_with_progress(source_path, staged_path, &mut |_| {});
            }
            match copy_file_and_verify_with(source_path, staged_path, |source, target| {
                fs::copy(source, target)
            }) {
                Ok(()) => Ok(()),
                Err(err) if native_copy_supports_stream_fallback(&err) => {
                    remove_partial_file_for_fallback(staged_path, &err)?;
                    copy_file_streaming_with_progress(source_path, staged_path, &mut |_| {})
                }
                Err(err) => Err(err),
            }
        }
        Err(error) => Err(error),
    }
}

/// 使用平台原生 copy 複製檔案，並在 copy 執行期間輪詢目的檔大小更新背景進度。
///
/// 優先嘗試 CoW 秒級克隆；跨磁區或跨 SMB 傳輸時在 blocking worker 中執行，
/// 另一個非同步工作定期讀取目的檔 metadata 輪詢進度。
///
/// 參數：`source_path`、`target_path` 為來源與目標；`progress` 在單檔完整完成後收到
/// 寫入增量。回傳：`io::Result<()>`；原生 copy 或大小驗證失敗時回傳原始 I/O 錯誤。
pub(crate) fn copy_file_native_with_progress<F>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
{
    let expected_size = fs::metadata(source_path)?.len();

    // 優先嘗試 CoW 秒級克隆（同 APFS 磁區 0ms 完成免建立 thread）
    match clone_file_cow(source_path, target_path) {
        Ok(()) => {
            progress(expected_size);
            return Ok(());
        }
        Err(error) if is_cow_unsupported_error(&error) => {
            // 跨磁區、跨網路芳鄰（SMB）或不支援檔案系統時，平滑進入進度輪詢或串流
        }
        Err(error) => return Err(error),
    }

    // 跨網路芳鄰（SMB/UNC/網路磁碟機）時，Windows CopyFileExW 或 macOS copyfile
    // 常因伺服器不支援 Server-Side Copy Offload 或擴展屬性而留下 0-byte 假死，
    // 或受客戶端 redirector 快取欺騙。因此網路路徑一律直走分塊串流複製與 sync_all 落盤！
    if is_network_path(source_path) || is_network_path(target_path) {
        return copy_file_streaming_with_progress(source_path, target_path, progress);
    }

    // 小檔案直接在既有 file worker 執行，避免每一筆檔案再建立一條監看 thread。
    // 對含數萬個小檔案的 build 目錄，這個分支是主要效能路徑。
    if expected_size < PROGRESSIVE_NATIVE_COPY_THRESHOLD_BYTES {
        return copy_file_with_native_fallback_known_size(
            source_path,
            target_path,
            expected_size,
            progress,
            |source, target| fs::copy(source, target),
        );
    }

    let mut native_reported = 0u64;
    let native_result = copy_file_native_with_progress_using(
        source_path,
        target_path,
        &mut |increment| {
            native_reported = native_reported.saturating_add(increment);
            progress(increment);
        },
        |source, target| fs::copy(source, target),
    );
    match native_result {
        Ok(()) => Ok(()),
        Err(error) if native_copy_supports_stream_fallback(&error) => {
            // macOS 的 copyfile 與 Windows redirector 在部分 SMB server 會留下 0-byte
            // 目標再回傳 not supported。串流重試前一定先移除，才能使用 create_new
            // 保證不會覆寫其他程序剛建立的檔案。
            remove_partial_file_for_fallback(target_path, &error)?;
            let mut fallback_reported = 0u64;
            copy_file_streaming_with_progress(source_path, target_path, &mut |increment| {
                fallback_reported = fallback_reported.saturating_add(increment);
                // 原生路徑已經把 partial 大小回報給 task，fallback 從零重寫時不能再
                // 重複累加同一段範圍；超過原生已回報量後才繼續增加百分比。
                let previous = fallback_reported.saturating_sub(increment);
                let newly_visible = fallback_reported.saturating_sub(native_reported)
                    - previous.saturating_sub(native_reported);
                if newly_visible > 0 {
                    progress(newly_visible);
                }
            })
        }
        Err(error) => Err(error),
    }
}

/// 先嘗試平台原生 copy，不支援時安全切換到跨平台分塊串流。
///
/// 參數：
/// - `source_path: &Path`，來源檔案。
/// - `target_path: &Path`，尚不存在的目的檔案。
/// - `progress: &mut F`，接收已完整寫入的 byte 增量。
/// - `native_copy: C`，可注入的平台原生 copy；正式環境使用 `std::fs::copy`。
///
/// 回傳：`io::Result<()>`；成功前一定驗證目的大小，原生 API 的一般權限或磁碟錯誤
/// 不會被 fallback 隱藏，只有明確的「不支援」錯誤才會改走串流。
#[allow(dead_code)]
pub(crate) fn copy_file_with_native_fallback<F, C>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
    native_copy: C,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
    C: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    let expected_size = fs::metadata(source_path)?.len();
    copy_file_with_native_fallback_known_size_using(
        source_path,
        target_path,
        expected_size,
        progress,
        |_, _| {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "test simulated fallback",
            ))
        },
        native_copy,
    )
}

/// 使用呼叫端已取得的來源大小執行原生 copy 與串流 fallback。
pub(crate) fn copy_file_with_native_fallback_known_size<F, C>(
    source_path: &Path,
    target_path: &Path,
    expected_size: u64,
    progress: &mut F,
    native_copy: C,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
    C: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    copy_file_with_native_fallback_known_size_using(
        source_path,
        target_path,
        expected_size,
        progress,
        clone_file_cow,
        native_copy,
    )
}

/// 接收 CoW 與平台原生 copy 實作的底層單檔複製器。
pub(crate) fn copy_file_with_native_fallback_known_size_using<F, CoW, C>(
    source_path: &Path,
    target_path: &Path,
    expected_size: u64,
    progress: &mut F,
    cow_copy: CoW,
    native_copy: C,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
    CoW: FnOnce(&Path, &Path) -> io::Result<()>,
    C: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    if target_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("copy target already exists: {}", target_path.display()),
        ));
    }

    // 優先嘗試 CoW 秒級克隆
    match cow_copy(source_path, target_path) {
        Ok(()) => {
            progress(expected_size);
            return Ok(());
        }
        Err(error) if is_cow_unsupported_error(&error) => {
            // 跨磁區/跨 SMB/不支援時平滑走 native copy
        }
        Err(error) => return Err(error),
    }

    match native_copy(source_path, target_path) {
        Ok(copied_size) => {
            // 原生 copy 宣稱完成後，重新開啟目標執行 sync_all，迫使 OS 沖刷 dirty buffer
            if let Ok(target_file) = fs::OpenOptions::new().write(true).open(target_path) {
                let _ = sync_target_file(&target_file);
            }
            let stored_size = fs::metadata(target_path).map(|m| m.len()).unwrap_or(0);
            if copied_size == expected_size && stored_size == expected_size {
                progress(expected_size);
                Ok(())
            } else {
                let error = io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "incomplete native copy: expected {expected_size} bytes, copied {copied_size}, stored {stored_size}"
                    ),
                );
                remove_partial_file_for_fallback(target_path, &error)?;
                copy_file_streaming_with_progress(source_path, target_path, progress)
            }
        }
        Err(error) if native_copy_supports_stream_fallback(&error) => {
            remove_partial_file_for_fallback(target_path, &error)?;
            copy_file_streaming_with_progress(source_path, target_path, progress)
        }
        Err(error) => Err(error),
    }
}

/// 判斷平台原生 copy 的錯誤是否適合改用一般 read/write 串流重試。
///
/// Rust 在 macOS 可能直接傳回 `ENOTSUP`（45），Windows SMB redirector 常見
/// `ERROR_INVALID_FUNCTION`（1）或 `ERROR_NOT_SUPPORTED`（50）。這些錯誤只表示伺服器
/// 不支援原生 copy 加速，不代表一般檔案寫入也失敗。
///
/// 參數：`error: &io::Error`，原生 copy 回傳的錯誤。
/// 回傳：`bool`，只有可安全降級的 unsupported 類型回傳 `true`。
fn native_copy_supports_stream_fallback(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported || error.kind() == io::ErrorKind::UnexpectedEof {
        return true;
    }

    match error.raw_os_error() {
        #[cfg(unix)]
        // EXDEV = 18, EINVAL = 22, ENOTSUP = 45 (macOS) / 95 (Linux), ENOSYS = 78 (macOS) / 38 (Linux), EOPNOTSUPP = 102 (macOS), EPERM = 1
        Some(1) | Some(18) | Some(22) | Some(38) | Some(45) | Some(78) | Some(95) | Some(102) => {
            true
        }
        #[cfg(windows)]
        // ERROR_INVALID_FUNCTION = 1, ERROR_NOT_SAME_DEVICE = 17, ERROR_NOT_SUPPORTED = 50
        Some(1) | Some(17) | Some(50) => true,
        _ => false,
    }
}

/// 清除原生 copy 失敗後可能留下的 0-byte 或 partial 目的檔。
///
/// 參數：`target_path: &Path` 是要重試的目標；`native_error: &io::Error` 是原始錯誤。
/// 回傳：`io::Result<()>`；清理失敗時同時保留原生與清理錯誤，不能在未知 partial
/// 上繼續寫入。
fn remove_partial_file_for_fallback(
    target_path: &Path,
    native_error: &io::Error,
) -> io::Result<()> {
    if !target_path.exists() {
        return Ok(());
    }
    fs::remove_file(target_path).map_err(|cleanup_error| {
        io::Error::new(
            cleanup_error.kind(),
            format!(
                "native copy is unsupported ({native_error}); removing partial target {} failed: {cleanup_error}",
                target_path.display()
            ),
        )
    })
}

/// 嘗試將檔案內容沖刷至儲存媒體（落盤）。
///
/// 若底層檔案系統或遠端網路磁碟（如 macOS smbfs / NFS / FAT32 / FUSE）
/// 不支援硬體級快取沖刷（例如 macOS 的 `fcntl(F_FULLFSYNC)` 回傳 `ENOTSUP` os error 45），
/// 則平滑忽略該項不支援錯誤，因為檔案資料已透過 `write` 與 `flush` 完整傳輸並由系統與伺服器管理。
pub(crate) fn sync_target_file(file: &File) -> io::Result<()> {
    match file.sync_all() {
        Ok(()) => Ok(()),
        Err(err) if is_sync_unsupported_error(&err) => Ok(()),
        Err(err) => Err(err),
    }
}

/// 判斷 `sync_all` 回傳的錯誤是否屬於檔案系統或網路協定不支援硬體級落盤的非致命錯誤。
pub(crate) fn is_sync_unsupported_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    match error.raw_os_error() {
        #[cfg(unix)]
        // EPERM = 1, EINVAL = 22, ENOSYS = 38 (Linux) / 78 (macOS), ENOTSUP = 45 (macOS) / 95 (Linux), EOPNOTSUPP = 102 (macOS) / 95 (Linux)
        Some(1) | Some(22) | Some(38) | Some(45) | Some(78) | Some(95) | Some(102) => true,
        #[cfg(windows)]
        // ERROR_INVALID_FUNCTION = 1, ERROR_NOT_SUPPORTED = 50, ERROR_CALL_NOT_IMPLEMENTED = 120
        Some(1) | Some(50) | Some(120) => true,
        _ => false,
    }
}

/// 使用固定大小 buffer 跨平台串流複製單一檔案並即時回報進度。
///
/// 這是 SMB 不支援平台原生 copy 時的可靠 fallback，不是本機預設路徑。目的檔使用
/// `create_new`，因此不會意外覆寫其他程序在 fallback 前建立的同名項目；寫完會 flush、
/// 關閉 handle，再重新讀 metadata 驗證完整大小。
///
/// 參數：`source_path: &Path`、`target_path: &Path` 為來源與目的；`progress: &mut F`
/// 接收每一塊成功寫入的 byte 數。
/// 回傳：`io::Result<()>`；任何 read/write/flush/驗證錯誤都交由外層交易清理 partial。
pub(crate) fn copy_file_streaming_with_progress<F>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
{
    let mut source = BufReader::with_capacity(STREAM_COPY_BUFFER_BYTES, File::open(source_path)?);
    let expected_size = source.get_ref().metadata()?.len();
    let target_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_path)?;
    let mut target = BufWriter::with_capacity(STREAM_COPY_BUFFER_BYTES, target_file);
    let mut buffer = vec![0u8; STREAM_COPY_BUFFER_BYTES];
    let mut copied = 0u64;

    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        target.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
        progress(read as u64);
    }
    target.flush()?;
    sync_target_file(target.get_ref())?;
    drop(target);

    let source_size_after_copy = fs::metadata(source_path)?.len();
    let stored_size = fs::metadata(target_path)?.len();
    if copied != expected_size
        || source_size_after_copy != expected_size
        || stored_size != expected_size
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "incomplete streaming copy: expected {expected_size} bytes, source now {source_size_after_copy}, copied {copied}, stored {stored_size}"
            ),
        ));
    }

    // 權限複製是 best effort：部分 SMB server 可寫內容但拒絕 chmod。資料完整性已經
    // 驗證成功，不應因伺服器不支援 Unix 權限而把有效檔案判定為失敗。
    if let Ok(metadata) = fs::metadata(source_path) {
        let _ = fs::set_permissions(target_path, metadata.permissions());
    }
    Ok(())
}

/// 執行可注入原生 copy 的 progressive copy 核心，供 metadata 輪詢行為做回歸測試。
///
/// 參數：`source_path`、`target_path` 與 `progress` 和公開核心相同；`native_copy` 型別為
/// `FnOnce(&Path, &Path) -> io::Result<u64> + Send`，會在 scoped worker 中執行。
/// 回傳：`io::Result<()>`；copy 進行中依目標檔大小回報增量，結束後驗證完整大小。
pub(crate) fn copy_file_native_with_progress_using<F, C>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
    native_copy: C,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
    C: FnOnce(&Path, &Path) -> io::Result<u64> + Send,
{
    if target_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("copy target already exists: {}", target_path.display()),
        ));
    }

    let expected_size = fs::metadata(source_path)?.len();
    let mut reported_size = 0u64;
    let copied_size = thread::scope(|scope| -> io::Result<u64> {
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        scope.spawn(move || {
            // 不能只靠固定 sleep 輪詢 `is_finished()`。小檔案通常在幾毫秒內完成，
            // 若每個檔案仍睡滿 200ms，大型 build 目錄會被人為拖慢到數十分鐘。
            // completion channel 會在 copy 返回時立即喚醒目前執行緒；只有大檔仍在
            // 傳輸時，timeout 才負責定期讀取 metadata 更新百分比。
            let _ = done_sender.send(native_copy(source_path, target_path));
        });

        loop {
            match done_receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let stored_size = fs::metadata(target_path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0)
                        .min(expected_size);
                    if stored_size > reported_size {
                        progress(stored_size - reported_size);
                        reported_size = stored_size;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(io::Error::other("native copy worker panicked"));
                }
            }
        }
    })?;
    if let Ok(target_file) = fs::OpenOptions::new().write(true).open(target_path) {
        let _ = sync_target_file(&target_file);
    }
    let stored_size = fs::metadata(target_path)?.len();
    if copied_size != expected_size || stored_size != expected_size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "incomplete native copy: expected {expected_size} bytes, copied {copied_size}, stored {stored_size}"
            ),
        ));
    }
    if copied_size > reported_size {
        progress(copied_size - reported_size);
    }
    Ok(())
}

/// 執行可注入平台複製器的單檔驗證核心，供不完整寫入情境做回歸測試。
///
/// 參數：
/// - `source_path: &Path`，來源檔案。
/// - `staged_path: &Path`，本次新建立的目標檔案。
/// - `platform_copy: F`，平台複製函數，型別為
///   `FnOnce(&Path, &Path) -> io::Result<u64>`，回傳宣稱已複製的 byte 數。
///
/// 回傳：`io::Result<()>`；只有平台 copy 返回且來源、回報值、目標大小一致才成功。
pub(crate) fn copy_file_and_verify_with<F>(
    source_path: &Path,
    staged_path: &Path,
    platform_copy: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    let expected_size = File::open(source_path)?.metadata()?.len();
    copy_file_and_verify_with_known_size(source_path, staged_path, expected_size, platform_copy)
}

/// 使用已知來源大小驗證平台 copy，避免大量小檔案重複開啟來源。
///
/// 參數：`expected_size: u64` 必須由本次 copy 開始前的來源 metadata 取得；其他參數與
/// [`copy_file_and_verify_with`] 相同。回傳：`io::Result<()>`；平台回報量及落盤目標大小
/// 都正確才成功。目的檔會以 metadata 重新確認，因此 SMB 的 0-byte 假成功仍會被拒絕。
pub(crate) fn copy_file_and_verify_with_known_size<F>(
    source_path: &Path,
    staged_path: &Path,
    expected_size: u64,
    platform_copy: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    if staged_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("copy target already exists: {}", staged_path.display()),
        ));
    }

    let copied_size = platform_copy(source_path, staged_path)?;
    if let Ok(target_file) = fs::OpenOptions::new().write(true).open(staged_path) {
        let _ = sync_target_file(&target_file);
    }
    let stored_size = fs::metadata(staged_path)?.len();
    if copied_size != expected_size || stored_size != expected_size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "incomplete copy: expected {expected_size} bytes, copied {copied_size}, stored {stored_size}"
            ),
        ));
    }
    Ok(())
}

/// 將已完成的暫存內容切換成正式名稱，覆蓋時先備份舊目標以便失敗回復。
///
/// 參數：
/// - `staged_path: &Path`，已完整寫入的暫存路徑。
/// - `target_path: &Path`，正式目標路徑。
/// - `overwrite: bool`，是否允許替換既有目標。
///
/// 回傳：`io::Result<()>`；提交失敗時會盡力把舊目標從備份改回原名。
pub(crate) fn commit_staged_copy(
    staged_path: &Path,
    target_path: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<Option<PathBuf>> {
    if target_path.exists() && !overwrite {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("paste target already exists: {}", target_path.display()),
        ));
    }

    let backup_path = target_path
        .exists()
        .then(|| create_unique_undo_backup_path(target_path));
    if let Some(backup_path) = &backup_path {
        move_path_with_fallback(target_path, backup_path)?;
    }

    if let Err(error) = fs::rename(staged_path, target_path) {
        if let Some(backup_path) = &backup_path {
            let _ = move_path_with_fallback(backup_path, target_path);
        }
        return Err(io::Error::new(
            error.kind(),
            format!("finish paste to {} failed: {error}", target_path.display()),
        ));
    }

    if let Some(backup_path) = &backup_path
        && !retain_backup
    {
        remove_transfer_path(backup_path)?;
    }
    Ok(backup_path.filter(|_| retain_backup))
}

/// 為交易式傳輸產生唯一的暫存檔名，避免與現有檔案衝突。
///
/// 參數：
/// - `target_path: &Path`，正式目標路徑，用來取得相同父目錄。
/// - `role: &str`，暫存用途，例如 `part` 或 `backup`。
///
/// 回傳：`PathBuf`，目前不存在且可供本次傳輸使用的路徑。
pub(crate) fn unique_transfer_path(target_path: &Path, role: &str) -> PathBuf {
    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let sequence = TRANSFER_TEMP_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
        let candidate = parent.join(format!(
            ".panefm-transfer-{}-{sequence}.{role}",
            std::process::id()
        ));
        if !candidate.exists() {
            return candidate;
        }
    }
}

/// 刪除交易式複製使用的檔案或資料夾暫存路徑。
///
/// 參數：`path: &Path`，要清除的內部暫存路徑。
/// 回傳：`io::Result<()>`；路徑不存在視為已完成清理。
pub(crate) fn remove_transfer_path(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    remove_existing_target(path)
}
