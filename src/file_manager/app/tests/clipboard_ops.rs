use super::*;

#[test]
/// 驗證 Yank 面板打開後，按 `p` 會開啟預填 `:copy-panel ` 的命令列。
fn app_copy_shortcut_yp_opens_copy_panel_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::YankPicker { pane_id: 1 })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("trigger copy-panel command");

    assert!(app.pending_action.is_none());
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "copy-panel ");
}

#[test]
/// 驗證 Yank 面板打開後，按 `y` (yy) 會將選取項目複製到剪貼簿。
fn app_copy_shortcut_yy_copies_selected_to_clipboard() {
    let dir = tempdir().expect("tempdir");
    let file = dir.path().join("sample.txt");
    fs::write(&file, "content").expect("write");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("trigger yy");

    assert!(app.pending_action.is_none());
    assert!(app.clipboard.is_some());
    let clipboard = app.clipboard.as_ref().unwrap();
    assert_eq!(clipboard.operation, ClipboardOperation::Copy);
    assert_eq!(clipboard.entries.len(), 1);
    assert_eq!(clipboard.entries[0].source_path, file);
}

#[test]
/// 驗證 Yank 面板打開後，按數字 `2` 會直接將檔案複製到 Pane 2，且來源檔案依然保留。
fn app_copy_shortcut_digit_copies_directly_to_target_pane() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("sample.txt");
    fs::write(&source_file, "copy sample").expect("write");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .expect("trigger copy to pane 2");

    assert!(app.pending_action.is_none());
    assert!(source_file.exists());
    assert!(target_dir.join("sample.txt").exists());
    assert_eq!(
        app.status,
        format!("copied 1 item -> {}", target_dir.display())
    );
}

#[test]
/// 驗證 Yank 面板打開後，按 Esc 或 q 可以正常取消並返回 normal mode。
fn app_copy_shortcut_cancel_returns_to_normal_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::YankPicker { .. })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 `y` 複製後可以用 `p` 把檔案貼到另一個目錄，且來源會保留。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_and_paste_preserves_source_file() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("alpha.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy yy");

    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.operation),
        Some(ClipboardOperation::Copy)
    );
    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.entries.len()),
        Some(1)
    );

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("paste");

    assert!(source_file.exists());
    assert!(target_dir.join("alpha.txt").exists());
    assert_eq!(app.status, "pasted copy: 1 item");
}

#[test]
/// 驗證大型檔案貼上會先建立背景 task，而不是在按下 `p` 時同步完成。
///
/// 保護目的：大 ZIP 或 SMB 傳輸可能需要數分鐘；測試以 sparse 8 MiB 檔觸發
/// 背景門檻，確認主處理函式先返回、完成事件仍會刷新列表並建立 Undo 歷史。
fn app_large_paste_runs_as_background_task_and_records_completion() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("large.zip");
    fs::File::create(&source_file)
        .expect("large source")
        .set_len(BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
        .expect("size source");

    let mut app = App::new(source_dir, default_loaded_config()).expect("app");
    app.copy_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.paste_into_focused_pane().expect("queue paste");

    assert!(!app.file_job_receivers.is_empty());
    assert!(app.status.contains("in background"));
    wait_for_file_jobs(&mut app);

    assert_eq!(
        fs::metadata(target_dir.join("large.zip"))
            .expect("target metadata")
            .len(),
        BACKGROUND_FILE_JOB_THRESHOLD_BYTES
    );
    assert_eq!(app.status, "pasted copy: 1 item");
    assert_eq!(app.operation_history.len(), 1);
    assert!(matches!(
        app.task_log.last().map(|task| task.state),
        Some(TaskState::Done)
    ));
    let task = app.task_log.last().expect("background paste task");
    assert_eq!(
        task.source_locations,
        vec![source_file.display().to_string()],
        "背景貼上必須永久保存實際來源，不可只留下完成訊息"
    );
    assert_eq!(
        task.destination_location,
        Some(target_dir.display().to_string()),
        "背景貼上完成後仍必須能辨識目的目錄"
    );
    assert_eq!(
        app.task_log.last().and_then(|task| task.completed_bytes),
        app.task_log.last().and_then(|task| task.total_bytes),
        "背景貼上完成後已處理 byte 必須等於總 byte，不能停在最後一次中途回報"
    );
}

