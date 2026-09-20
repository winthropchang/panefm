use super::*;

#[test]
/// 驗證 normal mode 按下 `R` 會打開預填好的 `rename-regex ` 命令輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_r_opens_prefilled_rename_regex_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
        .expect("open prefilled rename-regex command");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "rename-regex ");
    assert_eq!(app.status, "command mode");
}

#[test]
/// 驗證打開重新命名視窗時，會帶入目前選取項目的原名稱與預設輸入值。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_start_rename_opens_dialog_with_selected_name() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_rename();

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("alpha.txt"),
            cursor: 5,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證在重新命名視窗按下 Enter 後會套用新的檔名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_confirm_updates_selected_entry() {
    let dir = tempdir().expect("tempdir");
    let old_path = dir.path().join("alpha.txt");
    let new_path = dir.path().join("beta.txt");
    fs::write(&old_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::Rename {
        pane_id: 1,
        original_name: String::from("alpha.txt"),
        buffer: String::from("beta.txt"),
        cursor: 4,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("rename");

    assert!(!old_path.exists());
    assert!(new_path.exists());
    assert_eq!(app.status, "renamed alpha.txt -> beta.txt");
}

#[test]
/// 驗證 `:rename-regex` 會打開預覽面板，並正確標示 ready / unchanged。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_regex_command_opens_preview_panel() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.md"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("mark second");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");

    app.execute_command("rename-regex '^(.*)\\.txt$' '$1.md'")
        .expect("open regex rename");

    match app.pending_action.as_ref() {
        Some(PendingAction::RegexRename { previews, .. }) => {
            assert_eq!(previews.len(), 2);
            assert_eq!(previews[0].new_name, "alpha.md");
            assert_eq!(previews[0].outcome, RegexRenameOutcome::Ready);
            assert_eq!(previews[1].new_name, "beta.md");
            assert_eq!(previews[1].outcome, RegexRenameOutcome::Unchanged);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
}

#[test]
/// 驗證 regex 批次改名在按下 Enter 後會一次套用所有 ready 項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_regex_preview_applies_ready_entries() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "a").expect("alpha");
    fs::write(&beta, "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("mark second");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.execute_command("rename-regex '^(.*)\\.txt$' 'file_$1.md'")
        .expect("open regex rename");

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("apply regex rename");

    assert!(!alpha.exists());
    assert!(!beta.exists());
    assert!(dir.path().join("file_alpha.md").exists());
    assert!(dir.path().join("file_beta.md").exists());
    assert_eq!(app.status, "rename-regex: renamed 2 items");
}

#[test]
/// 驗證從命令輸入介面送出 `reg` 後，預覽面板再次按 Enter 會實際完成改名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_regex_rename_command_ui_enter_applies_preview() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("alpha.txt");
    let target = dir.path().join("alpha.md");
    fs::write(&source, "a").expect("source");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
        .expect("open regex command");
    assert!(app.command_mode);
    app.command_buffer = String::from("reg '^(.*)\\.txt$' '$1.md'");

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("submit regex command");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::RegexRename { .. })
    ));

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("apply regex preview");

    assert!(!source.exists());
    assert!(target.exists());
    assert_eq!(app.status, "rename-regex: renamed 1 item");
}

#[test]
/// 驗證 regex 批次改名若會撞名，會標示 conflict，且 Enter 不會直接套用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_regex_preview_blocks_conflicts() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "a").expect("alpha");
    fs::write(&beta, "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("mark second");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.execute_command("rename-regex '^(.*)\\.txt$' 'same.txt'")
        .expect("open regex rename");

    match app.pending_action.as_ref() {
        Some(PendingAction::RegexRename { previews, .. }) => {
            assert!(
                previews
                    .iter()
                    .all(|preview| preview.outcome == RegexRenameOutcome::Conflict)
            );
        }
        other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("try apply conflicting rename");

    assert!(alpha.exists());
    assert!(beta.exists());
    assert_eq!(app.status, "rename-regex: resolve conflicts before apply");
}

#[test]
/// 驗證 rename 預設游標會停在副檔名前，方便優先修改主檔名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_basename_cursor_stops_before_extension() {
    assert_eq!(rename_basename_cursor("alpha.txt"), 5);
    assert_eq!(rename_basename_cursor("archive.tar.gz"), 11);
    assert_eq!(rename_basename_cursor(".gitignore"), 10);
    assert_eq!(rename_basename_cursor("folder"), 6);
    assert_eq!(rename_basename_cursor("測試檔案.txt"), 4);
    assert_eq!(rename_basename_cursor("中文.tar.gz"), 6);
    assert_eq!(rename_basename_cursor(".隱藏檔"), 4);
    assert_eq!(rename_basename_cursor("純中文"), 3);
}

