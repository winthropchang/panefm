use super::*;

#[test]
/// 驗證單一檔案衝突時，按下 `o` 鍵會以覆蓋模式完成貼上，並正確更新目標內容與狀態。
fn test_paste_single_conflict_overwrite_key_o() {
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

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .expect("choose overwrite via hotkey o");

    assert_eq!(
        fs::read_to_string(&target_file).expect("target content after overwrite"),
        "from source"
    );
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "pasted copy with overwrite: 1 item");
}

#[test]
/// 驗證單一檔案衝突時，按下 `r` 鍵會自動更名（保留原檔，新檔命名為 `alpha copy.txt`）。
fn test_paste_single_conflict_auto_rename_key_r() {
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

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("choose auto rename via hotkey r");

    assert_eq!(
        fs::read_to_string(&target_file).expect("original target remains untouched"),
        "from target"
    );
    let renamed_file = target_dir.join("alpha copy.txt");
    assert!(
        renamed_file.exists(),
        "auto-renamed duplicate file must exist"
    );
    assert_eq!(
        fs::read_to_string(&renamed_file).expect("renamed content"),
        "from source"
    );
    assert!(app.pending_action.is_none());
}

#[test]
/// 驗證單一檔案衝突時，按下 `s` 鍵會略過此檔案，不貼上且保留目標原始內容。
fn test_paste_single_conflict_skip_key_s() {
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

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("choose skip via hotkey s");

    assert_eq!(
        fs::read_to_string(&target_file).expect("original target remains untouched"),
        "from target"
    );
    assert!(!target_dir.join("alpha copy.txt").exists());
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "all conflicting items skipped; nothing pasted");
}

#[test]
/// 驗證透過上下方向鍵導航選單項目並按 Enter 鍵確認執行。
fn test_paste_conflict_menu_arrow_key_navigation() {
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

    // 初始 selected_option 為 0（Overwrite）
    // 按 Down 鍵移動至 1（AutoRename）
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("press down arrow");

    match &app.pending_action {
        Some(PendingAction::ConfirmPasteOverwrite {
            selected_option, ..
        }) => {
            assert_eq!(*selected_option, 1);
        }
        other => panic!("expected ConfirmPasteOverwrite, got {other:?}"),
    }

    // 按 Enter 執行 AutoRename
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("press enter to execute auto-rename");

    assert_eq!(
        fs::read_to_string(&target_file).expect("original target remains untouched"),
        "from target"
    );
    let renamed_file = target_dir.join("alpha copy.txt");
    assert!(renamed_file.exists());
    assert_eq!(
        fs::read_to_string(&renamed_file).expect("renamed content"),
        "from source"
    );
    assert!(app.pending_action.is_none());
}

#[test]
/// 驗證多檔案衝突時，按下 `O` (Shift+O) 全部覆蓋。
fn test_paste_multi_conflict_overwrite_all_key_shift_o() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");

    fs::write(source_dir.join("a.txt"), "src_a").expect("src_a");
    fs::write(source_dir.join("b.txt"), "src_b").expect("src_b");
    fs::write(target_dir.join("a.txt"), "dst_a").expect("dst_a");
    fs::write(target_dir.join("b.txt"), "dst_b").expect("dst_b");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    // 設定多個剪貼簿項目
    app.clipboard = Some(ClipboardState {
        operation: ClipboardOperation::Copy,
        entries: vec![
            ClipboardEntry {
                source_path: source_dir.join("a.txt"),
                display_name: String::from("a.txt"),
            },
            ClipboardEntry {
                source_path: source_dir.join("b.txt"),
                display_name: String::from("b.txt"),
            },
        ],
    });

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open confirm");

    match &app.pending_action {
        Some(PendingAction::ConfirmPasteOverwrite { conflicts, .. }) => {
            assert_eq!(conflicts.len(), 2);
        }
        other => panic!("expected ConfirmPasteOverwrite, got {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
        .expect("press O for overwrite all");

    assert_eq!(
        fs::read_to_string(target_dir.join("a.txt")).expect("a.txt"),
        "src_a"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("b.txt")).expect("b.txt"),
        "src_b"
    );
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "pasted copy with overwrite: 2 items");
}

#[test]
/// 驗證多檔案衝突時，按下 `R` (Shift+R) 全部自動更名。
fn test_paste_multi_conflict_auto_rename_all_key_shift_r() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");

    fs::write(source_dir.join("a.txt"), "src_a").expect("src_a");
    fs::write(source_dir.join("b.txt"), "src_b").expect("src_b");
    fs::write(target_dir.join("a.txt"), "dst_a").expect("dst_a");
    fs::write(target_dir.join("b.txt"), "dst_b").expect("dst_b");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.clipboard = Some(ClipboardState {
        operation: ClipboardOperation::Copy,
        entries: vec![
            ClipboardEntry {
                source_path: source_dir.join("a.txt"),
                display_name: String::from("a.txt"),
            },
            ClipboardEntry {
                source_path: source_dir.join("b.txt"),
                display_name: String::from("b.txt"),
            },
        ],
    });

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open confirm");

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
        .expect("press R for auto rename all");

    assert_eq!(
        fs::read_to_string(target_dir.join("a.txt")).expect("dst_a"),
        "dst_a"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("b.txt")).expect("dst_b"),
        "dst_b"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("a copy.txt")).expect("a copy.txt"),
        "src_a"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("b copy.txt")).expect("b copy.txt"),
        "src_b"
    );
    assert!(app.pending_action.is_none());
}

