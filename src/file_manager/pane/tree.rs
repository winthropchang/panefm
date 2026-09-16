use std::{
    collections::VecDeque,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc,
    },
    thread,
};

use super::{
    copy::{
        PROGRESSIVE_NATIVE_COPY_THRESHOLD_BYTES, copy_file_and_verify,
        copy_file_native_with_progress, copy_file_streaming_with_progress,
        copy_file_with_native_fallback_known_size,
    },
    types::TransferProgress,
};
use crate::file_manager::platform::is_network_path;

/// 目錄傳輸使用的固定檔案 worker 數量。
const COPY_FILE_WORKERS: usize = 3;

pub(crate) fn copy_dir_recursive(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    fs::create_dir(target_dir)?;

    for item in fs::read_dir(source_dir)? {
        let item = item?;
        let item_path = item.path();
        let next_target = target_dir.join(item.file_name());

        if item.file_type()?.is_dir() {
            copy_dir_recursive(&item_path, &next_target)?;
        } else {
            copy_file_and_verify(&item_path, &next_target)?;
        }
    }

    Ok(())
}

/// 遞迴複製資料夾並以檔案實際寫入量更新背景 task。
///
/// 參數：`source_dir`、`target_dir` 為來源與目標；`progress` 接收新增完成的 byte 數。
/// 回傳：`io::Result<()>`；任一檔案不完整就停止並由外層清除 partial 目錄。
pub(crate) fn copy_dir_recursive_with_progress<F>(
    source_dir: &Path,
    target_dir: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(TransferProgress),
{
    copy_dir_parallel_with_progress(
        source_dir,
        target_dir,
        progress,
        |source_path, target_path, expected_size, file_progress| {
            if is_network_path(source_path) || is_network_path(target_path) {
                copy_file_streaming_with_progress(source_path, target_path, file_progress)
            } else if expected_size < PROGRESSIVE_NATIVE_COPY_THRESHOLD_BYTES {
                copy_file_with_native_fallback_known_size(
                    source_path,
                    target_path,
                    expected_size,
                    file_progress,
                    |source, target| fs::copy(source, target),
                )
            } else {
                copy_file_native_with_progress(source_path, target_path, file_progress)
            }
        },
    )
}

/// 表示目錄走訪器交給 file worker 的單一複製工作。
#[derive(Debug)]
struct CopyFileJob {
    source_path: PathBuf,
    target_path: PathBuf,
    expected_size: u64,
}

/// 表示 file worker 完成一個工作後回傳給協調執行緒的結果。
#[derive(Debug)]
enum CopyFileResult {
    Discovered(u64),
    Progress(u64),
    Copied,
    Failed(io::Error),
}

/// 以有界工作佇列與固定 worker 數並行複製目錄中的檔案。
///
/// 這個流程採用成熟的 producer-worker scheduler：走訪器一邊發現檔案，一邊把單檔 copy 排給多個
/// worker，不會等上一個檔案完成後才處理下一個。`sync_channel` 限制尚未處理的工作量，
/// 因此即使目錄有數十萬個檔案，也不會一次把全部路徑保留在記憶體。
///
/// 參數：
/// - `source_dir: &Path`，來源目錄。
/// - `target_dir: &Path`，要建立的目標目錄。
/// - `progress: &mut F`，接收走訪器與 worker 回報的傳輸事件。
/// - `copy_file: C`，平台原生單檔複製函數；第三項是已知來源大小，第四項回報完成量。
///
/// 回傳：`io::Result<()>`；走訪、建立目錄或任一 worker 失敗時回傳第一個錯誤。
pub(crate) fn copy_dir_parallel_with_progress<F, C>(
    source_dir: &Path,
    target_dir: &Path,
    progress: &mut F,
    copy_file: C,
) -> io::Result<()>
where
    F: FnMut(TransferProgress),
    C: Fn(&Path, &Path, u64, &mut dyn FnMut(u64)) -> io::Result<()> + Sync,
{
    // 第一層目標一建立就通知 App 刷新目的 panel。這個 0 不是完成 byte 數，而是
    // 「目標已可見」訊號；真正百分比仍只由後續成功複製的 byte 累計。
    fs::create_dir(target_dir)?;
    progress(TransferProgress::TargetVisible);

    let (job_sender, job_receiver) = mpsc::sync_channel::<CopyFileJob>(COPY_FILE_WORKERS * 8);
    let job_receiver = Arc::new(Mutex::new(job_receiver));
    let (result_sender, result_receiver) = mpsc::channel::<CopyFileResult>();
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut traversal_error = None;

    thread::scope(|scope| {
        for _ in 0..COPY_FILE_WORKERS {
            let jobs = Arc::clone(&job_receiver);
            let results = result_sender.clone();
            let cancelled = Arc::clone(&cancelled);
            let copy_file = &copy_file;
            scope.spawn(move || {
                loop {
                    if cancelled.load(AtomicOrdering::Relaxed) {
                        break;
                    }
                    let job = match jobs.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => break,
                    };
                    let Ok(job) = job else {
                        break;
                    };
                    let mut file_progress = |increment| {
                        let _ = results.send(CopyFileResult::Progress(increment));
                    };
                    match copy_file(
                        &job.source_path,
                        &job.target_path,
                        job.expected_size,
                        &mut file_progress,
                    ) {
                        Ok(()) => {
                            if results.send(CopyFileResult::Copied).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            cancelled.store(true, AtomicOrdering::Relaxed);
                            let _ = results.send(CopyFileResult::Failed(error));
                            break;
                        }
                    }
                }
            });
        }
        // producer 不可保留一份永遠存活的 receiver；所有 worker 因錯誤退出後，
        // bounded sender 必須收到 disconnected，才能結束而不是永久等待空位。
        drop(job_receiver);
        // 走訪器必須和結果接收端同時執行。若在目前執行緒先完整走訪再讀 result，
        // 大型目錄會等掃描結束後才把 worker 進度交給 App，畫面因此長時間停在
        // RUNNING。獨立 producer 一邊建立目錄、一邊送出檔案工作，下面的 consumer
        // 則能從第一個檔案開始立即轉送進度。
        let producer_cancelled = Arc::clone(&cancelled);
        let producer_results = result_sender.clone();
        let producer = scope.spawn(move || {
            if let Err(error) = enqueue_copy_tree(
                source_dir,
                target_dir,
                &job_sender,
                &producer_results,
                producer_cancelled.as_ref(),
            ) {
                producer_cancelled.store(true, AtomicOrdering::Relaxed);
                let _ = producer_results.send(CopyFileResult::Failed(error));
            }
        });
        drop(result_sender);

        for result in result_receiver {
            match result {
                CopyFileResult::Discovered(size) => {
                    progress(TransferProgress::BytesDiscovered(size));
                }
                CopyFileResult::Progress(increment) => {
                    progress(TransferProgress::BytesCopied(increment));
                }
                CopyFileResult::Copied => {}
                CopyFileResult::Failed(error) if traversal_error.is_none() => {
                    traversal_error = Some(error);
                }
                CopyFileResult::Failed(_) => {}
            }
        }

        if producer.join().is_err() && traversal_error.is_none() {
            traversal_error = Some(io::Error::other("copy tree producer panicked"));
        }
    });

    match traversal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// 以廣度優先方式建立目標目錄，並把每個一般檔案送入有界 worker 佇列。
///
/// 參數：`source_dir` 與 `target_dir` 為目前走訪層級；`jobs` 是工作 sender；
/// `results` 回報新發現的 byte；`cancelled` 在任一 worker 失敗後停止繼續發現新工作。
/// 回傳：`io::Result<()>`；讀取來源或建立目標失敗時保留原始作業系統錯誤。
fn enqueue_copy_tree(
    source_dir: &Path,
    target_dir: &Path,
    jobs: &mpsc::SyncSender<CopyFileJob>,
    results: &mpsc::Sender<CopyFileResult>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let mut directories = VecDeque::from([(source_dir.to_path_buf(), target_dir.to_path_buf())]);
    while let Some((current_source, current_target)) = directories.pop_front() {
        for item in fs::read_dir(&current_source)? {
            if cancelled.load(AtomicOrdering::Relaxed) {
                return Ok(());
            }
            let item = item?;
            let source_path = item.path();
            let target_path = current_target.join(item.file_name());
            if item.file_type()?.is_dir() {
                fs::create_dir(&target_path)?;
                directories.push_back((source_path, target_path));
            } else {
                let expected_size = item.metadata()?.len();
                results
                    .send(CopyFileResult::Discovered(expected_size))
                    .map_err(|_| io::Error::other("copy result channel disconnected"))?;
                send_copy_job(
                    jobs,
                    cancelled,
                    CopyFileJob {
                        source_path,
                        target_path,
                        expected_size,
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// 將工作放入有界佇列，並在其他 worker 失敗後立即停止等待。
///
/// 阻塞式 `send` 會自然施加 backpressure，避免忙等耗盡 CPU；外層不保留額外 receiver，
/// 因此所有 worker 因錯誤停止時 channel 會斷線，走訪器可立即結束而不會永久卡住。
///
/// 參數：`jobs` 為有界佇列；`cancelled` 是共享取消狀態；`job` 是待排入的檔案工作。
/// 回傳：`io::Result<()>`；佇列斷線時回傳錯誤，已取消時不再排入並正常返回。
fn send_copy_job(
    jobs: &mpsc::SyncSender<CopyFileJob>,
    cancelled: &AtomicBool,
    job: CopyFileJob,
) -> io::Result<()> {
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Ok(());
    }
    match jobs.send(job) {
        Ok(()) => Ok(()),
        Err(_) if cancelled.load(AtomicOrdering::Relaxed) => Ok(()),
        Err(_) => Err(io::Error::other("copy worker queue disconnected")),
    }
}