#[test]
/// 驗證來源只要是目錄就必須直接判定為背景貼上，不能用目錄本身的 metadata 大小
/// 代表內部內容。測試以 sparse file 表示超過 1 GiB 的大型 build 目錄，不實際寫入
/// 或複製 1 GiB 資料。
///
/// 保護目的：過去 `target/` 雖然包含大量資料，目錄 metadata 卻只有數百 bytes，
/// 因而被錯放到 UI thread 同步複製，造成整個 TUI 卡死。
fn directory_paste_is_background_even_when_directory_metadata_is_small() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("target");
    let destination_dir = dir.path().join("destination");
    fs::create_dir(&source_dir).expect("source directory");
    fs::create_dir(&destination_dir).expect("destination directory");
    fs::File::create(source_dir.join("large-build-output.bin"))
        .expect("sparse source")
        .set_len(1024 * 1024 * 1024 + 1)
        .expect("sparse source size");
    let clipboard = ClipboardState {
        entries: vec![ClipboardEntry {
            source_path: source_dir,
            display_name: String::from("target"),
        }],
        operation: ClipboardOperation::Copy,
    };

    assert!(paste_should_run_in_background(&clipboard, &destination_dir));
}

#[test]
/// 驗證 `x` 剪下後可以用 `p` 移動檔案，且剪貼簿會在成功後清空。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_cut_and_paste_moves_file_and_clears_clipboard() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("beta.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .expect("cut");

    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.operation),
        Some(ClipboardOperation::Cut)
    );
    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.entries.len()),
        Some(1)
    );

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("paste");

    assert!(!source_file.exists());
    assert!(target_dir.join("beta.txt").exists());
    assert!(app.clipboard.is_none());
    assert_eq!(app.status, "moved: 1 item");
}

#[test]
/// 驗證一般模式按下 `u` 會復原最近一次 Copy，來源保留且目的檔移入 Trash。
///
/// 保護目的：確保操作歷史不只底層可用，實際快捷鍵也能完成使用者最常見的貼錯復原。
fn app_u_undoes_latest_copy_paste() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "hello").expect("source file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.copy_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir;
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.paste_into_focused_pane().expect("paste");
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
        .expect("undo shortcut");

    assert!(source_file.exists());
    assert!(!target_file.exists());
    assert_eq!(app.status, "undid copy: 1 items");
    assert_eq!(
        app.trash_store.list_entries().expect("trash entries").len(),
        1
    );
}

#[test]
/// 驗證 `:undo` 可以把 cut/paste 的目的檔搬回原來源位置。
///
/// 保護目的：命令與快捷鍵必須呼叫相同 Undo 核心，避免兩套入口行為不一致。
fn app_undo_command_reverses_cut_paste() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("beta.txt");
    let target_file = target_dir.join("beta.txt");
    fs::write(&source_file, "hello").expect("source file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.cut_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir;
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.paste_into_focused_pane().expect("paste");
    app.execute_command("undo").expect("undo command");

    assert!(source_file.exists());
    assert!(!target_file.exists());
    assert_eq!(app.status, "undid move: 1 items");
}

#[test]
/// 驗證覆蓋貼上後執行 Undo，會恢復目的地原內容而不是只移除新檔。
///
/// 保護目的：覆蓋是最高風險操作，必須證明隱藏備份已接入 App 的完整流程。
fn app_undo_overwrite_paste_restores_previous_target() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    fs::write(source_dir.join("same.txt"), "new").expect("new source");
    let target_file = target_dir.join("same.txt");
    fs::write(&target_file, "old").expect("old target");

    let mut app = App::new(source_dir, default_loaded_config()).expect("app");
    app.copy_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir;
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.paste_into_focused_pane_with_overwrite()
        .expect("overwrite paste");
    app.undo_latest_file_operation().expect("undo overwrite");

    assert_eq!(
        fs::read_to_string(target_file).expect("restored target"),
        "old"
    );
    assert_eq!(app.status, "undid copy: 1 items");
}