#[test]
/// 驗證 rename 可以在 insert 與 normal 模式之間切換，並保留游標位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_mode_switches_between_insert_and_normal() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_rename();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("switch to normal");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("alpha.txt"),
            cursor: 5,
            mode: RenameMode::Normal,
        })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("move left");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("back to insert");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("alpha.txt"),
            cursor: 4,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證 rename 的 Vim 單字移動會依照檔名分隔符正確跳轉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_word_motion_helpers_follow_filename_segments() {
    let name = "my-long_file.txt";

    assert_eq!(rename_next_word_start(name, 0), 3);
    assert_eq!(rename_next_word_start(name, 3), 8);
    assert_eq!(rename_next_word_start(name, 8), 13);

    assert_eq!(rename_previous_word_start(name, 13), 8);
    assert_eq!(rename_previous_word_start(name, 8), 3);
    assert_eq!(rename_previous_word_start(name, 3), 0);

    assert_eq!(rename_word_end(name, 0), 1);
    assert_eq!(rename_word_end(name, 3), 6);
    assert_eq!(rename_word_end(name, 8), 11);
    assert_eq!(rename_word_end(name, 12), 15);
}

#[test]
/// 驗證 rename 的 normal 模式支援 `w`、`b`、`e`、`a`、`A` 這些 Vim 風格操作。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_normal_mode_supports_vim_word_motions_and_insert_shortcuts() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("my-long_file.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_rename();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("switch to normal");

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("move to previous word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("move to next word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("move to word end");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("append after cursor");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("my-long_file.txt"),
            buffer: String::from("my-long_file.txt"),
            cursor: 16,
            mode: RenameMode::Insert,
        })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("back to normal");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE))
        .expect("jump to start");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("jump to next word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("jump to end of word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("append inside basename");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("my-long_file.txt"),
            buffer: String::from("my-long_file.txt"),
            cursor: 7,
            mode: RenameMode::Insert,
        })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("back to normal again");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE))
        .expect("append at end");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("my-long_file.txt"),
            buffer: String::from("my-long_file.txt"),
            cursor: 16,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證中文檔名在 rename 時，畫面游標會依字元全形寬度（每個中文字 2 欄位）精確定位在副檔名前，
/// 且進行 Backspace、插入、Delete 等編輯時，實際字串與畫面游標皆同步正確，檔案能成功重新命名。
/// 保護目的：避免將中文檔名字元數直接當成終端欄位數計算游標 X 座標，導致游標落在錯誤中文字元上而引導使用者改錯檔名。
fn rename_chinese_filename_aligns_screen_cursor_and_edits_correctly() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("測試檔案.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_rename();

    // 初始進入 rename，游標應停在第 4 個字元（主檔名「測試檔案」末端、副檔名「.txt」前面）
    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("測試檔案.txt"),
            buffer: String::from("測試檔案.txt"),
            cursor: 4,
            mode: RenameMode::Insert,
        })
    );

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut cursor_pos = None;
    terminal
        .draw(|frame| {
            cursor_pos = app.render(frame);
        })
        .expect("render");

    // 畫面上「測試檔案」4 個中文字佔用 8 欄位寬度（非 4 欄位）
    // 游標 X 應位在輸入框內容起點 (input_inner.x) + 8 欄位處
    let initial_x = cursor_pos.expect("cursor").0;

    // 按 Backspace 刪除最後一個中文字「案」
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
        .expect("backspace");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("測試檔案.txt"),
            buffer: String::from("測試檔.txt"),
            cursor: 3,
            mode: RenameMode::Insert,
        })
    );
    terminal
        .draw(|frame| {
            cursor_pos = app.render(frame);
        })
        .expect("render after backspace");
    // 刪除一個中文字應後退 2 欄位
    assert_eq!(cursor_pos.expect("cursor").0, initial_x - 2);

    // 輸入新中文字「新」
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('新'), KeyModifiers::NONE))
        .expect("insert chinese char");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("測試檔案.txt"),
            buffer: String::from("測試檔新.txt"),
            cursor: 4,
            mode: RenameMode::Insert,
        })
    );
    terminal
        .draw(|frame| {
            cursor_pos = app.render(frame);
        })
        .expect("render after insert");
    assert_eq!(cursor_pos.expect("cursor").0, initial_x);

    // 測試 Home / End 鍵
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))
        .expect("home");
    terminal
        .draw(|frame| {
            cursor_pos = app.render(frame);
        })
        .expect("render after home");
    assert_eq!(cursor_pos.expect("cursor").0, initial_x - 8);

    app.handle_pending_action_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
        .expect("end");
    terminal
        .draw(|frame| {
            cursor_pos = app.render(frame);
        })
        .expect("render after end");
    // 「測試檔新.txt」: 4箇中文字(8) + 4個ASCII(4) = 12 欄位
    assert_eq!(cursor_pos.expect("cursor").0, initial_x - 8 + 12);

    // 回到主檔名末尾
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("left");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("left");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("left");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("left");

    // 測試 Delete 鍵刪除 '.'
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE))
        .expect("delete dot");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("測試檔案.txt"),
            buffer: String::from("測試檔新txt"),
            cursor: 4,
            mode: RenameMode::Insert,
        })
    );

    // 補回 '.'
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
        .expect("insert dot");

    // 按 Enter 確認重新命名
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("confirm rename");
    assert!(dir.path().join("測試檔新.txt").exists());
    assert!(!dir.path().join("測試檔案.txt").exists());
}
