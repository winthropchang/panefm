use super::*;

#[test]
/// 驗證交易式複製成功後只留下正式檔名，不會把內部暫存檔暴露在目標目錄。
///
/// 參數：無。
/// 回傳：無；若正式內容錯誤或 `.panefm-transfer-*` 暫存路徑殘留則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn transactional_copy_commits_complete_file_without_temp_residue() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target_dir = dir.path().join("target");
    let target = target_dir.join("source.zip");
    fs::create_dir(&target_dir).expect("target dir");
    fs::write(&source, b"complete zip bytes").expect("source");

    super::copy_path_transactional(&source, &target, false).expect("transactional copy");

    assert_eq!(
        fs::read(&target).expect("target content"),
        b"complete zip bytes"
    );
    assert!(!directory_has_transfer_temp(&target_dir));
}

#[test]
/// 模擬 SMB 寫入部分內容後失敗，驗證正式檔名與暫存檔都不會殘留。
///
/// 參數：無。
/// 回傳：無；若失敗後仍佔用正式檔名，Finder 下一次複製可能被迫產生 `copy` 名稱。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn transactional_copy_failure_removes_partial_staged_file() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target = dir.path().join("target.zip");
    fs::write(&source, b"complete zip bytes").expect("source");

    let result = copy_path_transactional_with(&source, &target, false, |_, staged| {
        fs::write(staged, b"partial")?;
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "simulated SMB disconnect",
        ))
    });

    assert_eq!(
        result.expect_err("copy must fail").kind(),
        std::io::ErrorKind::ConnectionReset
    );
    assert!(!target.exists());
    assert!(!directory_has_transfer_temp(dir.path()));
}

#[test]
/// 模擬一般 SMB 複製直接寫入部分正式內容後失敗，驗證該檔名會被立即清除。
///
/// 參數：無。
/// 回傳：無；若部分檔案仍存在，Finder 後續複製就可能自動改成 `copy` 名稱。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn direct_copy_failure_removes_partial_target() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target = dir.path().join("target.zip");
    fs::write(&source, b"complete zip bytes").expect("source");

    let result = copy_path_direct_with_cleanup(&source, &target, |_, target| {
        fs::write(target, b"partial")?;
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "simulated SMB disconnect",
        ))
    });

    assert_eq!(
        result.expect_err("copy must fail").kind(),
        std::io::ErrorKind::ConnectionReset
    );
    assert!(!target.exists());
}

#[test]
/// 驗證跨平台原生 copy 會完整複製內容，並回報正確 byte 數。
///
/// 參數：無。
/// 回傳：無；若目標內容或進度累計與來源不同，測試失敗。
/// 保護目的：本機、Windows UNC 與 macOS 掛載路徑統一改用原生 API 後，仍須驗證
/// 回報進度與目標內容，避免把不完整檔案當成成功。
fn native_copy_verifies_content_and_reports_progress() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.bin");
    let target = dir.path().join("target.bin");
    let bytes = b"native local copy content";
    fs::write(&source, bytes).expect("source");
    let mut completed = 0u64;

    copy_file_native_with_progress(&source, &target, &mut |increment| {
        completed = completed.saturating_add(increment);
    })
    .expect("native copy");

    assert_eq!(completed, bytes.len() as u64);
    assert_eq!(fs::read(&target).expect("target"), bytes);
}

#[test]
/// 驗證平台原生 copy 回傳「不支援」且留下 0-byte 目標時，會清理該目標並改走
/// 分塊串流，最後仍得到完整內容與正確進度。
///
/// 保護目的：部分 macOS／Windows SMB server 不支援原生 copy 加速；PaneFM 過去
/// 會直接顯示失敗或留下 0 KB ZIP。這個測試確保 fallback 是跨平台傳檔的必要
/// 正確性路徑，而不是只在特定公司環境手動驗證。
fn unsupported_native_copy_falls_back_to_verified_streaming_copy() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target = dir.path().join("target.zip");
    let payload = vec![0x5a; 2 * 1024 * 1024 + 37];
    fs::write(&source, &payload).expect("source");
    let mut reported = 0u64;

    copy_file_with_native_fallback(
        &source,
        &target,
        &mut |increment| reported = reported.saturating_add(increment),
        |_, target| {
            File::create(target).expect("simulated partial target");
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "native copy unsupported by share",
            ))
        },
    )
    .expect("stream fallback");

    assert_eq!(reported, payload.len() as u64);
    assert_eq!(fs::read(&target).expect("target bytes"), payload);
}