#[test]
/// 驗證覆蓋貼上時，備份會存到專屬的 undoBackup 目錄，而不會留在專案目標目錄中。
fn app_overwrite_paste_places_backup_in_undo_backup_dir_and_restores() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    fs::write(source_dir.join("Logo.png"), "new logo").expect("new source");
    let target_file = target_dir.join("Logo.png");
    fs::write(&target_file, "old logo").expect("old target");

    let mut app = App::new(source_dir, default_loaded_config()).expect("app");
    app.copy_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    app.paste_into_focused_pane_with_overwrite()
        .expect("overwrite paste");

    // 驗證目標專案目錄內絕對沒有殘留任何 .backup 或 undo-backup 項目
    let target_entries: Vec<_> = fs::read_dir(&target_dir)
        .expect("read target")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(target_entries, vec!["Logo.png"]);

    // 驗證內容已覆蓋成新檔
    assert_eq!(
        fs::read_to_string(&target_file).expect("new target content"),
        "new logo"
    );

    // 驗證執行檔旁的 undoBackup 目錄中有備份檔案
    let backup_dir = crate::file_manager::undo_backup::resolve_undo_backup_dir();
    assert!(backup_dir.exists());
    let backup_entries: Vec<_> = fs::read_dir(&backup_dir)
        .expect("read backup dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("Logo.png-"))
        .collect();
    assert_eq!(backup_entries.len(), 1);

    // 執行 Undo，驗證檔案還原成舊檔，且 undoBackup 中的備份已被移除
    app.undo_latest_file_operation().expect("undo overwrite");
    assert_eq!(
        fs::read_to_string(&target_file).expect("restored old logo"),
        "old logo"
    );
    assert!(!backup_dir.join(&backup_entries[0]).exists());
}

#[test]
/// 驗證按下 `Y` / `X` 可以清掉目前內部剪貼簿中的 copy / cut 狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_y_and_shift_x_clear_clipboard_state() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy yy");
    app.handle_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT))
        .expect("clear copied items");
    assert!(app.clipboard.is_none());
    assert_eq!(app.status, "cleared copied items");

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .expect("cut");
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("clear cut items");
    assert!(app.clipboard.is_none());
    assert_eq!(app.status, "cleared cut items");
}

#[test]
/// 驗證按下 `P` 會以覆蓋模式貼上，而不是自動產生 `copy` 檔名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_p_pastes_with_overwrite_when_clipboard_exists() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "from source").expect("source file");
    fs::write(&target_file, "from target").expect("target file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy yy");

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");
    app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT))
        .expect("overwrite paste");

    assert_eq!(
        fs::read_to_string(&target_file).expect("target content"),
        "from source"
    );
    assert!(!target_dir.join("alpha copy.txt").exists());
    assert_eq!(app.status, "pasted copy with overwrite: 1 item");
}

#[test]
/// 驗證 `:copy-panel <id>` 會把目前選取的檔案複製到指定 pane 的目錄，且來源保留。
fn app_copy_panel_command_copies_selected_entry_to_target_pane_dir() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("delta.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);

    app.execute_command("copy-panel 2").expect("copy panel");

    assert!(source_file.exists());
    assert!(target_dir.join("delta.txt").exists());
    assert_eq!(
        app.status,
        format!("copied 1 item -> {}", target_dir.display())
    );
}

#[test]
/// 驗證 `:copy-panel <id>` 能將所有標記的項目批次複製到目標 pane。
fn app_copy_panel_command_copies_multiple_marked_entries() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let file1 = source_dir.join("a.txt");
    let file2 = source_dir.join("b.txt");
    let file3 = source_dir.join("c.txt");
    fs::write(&file1, "1").expect("file1");
    fs::write(&file2, "2").expect("file2");
    fs::write(&file3, "3").expect("file3");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    // 標記 a.txt 與 b.txt
    let pane = app.current_pane_mut().expect("pane");
    pane.marked_paths.insert(file1.clone());
    pane.marked_paths.insert(file2.clone());

    app.execute_command("copy-panel 2").expect("copy panel");

    assert!(file1.exists());
    assert!(file2.exists());
    assert!(file3.exists());
    assert!(target_dir.join("a.txt").exists());
    assert!(target_dir.join("b.txt").exists());
    assert!(!target_dir.join("c.txt").exists());
    assert_eq!(
        app.status,
        format!("copied 2 items -> {}", target_dir.display())
    );
}

#[test]
/// 驗證 `:copy-panel` 複製後按下 `u` (undo) 能自動清理目標 pane 的副本。
fn app_copy_panel_command_supports_undo() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("delta.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);

    app.execute_command("copy-panel 2").expect("copy panel");
    assert!(target_dir.join("delta.txt").exists());

    app.undo_latest_file_operation().expect("undo copy");
    assert!(!target_dir.join("delta.txt").exists());
    assert!(source_file.exists());
    assert_eq!(app.status, "undid copy: 1 items");
}

