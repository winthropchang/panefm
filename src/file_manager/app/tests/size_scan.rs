use super::*;

#[test]
/// 驗證 `ms` 會在背景遞迴計算每個直接子目錄的檔案總 byte。
///
/// 保護目的：舊版只顯示直接子項目數量，而且同步讀取會拖慢 SMB；這個測試確保
/// linemode 立即啟動 worker、主執行緒可持續 poll，最後得到真正內容大小。
fn size_linemode_scans_recursive_directory_bytes_in_background() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    let sibling = dir.path().join("sibling");
    fs::create_dir_all(child.join("nested")).expect("nested");
    fs::create_dir(&sibling).expect("sibling");
    fs::write(child.join("one.bin"), vec![0u8; 7]).expect("first file");
    fs::write(child.join("nested/two.bin"), vec![0u8; 11]).expect("second file");
    fs::write(child.join(".hidden.bin"), vec![0u8; 5]).expect("hidden file");
    fs::write(child.join(".gitignore"), "ignored.bin\n").expect("ignore rules");
    fs::write(child.join("ignored.bin"), vec![0u8; 17]).expect("ignored file");
    fs::write(sibling.join("three.bin"), vec![0u8; 13]).expect("third file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size)
        .expect("enable size linemode");
    assert!(!app.directory_size_jobs.is_empty());
    assert!(
        app.panes[&1]
            .entries
            .iter()
            .filter(|entry| entry.is_dir)
            .all(|entry| entry.directory_size == Some(0) && !entry.directory_size_complete),
        "啟動背景掃描的當下就要顯示 ~0B，不可長時間停在省略號"
    );
    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_size_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == child)
        .expect("child entry");
    assert_eq!(entry.directory_size, Some(52));
    assert!(entry.directory_size_complete);
    let sibling_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == sibling)
        .expect("sibling entry");
    assert_eq!(sibling_entry.directory_size, Some(13));
    assert!(sibling_entry.directory_size_complete);
    assert!(app.directory_size_jobs.is_empty());
}

#[test]
/// 驗證目錄清單分批載入期間啟用 `ms`，完整清單抵達後會以全部目錄重啟容量掃描。
///
/// 保護目的：舊流程會保留只看過首批項目的同 cwd 掃描，導致稍後加入的目錄右側
/// 永久空白。測試刻意讓舊掃描留在工作表中，再送入含新目錄的完成事件，確保舊
/// worker 被取消、新目錄立即顯示部分值，且最後能取得正確容量。
fn completed_directory_load_restarts_size_scan_for_late_entries() {
    let dir = tempdir().expect("tempdir");
    let early = dir.path().join("early");
    let late = dir.path().join("晚到的目錄");
    fs::create_dir(&early).expect("early directory");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size)
        .expect("enable size linemode");
    let old_scan_cancelled = Arc::clone(&app.directory_size_jobs[&1].cancelled);

    fs::create_dir(&late).expect("late directory");
    fs::write(late.join("payload.bin"), vec![0u8; 31]).expect("late payload");
    let complete_entries = PaneState::new(dir.path().to_path_buf())
        .expect("reload complete entries")
        .entries;
    let (sender, receiver) = mpsc::channel();
    sender
        .send(DirectoryLoadEvent {
            pane_id: 1,
            cwd: dir.path().to_path_buf(),
            selected_path: None,
            result: Ok(super::DirectoryLoadProgress::Complete(complete_entries)),
        })
        .expect("complete directory event");
    app.directory_load_jobs.insert(
        1,
        DirectoryLoadJob {
            cwd: dir.path().to_path_buf(),
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    );

    app.poll_directory_load_jobs();

    assert!(old_scan_cancelled.load(Ordering::Relaxed));
    assert_eq!(app.directory_size_jobs[&1].cwd, dir.path());
    let late_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == late)
        .expect("late entry");
    assert_eq!(late_entry.directory_size, Some(0));
    assert!(!late_entry.directory_size_complete);

    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_size_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let late_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == late)
        .expect("late entry after scan");
    assert_eq!(late_entry.directory_size, Some(31));
    assert!(late_entry.directory_size_complete);
}

#[test]
/// 驗證 `ms` 啟用期間進入下一層目錄，會立即替新列表重新啟動容量掃描。
///
/// 保護目的：舊流程只在首次切換 linemode 時排程；使用 `l` 進入已顯示容量的目錄後，
/// 舊工作因 cwd 不符被取消，新目錄卻永久顯示 `...`。這個測試固定新工作必須綁定
/// 子目錄、子目錄中的資料夾立即顯示部分值，最後取得正確 byte 數。
fn size_linemode_restarts_scan_after_entering_directory() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    let nested = child.join("nested");
    fs::create_dir_all(&nested).expect("nested directory");
    fs::write(nested.join("payload.bin"), vec![0u8; 29]).expect("payload");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size)
        .expect("enable size linemode");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter child directory");

    assert_eq!(app.panes[&1].cwd, child);
    assert!(app.directory_load_jobs.contains_key(&1));
    assert!(
        app.panes[&1].entries.is_empty(),
        "首次進入時應先交還事件迴圈，不能同步等待目錄 I/O"
    );
    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_load_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(app.directory_load_jobs.is_empty());
    assert_eq!(app.directory_size_jobs[&1].cwd, child);
    let nested_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == nested)
        .expect("nested entry");
    assert_eq!(nested_entry.directory_size, Some(0));
    assert!(!nested_entry.directory_size_complete);

    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_size_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let nested_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == nested)
        .expect("nested entry after scan");
    assert_eq!(nested_entry.directory_size, Some(29));
    assert!(nested_entry.directory_size_complete);
    assert!(app.directory_size_jobs.is_empty());

    // 回到父目錄再立刻重進時，已完成的清單必須在按鍵處直接由快取恢復；背景
    // refresh 仍會校正磁碟最新狀態，但畫面不能再次短暫變成空白。
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("return to parent");
    assert!(
        app.panes[&1]
            .entries
            .iter()
            .any(|entry| entry.path == child)
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("re-enter cached child");
    assert!(
        app.panes[&1]
            .entries
            .iter()
            .any(|entry| entry.path == nested)
    );
}

#[test]
/// 驗證 `ms` 背景容量在 200ms 邊界才傳回下一份部分結果。
///
/// 保護目的：太慢會讓大目錄長時間看不到變化，太快則會對每個檔案發送事件並
/// 影響鍵盤操作；測試固定 199ms 不更新、200ms 立即更新的規格。
fn directory_size_partial_updates_use_two_hundred_millisecond_interval() {
    assert!(!super::should_report_directory_size(199, 0));
    assert!(super::should_report_directory_size(200, 0));
    assert!(!super::should_report_directory_size(399, 200));
    assert!(super::should_report_directory_size(400, 200));
}

#[test]
/// 驗證離開 size linemode 會取消該 panel 的背景掃描。
///
/// 保護目的：使用者可能快速切換模式或目錄；若舊 worker 不取消，會持續讀磁碟／SMB
/// 並把晚到結果寫進新的畫面。
fn leaving_size_linemode_cancels_panel_scan() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("child")).expect("child");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size).expect("enable size");
    app.apply_line_mode(1, LineMode::None)
        .expect("disable size");

    assert!(app.directory_size_jobs.is_empty());
}