#[test]
/// 模擬原生 copy（如 Windows CopyFileExW 在 macOS SMB）宣稱完成但目的檔只有 0-byte。
/// 驗證檔案引擎會自動辨識大小不符（UnexpectedEof）、清理 0-byte 殘留，並無縫切換到串流複製。
fn native_copy_zero_byte_mismatch_triggers_stream_fallback() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target = dir.path().join("target.zip");
    let payload = vec![0x42; 512 * 1024];
    fs::write(&source, &payload).expect("source");
    let mut reported = 0u64;

    copy_file_with_native_fallback(
        &source,
        &target,
        &mut |increment| reported = reported.saturating_add(increment),
        |_, target| {
            // 模擬只建立 0-byte 檔案卻回傳全部寫入大小的 Win32 SMB bug
            File::create(target).expect("0-byte partial target");
            Ok(payload.len() as u64)
        },
    )
    .expect("stream fallback for 0-byte mismatch");

    assert_eq!(reported, payload.len() as u64);
    assert_eq!(fs::read(&target).expect("target bytes"), payload);
}

#[test]
#[cfg(unix)]
/// 驗證 macOS 上的 os error 102 (EOPNOTSUPP, Operation not supported on socket)
/// 能正確觸發串流 fallback。
fn native_copy_macos_os_error_102_triggers_stream_fallback() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.txt");
    let target = dir.path().join("target.txt");
    fs::write(&source, b"test content").expect("source");
    let mut reported = 0u64;

    copy_file_with_native_fallback(
        &source,
        &target,
        &mut |increment| reported = reported.saturating_add(increment),
        |_, target| {
            File::create(target).expect("simulated partial target");
            Err(io::Error::from_raw_os_error(102))
        },
    )
    .expect("stream fallback for os error 102");

    assert_eq!(reported, 12);
    assert_eq!(fs::read(&target).expect("target bytes"), b"test content");
}

#[test]
/// 驗證原生 copy 的一般權限錯誤不會被錯誤地改成串流重試。
///
/// 保護目的：fallback 只能處理「API 不支援」；若權限、磁碟空間或連線本身失敗，
/// 必須保留原始 OS error，避免第二次寫入掩蓋真正原因或造成額外 partial 檔案。
fn native_copy_permission_error_does_not_trigger_stream_fallback() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.bin");
    let target = dir.path().join("target.bin");
    fs::write(&source, b"protected").expect("source");
    let mut reported = 0u64;

    let error = copy_file_with_native_fallback(
        &source,
        &target,
        &mut |increment| reported = reported.saturating_add(increment),
        |_, _| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "permission denied by share",
            ))
        },
    )
    .expect_err("permission error must remain visible");

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(reported, 0);
    assert!(!target.exists());
}

#[test]
/// 驗證原生 copy 尚未完成時，metadata 輪詢就能先回報部分進度。
///
/// 參數：無。
/// 回傳：無；若 progress 只能在整個 copy 結束後一次跳到 100%，或累計 byte 不等於
/// 來源大小，測試就會失敗。
/// 保護目的：PaneFM 改用平台原生 copy 後，不能為了顯示百分比退回手寫串流；
/// 此測試保護「copy 引擎與進度 metadata 輪詢互相獨立」的核心設計。
fn native_copy_polls_destination_metadata_before_completion() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.bin");
    let target = dir.path().join("target.bin");
    let bytes = b"12345678";
    fs::write(&source, bytes).expect("source");
    let mut increments = Vec::new();

    copy_file_native_with_progress_using(
        &source,
        &target,
        &mut |increment| increments.push(increment),
        |source, target| {
            let source_bytes = fs::read(source)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            output.write_all(&source_bytes[..4])?;
            output.flush()?;
            thread::sleep(Duration::from_millis(450));
            output.write_all(&source_bytes[4..])?;
            output.flush()?;
            Ok(source_bytes.len() as u64)
        },
    )
    .expect("progressive native copy");

    assert!(increments.len() >= 2, "應在完成前至少回報一次部分進度");
    assert_eq!(increments.iter().sum::<u64>(), bytes.len() as u64);
    assert_eq!(fs::read(&target).expect("target"), bytes);
}