#[test]
/// 驗證 `:copy-panel <id>` 若指定不存在的 pane 或無參數，會提示可用編號與語法。
fn app_copy_panel_reports_available_panes_on_invalid_id() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");

    app.execute_command("copy-panel 9").expect("copy panel");
    assert_eq!(app.status, "unknown panel 9. available: 1, 2");

    app.execute_command("copy-panel").expect("copy panel");
    assert_eq!(app.status, "usage: copy-panel <panel-id>. available: 1, 2");
}

#[test]
/// 驗證 `:copy-panel` 複製大檔案時，會自動交給背景工作處理以避免卡住 TUI。
fn app_copy_panel_command_runs_in_background_when_target_is_external() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("large.zip");
    fs::File::create(&source_file)
        .expect("create file")
        .set_len(BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
        .expect("set len");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    app.execute_command("copy-panel 2").expect("copy panel");

    assert!(!app.file_job_receivers.is_empty());
    assert!(app.task_log.iter().any(|t| t.title.starts_with("copy")));
}

#[test]
/// 驗證一般貼上遇到同名檔案時，會先開啟覆蓋確認視窗，使用者確認後才覆蓋。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_paste_with_conflict_requires_confirmation_before_overwrite() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "from source").expect("source file");
    fs::write(&target_file, "from target").expect("target file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy yy");

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open overwrite confirm");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmPasteOverwrite { .. })
    ));
    assert_eq!(
        fs::read_to_string(&target_file).expect("target content before confirm"),
        "from target"
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("confirm overwrite");

    assert_eq!(
        fs::read_to_string(&target_file).expect("target content after confirm"),
        "from source"
    );
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "pasted copy with overwrite: 1 item");
}

#[test]
/// 驗證一般貼上遇到同名檔案時，若使用者取消，會保留原檔案且不執行貼上。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_paste_with_conflict_can_be_cancelled() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "from source").expect("source file");
    fs::write(&target_file, "from target").expect("target file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy yy");

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open overwrite confirm");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel overwrite");

    assert_eq!(
        fs::read_to_string(&target_file).expect("target content after cancel"),
        "from target"
    );
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "paste cancelled: alpha.txt");
}

#[test]
/// 驗證 normal mode 按下 `c` 會打開文字複製小視窗。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_c_key_opens_copy_picker() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("open copy picker");

    match app.pending_action {
        Some(PendingAction::CopyPicker {
            pane_id, selected, ..
        }) => {
            assert_eq!(pane_id, 1);
            assert_eq!(selected, 0);
        }
        other => panic!("expected copy picker, got {other:?}"),
    }
    assert_eq!(app.status, "copy to clipboard: alpha.txt");
}

#[test]
/// 驗證文字複製小視窗按下 `h` 會關閉並回到一般模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_picker_h_closes_panel() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_copy_picker().expect("open copy picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("close copy picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證文字複製小視窗打開後，再按一次 `c` 會直接關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_picker_c_toggles_closed() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_copy_picker().expect("open copy picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("toggle close copy picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證文字複製小視窗中，原本的檔案路徑複製已改成 `u`，避免和 opener `c` 衝突。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_picker_u_copies_file_path() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_copy_picker().expect("open copy picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
        .expect("copy file path");

    assert_eq!(app.status, "copied file path: alpha.txt");
}

#[test]
/// 驗證使用者從 c->d 拷貝目錄，再到 gt 開啟 goto 命令、按 Esc 轉 Normal 模式後按 p 貼上的完整流程。
fn test_gt_flow_copy_cd_then_gt_esc_p_pastes() {
    let _lock = crate::file_manager::platform::TEST_CLIPBOARD_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempdir().expect("tempdir");
    let test_folder = dir.path().join("my_subfolder");
    fs::create_dir_all(&test_folder).expect("create folder");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    // 模擬在 pane 中使用 c 然後 d 複製目錄路徑
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("press c");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::CopyPicker { .. })
    ));
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("press d");
    assert!(app.pending_action.is_none());

    // 模擬執行 gt 打開 goto 輸入框
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("press g");
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("press t");
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "goto ");
    assert_eq!(app.text_input_mode, RenameMode::Insert);

    // 按下 Esc 切換到 Normal 模式
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("press Esc");
    assert!(app.command_mode);
    assert_eq!(app.text_input_mode, RenameMode::Normal);

    // 在 Normal 模式下按下 p
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("press p");

    assert!(app.command_buffer.starts_with("goto "));
    assert!(
        app.command_buffer.len() > "goto ".len(),
        "command buffer was not updated: '{}'",
        app.command_buffer
    );
}
