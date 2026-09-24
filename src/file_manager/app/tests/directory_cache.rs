use std::time::Duration;

use super::*;

#[test]
/// 驗證在 TTL 內重訪目錄時，直接命中快取（0ms 瞬間就緒），完全跳過背景 Worker 的建立與硬碟讀取。
fn directory_cache_instant_hit_skips_background_worker_within_ttl() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child_folder");
    fs::create_dir_all(&child).expect("create child");
    fs::write(child.join("file_1.txt"), b"1").expect("write 1");
    fs::write(child.join("file_2.txt"), b"2").expect("write 2");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 首次進入 child 目錄，此時應啟動背景載入工作
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter child");
    assert_eq!(app.panes[&1].cwd, child);

    // 等待背景載入完成並寫入快取
    for _ in 0..100 {
        app.poll_background_tasks();
        if !app.directory_load_jobs.contains_key(&1) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !app.directory_load_jobs.contains_key(&1),
        "背景載入必須已完成"
    );
    assert_eq!(app.panes[&1].entries.len(), 2);

    // 驗證快取時間戳與新鮮度
    assert!(
        app.is_directory_cache_fresh(&child, Duration::from_secs(5)),
        "剛載入完成之目錄必須處於新鮮快取期內"
    );

    // 按 h 退回父目錄
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("return to parent");
    assert_eq!(app.panes[&1].cwd, dir.path());

    // 再次按 l 進入 child 目錄：此時處於 5 秒新鮮期內，必須直接 0ms 快取命中，不產生任何背景載入工作！
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("re-enter child");
    assert_eq!(app.panes[&1].cwd, child);

    assert!(
        !app.directory_load_jobs.contains_key(&1),
        "0ms 快取命中必須完全跳過背景 Worker 建立，達到零磁碟 I/O 與零等待"
    );
    assert_eq!(
        app.panes[&1].entries.len(),
        2,
        "快取命中後檔案清單必須完整存在"
    );
}

#[test]
/// 驗證退出目錄時自動記憶游標，並在主動讓快取失效時正確清理。
fn directory_cache_invalidation_and_cursor_memory() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let test_dir = dir.path().join("cached_dir");
    fs::create_dir_all(&test_dir).expect("create dir");

    let test_entries = vec![crate::file_manager::entry::FileEntry {
        name: "test.txt".to_string(),
        path: test_dir.join("test.txt"),
        is_dir: false,
        size: 10,
        is_sparse_empty: false,
        directory_size: None,
        directory_size_complete: false,
        modified: std::time::SystemTime::UNIX_EPOCH,
        created: std::time::SystemTime::UNIX_EPOCH,
        readonly: false,
        unix_mode: None,
    }];

    // 存入快取
    app.store_directory_cache(test_dir.clone(), test_entries);
    assert!(app.is_directory_cache_fresh(&test_dir, Duration::from_secs(5)));

    // 0 逾時即視為過期
    assert!(!app.is_directory_cache_fresh(&test_dir, Duration::ZERO));

    // 主動失效清理
    app.invalidate_directory_cache(&test_dir);
    assert!(
        !app.is_directory_cache_fresh(&test_dir, Duration::from_secs(5)),
        "主動失效後快取必須清除"
    );
    assert!(
        !app.directory_entry_cache.contains_key(&test_dir),
        "entry 快取必須已移除"
    );
    assert!(
        !app.directory_cache_timestamps.contains_key(&test_dir),
        "時間戳必須已移除"
    );
}

#[test]
/// 驗證外部接管（pending launch / fzf）狀態能被即時偵測以中斷事件折疊。
fn external_takeover_detection() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    assert!(!app.has_pending_external_takeover());

    app.pending_launch = Some(QueuedLaunch {
        task_id: 1,
        launch: LaunchSpec {
            program: "vim".to_string(),
            args: vec![],
            mode: LaunchMode::TerminalBlocking,
        },
    });

    assert!(
        app.has_pending_external_takeover(),
        "有待啟動之外部程式時必須判定為外部接管"
    );
}