#[test]
/// 驗證快速完成的小檔案 copy 不會因進度輪詢而被強制延遲 200ms。
///
/// 參數：無。
/// 回傳：無；四個各耗時約 10ms 的 copy 若累計超過 500ms，測試失敗。
/// 保護目的：舊實作使用 `is_finished` 後固定 sleep 200ms，導致每個小檔都可能
/// 多等一次完整輪詢週期；只有三個 worker 時，數千個檔案會被拖到數十分鐘。
fn completed_small_file_copy_wakes_progress_waiter_immediately() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.bin");
    fs::write(&source, b"small file").expect("source");
    let started = Instant::now();

    for index in 0..4 {
        let target = dir.path().join(format!("target-{index}.bin"));
        copy_file_native_with_progress_using(&source, &target, &mut |_| {}, |source, target| {
            thread::sleep(Duration::from_millis(10));
            fs::copy(source, target)
        })
        .expect("small native copy");
    }

    assert!(
        started.elapsed() < Duration::from_millis(500),
        "small copies were delayed by progress polling: {:?}",
        started.elapsed()
    );
}

#[test]
/// 驗證同磁碟移動資料夾會直接使用 rename，成功前不遞迴掃描內容或假造 byte 進度。
///
/// 參數：無。
/// 回傳：無；若來源仍存在、目標內容遺失或 rename 路徑回報 byte 進度則測試失敗。
/// 保護目的：大型目錄本可由檔案系統瞬間改名，過去卻先呼叫 `path_content_size`
/// 走訪整棵樹，導致 cut/paste 平白停頓數秒。
fn same_device_directory_move_renames_without_prescanning_for_progress() {
    let dir = tempdir().expect("tempdir");
    let source_parent = dir.path().join("source-parent");
    let target_parent = dir.path().join("target-parent");
    let source = source_parent.join("build-output");
    fs::create_dir_all(&source).expect("source directory");
    fs::create_dir(&target_parent).expect("target parent");
    let bin_path = source.join("artifact.bin");
    {
        use std::io::Write;
        let mut file = fs::File::create(&bin_path).expect("source file");
        file.write_all(b"artifact").expect("write artifact");
        file.sync_all().expect("sync artifact");
    }
    let mut progress_calls = Vec::new();

    let outcome = PaneState::move_path_to_dir_with_history_progress(
        &source,
        &target_parent,
        false,
        &mut |increment| progress_calls.push(increment),
    )
    .expect("same-device move");

    assert!(!source.exists());
    assert_eq!(
        fs::read(outcome.target_path.join("artifact.bin")).expect("moved content"),
        b"artifact"
    );
    assert!(progress_calls.is_empty());
}

#[test]
/// 模擬 SMB 不支援 rename，驗證 move 會改用原生 copy 後刪除來源。
///
/// 參數：無。
/// 回傳：無；若 rename 錯誤直接中止、目標內容不完整、來源提早刪除或進度不正確，
/// 測試就會失敗。
/// 保護目的：Windows UNC 與 macOS 掛載 share 可能拒絕 rename，但仍允許 copy；move
/// 不可因此失效，而且只有完整 copy 通過大小驗證後才可移除來源。
fn unsupported_rename_falls_back_to_copy_then_removes_source() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source = source_dir.join("archive.zip");
    let bytes = b"complete archive bytes";
    fs::write(&source, bytes).expect("source file");
    let mut completed = 0u64;

    let outcome = PaneState::move_path_to_dir_with_history_progress_using_rename(
        &source,
        &target_dir,
        false,
        &mut |event| {
            if let TransferProgress::BytesCopied(increment) = event {
                completed = completed.saturating_add(increment);
            }
        },
        |_, _| Err(std::io::Error::from_raw_os_error(45)),
    )
    .expect("copy fallback");

    assert!(!source.exists());
    assert_eq!(fs::read(&outcome.target_path).expect("target"), bytes);
    assert_eq!(completed, bytes.len() as u64);
}