#[test]
/// 驗證多檔案衝突時，按下 `S` (Shift+S) 全部略過衝突項目，其餘非衝突項目正常貼上。
fn test_paste_multi_conflict_skip_all_key_shift_s() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");

    fs::write(source_dir.join("a.txt"), "src_a").expect("src_a");
    fs::write(source_dir.join("b.txt"), "src_b").expect("src_b");
    fs::write(source_dir.join("c.txt"), "src_c").expect("src_c");
    fs::write(target_dir.join("a.txt"), "dst_a").expect("dst_a");
    fs::write(target_dir.join("b.txt"), "dst_b").expect("dst_b");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.clipboard = Some(ClipboardState {
        operation: ClipboardOperation::Copy,
        entries: vec![
            ClipboardEntry {
                source_path: source_dir.join("a.txt"),
                display_name: String::from("a.txt"),
            },
            ClipboardEntry {
                source_path: source_dir.join("b.txt"),
                display_name: String::from("b.txt"),
            },
            ClipboardEntry {
                source_path: source_dir.join("c.txt"),
                display_name: String::from("c.txt"),
            },
        ],
    });

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open confirm");

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))
        .expect("press S for skip all");

    assert_eq!(
        fs::read_to_string(target_dir.join("a.txt")).expect("dst_a"),
        "dst_a"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("b.txt")).expect("dst_b"),
        "dst_b"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("c.txt")).expect("c.txt"),
        "src_c"
    );
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "pasted copy: 1 item(s) (skipped 2 item(s))");
}

#[test]
/// 驗證多檔案衝突逐一決定：第一筆覆蓋、第二筆更名。
fn test_paste_multi_conflict_step_by_step_decisions() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");

    fs::write(source_dir.join("a.txt"), "src_a").expect("src_a");
    fs::write(source_dir.join("b.txt"), "src_b").expect("src_b");
    fs::write(target_dir.join("a.txt"), "dst_a").expect("dst_a");
    fs::write(target_dir.join("b.txt"), "dst_b").expect("dst_b");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.clipboard = Some(ClipboardState {
        operation: ClipboardOperation::Copy,
        entries: vec![
            ClipboardEntry {
                source_path: source_dir.join("a.txt"),
                display_name: String::from("a.txt"),
            },
            ClipboardEntry {
                source_path: source_dir.join("b.txt"),
                display_name: String::from("b.txt"),
            },
        ],
    });

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open confirm");

    // 第一筆衝突 (a.txt)：按下 o 覆蓋
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .expect("decide item 1 as overwrite");

    // 應該推進至第二筆衝突 (b.txt)
    match &app.pending_action {
        Some(PendingAction::ConfirmPasteOverwrite {
            current_index,
            target_name,
            ..
        }) => {
            assert_eq!(*current_index, 1);
            assert_eq!(target_name, "b.txt");
        }
        other => panic!("expected step 2 ConfirmPasteOverwrite, got {other:?}"),
    }

    // 第二筆衝突 (b.txt)：按下 r 自動更名
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("decide item 2 as rename");

    // 兩筆衝突皆已決定，執行貼上完成
    assert!(app.pending_action.is_none());
    assert_eq!(
        fs::read_to_string(target_dir.join("a.txt")).expect("a.txt was overwritten"),
        "src_a"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("b.txt")).expect("b.txt was preserved"),
        "dst_b"
    );
    assert_eq!(
        fs::read_to_string(target_dir.join("b copy.txt")).expect("b copy.txt was created"),
        "src_b"
    );
}

#[test]
/// 驗證剪下 (Cut) 遇到衝突略過時，被略過的項目保留在來源且保留在剪貼簿中。
fn test_paste_cut_skip_preserves_unmoved_in_clipboard() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");

    fs::write(source_dir.join("a.txt"), "src_a").expect("src_a");
    fs::write(source_dir.join("b.txt"), "src_b").expect("src_b");
    fs::write(target_dir.join("a.txt"), "dst_a").expect("dst_a");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.clipboard = Some(ClipboardState {
        operation: ClipboardOperation::Cut,
        entries: vec![
            ClipboardEntry {
                source_path: source_dir.join("a.txt"),
                display_name: String::from("a.txt"),
            },
            ClipboardEntry {
                source_path: source_dir.join("b.txt"),
                display_name: String::from("b.txt"),
            },
        ],
    });

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open confirm");

    // 只有 a.txt 衝突，選擇略過
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("skip a.txt");

    // a.txt 未被搬移（仍留在 source_dir）
    assert!(source_dir.join("a.txt").exists());
    assert_eq!(
        fs::read_to_string(target_dir.join("a.txt")).expect("target a.txt"),
        "dst_a"
    );

    // b.txt 成功搬移至 target_dir（從 source_dir 移除）
    assert!(!source_dir.join("b.txt").exists());
    assert_eq!(
        fs::read_to_string(target_dir.join("b.txt")).expect("target b.txt"),
        "src_b"
    );

    // 剪貼簿中仍保留未搬移成功的 a.txt
    let clipboard = app.clipboard.expect("clipboard should retain skipped item");
    assert_eq!(clipboard.entries.len(), 1);
    assert_eq!(clipboard.entries[0].source_path, source_dir.join("a.txt"));
}