#[test]
/// 驗證同步 move 遇到跨裝置錯誤 (os error 18 EXDEV) 時，會降級為 copy 後刪除來源。
fn move_path_into_dir_falls_back_to_copy_on_cross_device_error() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source = source_dir.join("payload.zip");
    let bytes = b"payload bytes for cross device move";
    fs::write(&source, bytes).expect("source file");

    let outcome =
        PaneState::move_path_to_dir_with_history_using_rename(&source, &target_dir, |_, _| {
            Err(std::io::Error::from_raw_os_error(18))
        })
        .expect("cross device fallback");

    assert!(!source.exists());
    assert_eq!(fs::read(&outcome.target_path).expect("target"), bytes);
}

#[test]
/// 驗證目錄複製會同時執行多個單檔工作，而不是退化成逐檔等待。
///
/// 參數：無。
/// 回傳：無；若目標未先變成可見、最高同時工作數小於 2、內容遺失或進度錯誤，
/// 測試就會失敗。
/// 保護目的：PaneFM 過去複製包含大量小檔案的 `target` 目錄時只用一條 worker，
/// 小檔案處理會大幅變慢；此測試防止後續重構再次移除並行 scheduler。
fn directory_copy_uses_multiple_workers_and_preserves_content() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::create_dir(&source).expect("source dir");
    for index in 0..9 {
        fs::write(
            source.join(format!("file-{index}.txt")),
            format!("data-{index}"),
        )
        .expect("source file");
    }

    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let active_for_copy = Arc::clone(&active);
    let maximum_for_copy = Arc::clone(&maximum);
    let mut completed = 0u64;
    let mut target_became_visible = false;
    copy_dir_parallel_with_progress(
        &source,
        &target,
        &mut |event| match event {
            TransferProgress::TargetVisible => target_became_visible = target.is_dir(),
            TransferProgress::BytesCopied(increment) => {
                completed = completed.saturating_add(increment);
            }
            TransferProgress::BytesDiscovered(_) => {}
        },
        move |from, to, _, progress| {
            let running = active_for_copy.fetch_add(1, Ordering::SeqCst) + 1;
            maximum_for_copy.fetch_max(running, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(15));
            #[allow(clippy::redundant_closure)]
            let result = fs::copy(from, to).map(|copied| progress(copied));
            active_for_copy.fetch_sub(1, Ordering::SeqCst);
            result
        },
    )
    .expect("parallel directory copy");

    assert!(target_became_visible);
    assert!(maximum.load(Ordering::SeqCst) >= 2);
    let expected_bytes = (0..9)
        .map(|index| format!("data-{index}").len() as u64)
        .sum::<u64>();
    assert_eq!(completed, expected_bytes);
    for index in 0..9 {
        assert_eq!(
            fs::read_to_string(target.join(format!("file-{index}.txt"))).expect("target file"),
            format!("data-{index}")
        );
    }
}

#[test]
/// 驗證複製一般專案目錄時會完整包含巢狀 `.tfm`，不會擅自略過使用者資料。
///
/// 保護目的：即使 `.tfm` 可能很大，傳輸引擎仍必須靠高效率排程解決，不可用檔名
/// 規則刪減複製內容，否則副本和來源不一致且可能遺失使用者需要的資料。
fn directory_copy_preserves_nested_internal_state() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("project");
    let target = dir.path().join("project-copy");
    fs::create_dir_all(source.join(".tfm/trash/items")).expect("internal state");
    fs::write(source.join("source.rs"), b"project source").expect("project file");
    fs::write(source.join(".tfm/trash/items/large.bin"), b"internal trash").expect("trash file");

    copy_dir_parallel_with_progress(&source, &target, &mut |_| {}, |from, to, _, progress| {
        let copied = fs::copy(from, to)?;
        progress(copied);
        Ok(())
    })
    .expect("copy project");

    assert_eq!(
        fs::read(target.join("source.rs")).expect("copied source"),
        b"project source"
    );
    assert_eq!(
        fs::read(target.join(".tfm/trash/items/large.bin")).expect("nested state copy"),
        b"internal trash"
    );
}

#[test]
/// 驗證 producer 尚在排入大型目錄內容時，已完成檔案的進度就會立刻送到呼叫端。
///
/// 參數：無。
/// 回傳：無；若第一筆進度必須等大部分工作都排完或複製完才出現，測試失敗。
/// 保護目的：舊流程在目前執行緒完整呼叫 `enqueue_copy_tree` 後才讀 result channel，
/// 大型 `target` 目錄會長時間只顯示 RUNNING。此測試確保走訪 producer 與進度
/// consumer 保持真正並行，第一批檔案完成時 UI 就有資料可更新。
fn directory_copy_reports_progress_while_producer_is_still_working() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::create_dir(&source).expect("source dir");
    for index in 0..100 {
        fs::write(source.join(format!("file-{index}.txt")), b"data").expect("source file");
    }

    let finished = Arc::new(AtomicUsize::new(0));
    let finished_for_copy = Arc::clone(&finished);
    let mut finished_when_first_progress_arrived = None;
    copy_dir_parallel_with_progress(
        &source,
        &target,
        &mut |event| {
            if matches!(event, TransferProgress::BytesCopied(increment) if increment > 0)
                && finished_when_first_progress_arrived.is_none()
            {
                finished_when_first_progress_arrived = Some(finished.load(Ordering::SeqCst));
            }
        },
        move |from, to, _, progress| {
            thread::sleep(Duration::from_millis(5));
            let copied = fs::copy(from, to)?;
            finished_for_copy.fetch_add(1, Ordering::SeqCst);
            progress(copied);
            Ok(())
        },
    )
    .expect("parallel directory copy");

    assert!(
        finished_when_first_progress_arrived.is_some_and(|count| count < 20),
        "first progress arrived too late: {finished_when_first_progress_arrived:?}"
    );
    assert_eq!(finished.load(Ordering::SeqCst), 100);
}

#[test]
/// 驗證任一並行 worker 失敗後，有界工作佇列會停止接受新檔案並回傳錯誤。
///
/// 參數：無。
/// 回傳：無；若模擬 I/O 錯誤被忽略或函數無法結束，測試失敗。
/// 保護目的：大量 SMB 工作塞滿佇列時，若 server 斷線，阻塞式 send 曾可能永遠
/// 等待已停止的 worker；此測試保護錯誤取消路徑，避免背景 task 永久停在 RUNNING。
fn parallel_directory_copy_stops_when_a_worker_fails() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::create_dir(&source).expect("source dir");
    for index in 0..40 {
        fs::write(source.join(format!("file-{index}.txt")), b"data").expect("source file");
    }

    let result = copy_dir_parallel_with_progress(&source, &target, &mut |_| {}, |_, _, _, _| {
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "simulated SMB disconnect",
        ))
    });

    assert_eq!(
        result.expect_err("copy must fail").kind(),
        std::io::ErrorKind::ConnectionReset
    );
}

#[test]
/// 驗證可靠複製會完整寫入資料，且回傳前已關閉目標檔案 handle。
///
/// 參數：無。
/// 回傳：無；若內容不一致或 Windows 因 handle 尚未關閉而無法改名，測試就會失敗。
/// 保護目的：避免未來移除平台 copy 後的同步與大小驗證，導致 SMB 尚未完成就被 UI 視為成功。
fn verified_file_copy_is_complete_and_closed_before_returning() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target = dir.path().join("target.zip");
    let renamed = dir.path().join("renamed.zip");
    fs::write(&source, b"complete zip bytes").expect("source");

    copy_file_and_verify(&source, &target).expect("verified copy");
    fs::rename(&target, &renamed).expect("target handle must be closed");

    assert_eq!(
        fs::read(&renamed).expect("renamed target content"),
        b"complete zip bytes"
    );
}

#[test]
/// 模擬 SMB 複製 API 宣稱已寫入完整 byte 數，實際目的檔卻仍是 0-byte。
///
/// 參數：無。
/// 回傳：無；若驗證流程錯把 0-byte 目的檔當成成功，或失敗後仍留下正式檔名，
/// 測試就會失敗。
/// 保護目的：重現公司 Windows 傳到 SMB 後，PaneFM 看得到檔名但 macOS 端讀到
/// 0 KB 的問題，確保 UI 不會回報成功且 partial target 一定會被清除。
fn smb_zero_byte_copy_is_rejected_and_partial_target_is_removed() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target = dir.path().join("target.zip");
    let source_bytes = b"zip content that must reach the SMB server";
    fs::write(&source, source_bytes).expect("source");

    let result = copy_path_direct_with_cleanup(&source, &target, |source, target| {
        copy_file_and_verify_with(source, target, |_, target| {
            fs::write(target, [])?;
            Ok(source_bytes.len() as u64)
        })
    });

    let error = result.expect_err("0-byte SMB target must fail verification");
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    assert!(error.to_string().contains("stored 0"));
    assert!(!target.exists(), "failed target must not occupy its name");
}

#[test]
/// 模擬資料夾傳輸建立數個 partial 子項目後失敗，驗證整棵未完成目錄會被清除。
///
/// 參數：無。
/// 回傳：無；若目標資料夾或其中任何 partial 檔案仍存在，測試就會失敗。
/// 保護目的：避免單檔清理正常、但 SMB 資料夾貼上失敗時仍留下殘缺目錄佔用名稱。
fn direct_directory_copy_failure_removes_partial_tree() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source");
    let target = dir.path().join("target");
    fs::create_dir(&source).expect("source dir");

    let result = copy_path_direct_with_cleanup(&source, &target, |_, target| {
        fs::create_dir_all(target.join("nested"))?;
        fs::write(target.join("nested").join("partial.zip"), b"partial")?;
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "simulated SMB disconnect",
        ))
    });

    assert_eq!(
        result.expect_err("directory copy must fail").kind(),
        std::io::ErrorKind::ConnectionReset
    );
    assert!(!target.exists());
}

#[test]
/// 驗證覆蓋傳輸在新內容尚未完成前不會刪除既有目標檔案。
///
/// 參數：無。
/// 回傳：無；若模擬傳輸失敗後舊內容被刪除或改寫則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn transactional_overwrite_failure_preserves_existing_target() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("source.zip");
    let target = dir.path().join("target.zip");
    fs::write(&source, b"new content").expect("source");
    fs::write(&target, b"existing content").expect("existing target");

    let result = copy_path_transactional_with(&source, &target, true, |_, staged| {
        fs::write(staged, b"partial new content")?;
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "simulated SMB timeout",
        ))
    });

    assert!(result.is_err());
    assert_eq!(
        fs::read(&target).expect("preserved target"),
        b"existing content"
    );
    assert!(!directory_has_transfer_temp(dir.path()));
}

/// 檢查測試目錄中是否仍存在交易式複製的內部暫存路徑。
///
/// 參數：`path: &Path`，要掃描的測試目錄。
/// 回傳：`bool`；找到 `.panefm-transfer-*` 名稱時回傳 `true`。
fn directory_has_transfer_temp(path: &std::path::Path) -> bool {
    fs::read_dir(path)
        .expect("read test directory")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".panefm-transfer-")
        })
}

#[test]
/// 驗證 pane 的單檔與目錄複製支援 CoW 快速克隆，並在跨檔案系統／不支援情境下平滑降級。
fn cow_and_fallback_copy_file_and_directory_in_pane() {
    let dir = tempdir().expect("tempdir");
    let src_file = dir.path().join("source.dat");
    let dst_file = dir.path().join("target.dat");
    let payload = vec![0x42u8; 1024 * 64];
    fs::write(&src_file, &payload).expect("write src");

    // 1. 單檔複製
    copy_file_and_verify(&src_file, &dst_file).expect("copy file");
    assert!(dst_file.exists());
    assert_eq!(fs::read(&dst_file).expect("read dst"), payload);

    // 2. 資料夾遞迴複製
    let src_dir = dir.path().join("source_dir");
    let dst_dir = dir.path().join("target_dir");
    fs::create_dir_all(&src_dir).expect("create src dir");
    fs::write(src_dir.join("nested.txt"), b"nested content").expect("write nested");

    copy_dir_recursive(&src_dir, &dst_dir).expect("copy dir");
    assert!(dst_dir.is_dir());
    assert_eq!(
        fs::read_to_string(dst_dir.join("nested.txt")).expect("read nested"),
        "nested content"
    );

    // 3. 帶進度回報的資料夾複製
    let dst_dir2 = dir.path().join("target_dir2");
    let mut discovered = 0u64;
    let mut copied = 0u64;
    copy_dir_recursive_with_progress(&src_dir, &dst_dir2, &mut |progress| match progress {
        TransferProgress::BytesDiscovered(b) => discovered = discovered.saturating_add(b),
        TransferProgress::BytesCopied(b) => copied = copied.saturating_add(b),
        _ => {}
    })
    .expect("copy dir with progress");

    assert!(dst_dir2.is_dir());
    assert_eq!(
        fs::read_to_string(dst_dir2.join("nested.txt")).expect("read nested2"),
        "nested content"
    );
    assert!(discovered > 0);
    assert!(copied > 0);
}

#[test]
/// 驗證落盤錯誤判斷函式能精確辨識 macOS SMB / NFS / 虛擬檔案系統常見的非致命不支援錯誤，
/// 特別是 macOS smbfs 的 ENOTSUP (os error 45) 與 EOPNOTSUPP (os error 102)。
fn sync_unsupported_error_detects_network_and_virtual_fs_errors() {
    let unsupported_err = io::Error::new(io::ErrorKind::Unsupported, "not supported");
    assert!(is_sync_unsupported_error(&unsupported_err));

    #[cfg(unix)]
    {
        // macOS ENOTSUP = 45, EOPNOTSUPP = 102
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(45)));
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(
            102
        )));
        // Linux ENOTSUP / EOPNOTSUPP = 95
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(95)));
        // EINVAL = 22, ENOSYS = 78/38, EPERM = 1
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(22)));
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(38)));
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(78)));
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(1)));
    }

    #[cfg(windows)]
    {
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(1)));
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(50)));
        assert!(is_sync_unsupported_error(&io::Error::from_raw_os_error(
            120
        )));
    }

    let perm_err = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
    assert!(!is_sync_unsupported_error(&perm_err));

    let not_found_err = io::Error::new(io::ErrorKind::NotFound, "not found");
    assert!(!is_sync_unsupported_error(&not_found_err));
}

#[test]
/// 驗證 sync_target_file 在常態檔案能正常完成 flush / sync 操作。
fn sync_target_file_succeeds_on_real_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("sync_test.txt");
    let file = File::create(&file_path).expect("create file");
    assert!(sync_target_file(&file).is_ok());
}

#[test]
/// 驗證單一檔案複製時，能正確發送 TargetVisible 事件以通知目的端 pane 刷新。
/// 保護目的：避免單檔複製遺漏 TargetVisible，導致背景傳輸期間目的端檔案清單無法呈現進度。
fn single_file_copy_emits_target_visible_progress() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("single.dat");
    let target_dir = dir.path().join("target");
    fs::create_dir(&target_dir).expect("target dir");
    fs::write(&source, b"hello single file copy").expect("write source");

    let mut target_became_visible = false;
    let mut bytes_copied = 0u64;
    PaneState::copy_path_to_dir_with_history_progress(&source, &target_dir, false, &mut |event| {
        match event {
            TransferProgress::TargetVisible => target_became_visible = true,
            TransferProgress::BytesCopied(n) => bytes_copied += n,
            TransferProgress::BytesDiscovered(_) => {}
        }
    })
    .expect("copy single file");

    assert!(target_became_visible);
    assert_eq!(bytes_copied, 22);
    assert!(target_dir.join("single.dat").exists());
}
