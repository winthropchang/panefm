use super::*;

#[test]
/// 驗證文字輸入 helper 會把 `Shift+6` 這類終端事件正規化成真正的符號字元。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn typed_char_from_key_normalizes_shifted_symbols() {
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('6'), KeyModifiers::SHIFT)),
        Some('^')
    );
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('-'), KeyModifiers::SHIFT)),
        Some('_')
    );
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT)),
        Some('A')
    );
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL)),
        None
    );
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT)),
        None
    );
}

#[test]
/// 驗證功能型按鍵 helper 會接受常見的 terminal 事件變體。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn key_normalization_helpers_accept_terminal_variants() {
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        'h'
    ));
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        'j'
    ));
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        'k'
    ));
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        'l'
    ));
    assert!(key_matches_shifted_letter(
        &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT),
        'N'
    ));
    assert!(key_matches_shifted_letter(
        &KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE),
        'N'
    ));
    assert!(key_matches_ctrl_letter(
        &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        'p'
    ));
    assert!(key_matches_ctrl_letter(
        &KeyEvent::new(
            KeyCode::Char('P'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        'p'
    ));
    assert!(key_matches_letter_any_case(
        &KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        'y'
    ));
    assert!(key_matches_letter_any_case(
        &KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE),
        'y'
    ));
}

#[test]
/// 驗證可以用 `V` 視覺標記多個項目，並一次放進剪貼簿。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_marked_entries_copy_into_clipboard_as_batch() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("open yank picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy batch yy");

    let clipboard = app.clipboard.as_ref().expect("clipboard");
    assert_eq!(clipboard.operation, ClipboardOperation::Copy);
    assert_eq!(clipboard.entries.len(), 2);
    assert_eq!(app.status, "copied 2 items");
}

#[test]
/// 驗證 `V` 視覺標記多個項目後，刪除確認會一次刪掉整批項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_marked_entries_delete_as_batch() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "a").expect("alpha");
    fs::write(&beta, "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.start_delete_confirmation(false);

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete { ref target_name, .. }) if target_name == "2 items"
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm delete batch");

    assert!(!alpha.exists());
    assert!(!beta.exists());
    assert_eq!(app.status, "trashed 2 items");
}

#[test]
/// 驗證 `V` 進入 visual selection 後，移動游標再按一次 `V` 會提交整段標記。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_selection_commits_range_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down again");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");

    let pane = app.panes.get(&1).expect("pane");
    assert!(app.visual_selection.is_none());
    assert_eq!(pane.marked_count(), 3);
    assert_eq!(app.status, "marked 3 items");
}

#[test]
/// 驗證小寫 `v` 可以進入、移動並結束 visual selection。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_lowercase_v_controls_visual_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("extend visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("close visual");

    assert!(app.visual_selection.is_none());
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);
}

#[test]
/// 驗證某些終端把 `Shift+v` 回報成 `v + Shift` 時，也能正確進入 visual selection。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_v_opens_visual_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SHIFT))
        .expect("open visual with shifted v");

    assert_eq!(
        app.visual_selection,
        Some(VisualSelectionState {
            pane_id: 1,
            anchor: 0,
            current: 0,
        })
    );
    assert_eq!(app.status, "visual: range selection");
}

#[test]
/// 驗證某些終端把 `Shift+g` 回報成 `g + Shift` 時，也能正確執行 `G` 跳到列表底部。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_g_jumps_to_bottom() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT))
        .expect("jump bottom with shifted g");

    assert_eq!(app.panes.get(&1).expect("pane").selected, 2);
    assert_eq!(app.status, "jumped to bottom");
}

#[test]
/// 驗證 visual selection 按下 `Esc` 會先提交這一段範圍並離開選取模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_selection_escape_commits_current_range() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit first range");

    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move to third");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open second visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("move back");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("commit second visual");

    let pane = app.panes.get(&1).expect("pane");
    assert!(app.visual_selection.is_none());
    assert_eq!(pane.marked_count(), 3);
    assert_eq!(app.status, "marked 1 items");
}

#[test]
/// 驗證離開選取模式後再按一次 `Esc`，會清掉目前所有已提交標記。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_escape_in_normal_mode_clears_all_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("clear all marks");

    let pane = app.panes.get(&1).expect("pane");
    assert!(app.visual_selection.is_none());
    assert_eq!(pane.marked_count(), 0);
    assert_eq!(app.status, "cleared 2 marks");
}

#[test]
/// 驗證 normal mode 的 `Ctrl-u` 會依照目前列表 viewport 高度做半頁移動，`Ctrl-d` 則切換 VCS Diff 預覽模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_d_and_ctrl_u_move_by_half_page() {
    let dir = tempdir().expect("tempdir");
    for index in 0..10 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let pane = app.panes.get_mut(&1).expect("pane");
    pane.set_list_viewport_height(6);
    pane.move_down_by(3);
    assert_eq!(pane.selected, 3);

    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("page up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(app.status, "half page up: 3");

    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("toggle diff");
    assert!(app.panes.get(&1).expect("pane").preview_open);
    assert!(app.panes.get(&1).expect("pane").preview_diff_mode);
    assert!(app.status.contains("preview diff mode"));
}

#[test]
/// 驗證 normal mode 的 `Ctrl-f / Ctrl-b` 會依照目前列表 viewport 高度做整頁移動。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_f_and_ctrl_b_move_by_full_page() {
    let dir = tempdir().expect("tempdir");
    for index in 0..12 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_list_viewport_height(5);

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        .expect("full page down");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
    assert_eq!(app.status, "page down: 5");

    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL))
        .expect("full page up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(app.status, "page up: 5");
}

#[test]
/// 驗證 visual selection 中的 `Ctrl-d / Ctrl-u` 也會用半頁步長移動，並同步更新範圍。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_selection_ctrl_d_and_ctrl_u_follow_half_page() {
    let dir = tempdir().expect("tempdir");
    for index in 0..10 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_list_viewport_height(6);

    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("visual page down");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 3);
    assert_eq!(
        app.visual_selection,
        Some(VisualSelectionState {
            pane_id: 1,
            anchor: 0,
            current: 3,
        })
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("visual page up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(
        app.visual_selection,
        Some(VisualSelectionState {
            pane_id: 1,
            anchor: 0,
            current: 0,
        })
    );
}

#[test]
/// 驗證 regex rename 使用的 command UI 可以切到 Normal 模式移動游標，再回到 Insert 修正中間文字。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn command_input_supports_vim_normal_and_insert_modes() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_prefilled_command("rename-regex foo baz");
    app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("enter normal mode");
    assert!(app.command_mode);
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert_eq!(app.rename_cursor_mode(), Some(RenameMode::Normal));

    for _ in 0..2 {
        app.handle_command_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("move left");
    }
    app.handle_command_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("enter insert mode");
    app.handle_command_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("insert correction");

    assert_eq!(app.command_buffer, "rename-regex foo Xbaz");
    assert_eq!(app.text_input_mode, RenameMode::Insert);
}

#[test]
/// 驗證一般 filter 第一次 Esc 只切換模式，Normal 模式第二次 Esc 才鎖定並離開輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn filter_input_uses_two_stage_escape_and_supports_cursor_editing() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("aXbc.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_filter_input(FilterMode::Normal);
    for character in ['a', 'b', 'c'] {
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
            .expect("type filter");
    }
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("normal mode");
    assert!(app.filter.as_ref().is_some_and(|filter| filter.editing));

    app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("move left");
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("insert mode");
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("insert middle");
    assert_eq!(app.filter.as_ref().expect("filter").buffer, "aXbc");

    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("normal mode again");
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("leave editor");
    assert!(!app.filter.as_ref().expect("filter").editing);
}

#[test]
/// 驗證 help、trash、task、bookmark 與 zoxide 共用的面板搜尋器會攔截 Normal 模式按鍵，不會誤關面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn panel_search_uses_shared_vim_editor_before_panel_actions() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open help filter");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type filter");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("normal mode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("move cursor instead of closing panel");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            search: PanelSearchState { editing: true, .. },
            ..
        })
    ));
    assert_eq!(app.text_input_mode, RenameMode::Normal);

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close panel search");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            search: PanelSearchState { editing: false, .. },
            ..
        })
    ));
}

#[test]
/// 驗證 sanitize_pasted_text 能移除結尾 \r\n 並將內部換行替換為空白。
fn test_sanitize_pasted_text() {
    assert_eq!(sanitize_pasted_text("C:\\work\\dir\r\n"), "C:\\work\\dir");
    assert_eq!(sanitize_pasted_text("/home/user/dir\n"), "/home/user/dir");
    assert_eq!(
        sanitize_pasted_text("line1\r\nline2\nline3"),
        "line1 line2 line3"
    );
    assert_eq!(sanitize_pasted_text("simple"), "simple");
    assert_eq!(sanitize_pasted_text(""), "");
}

#[test]
/// 驗證 insert_str 可以在指定字元游標處插入字串並正確推進游標，且支援 UTF-8。
fn test_insert_str_unicode() {
    let mut buffer = String::from("hello world");
    let mut cursor = 5;
    insert_str(&mut buffer, &mut cursor, " 中文");
    assert_eq!(buffer, "hello 中文 world");
    assert_eq!(cursor, 8);

    insert_str(&mut buffer, &mut cursor, "");
    assert_eq!(buffer, "hello 中文 world");
    assert_eq!(cursor, 8);
}

#[test]
/// 驗證 handle_bracketed_paste 能正確將路徑貼上到 command_mode（模擬 gt 貼上路徑流程）。
fn test_handle_bracketed_paste_into_command_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_prefilled_command("goto ");
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "goto ");
    assert_eq!(app.text_input_cursor, 5);

    app.handle_bracketed_paste("D:\\target\\folder\r\n")
        .expect("paste");
    assert_eq!(app.command_buffer, "goto D:\\target\\folder");
    assert_eq!(app.text_input_cursor, 21);
}

#[test]
/// 驗證 handle_bracketed_paste 能正確貼入 Rename 與 CreateEntry 的輸入框。
fn test_handle_bracketed_paste_into_modal_actions() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // Rename
    app.pending_action = Some(PendingAction::Rename {
        pane_id: 1,
        original_name: "doc.txt".into(),
        buffer: "doc".into(),
        cursor: 3,
        mode: RenameMode::Insert,
    });
    app.handle_bracketed_paste("_backup\n").expect("paste");
    if let Some(PendingAction::Rename { buffer, cursor, .. }) = app.pending_action.as_ref() {
        assert_eq!(buffer, "doc_backup");
        assert_eq!(*cursor, 10);
    } else {
        panic!("expected Rename action");
    }

    // CreateEntry
    app.pending_action = Some(PendingAction::CreateEntry {
        pane_id: 1,
        buffer: "src/".into(),
        cursor: 4,
        mode: RenameMode::Insert,
    });
    app.handle_bracketed_paste("main.rs\r\n").expect("paste");
    if let Some(PendingAction::CreateEntry { buffer, cursor, .. }) = app.pending_action.as_ref() {
        assert_eq!(buffer, "src/main.rs");
        assert_eq!(*cursor, 11);
    } else {
        panic!("expected CreateEntry action");
    }
}

#[test]
/// 驗證在系統剪貼簿有內容時，edit_text_buffer 於 Insert 模式支援 Ctrl+v，於 Normal 模式支援 p 與 P。
fn test_edit_text_buffer_paste_insert_and_normal() {
    let _lock = crate::file_manager::platform::TEST_CLIPBOARD_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let test_str = "pasted_path";
    let write_res = write_text_to_system_clipboard(test_str);
    let read_res = read_text_from_system_clipboard();
    if write_res.is_ok()
        && let Some(clipboard_text) = read_res
        && clipboard_text.contains(test_str)
    {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        // 1. Insert 模式下的 Ctrl+v
        let mut buffer = String::from("goto ");
        app.text_input_mode = RenameMode::Insert;
        app.text_input_cursor = 5;
        let result = app.edit_text_buffer(
            &mut buffer,
            &KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
        );
        assert_eq!(result, TextEditResult::Changed);
        assert!(buffer.starts_with("goto pasted_path"));

        // 2. Normal 模式下的 p (在游標後貼上)
        let mut buffer_p = String::from("goto ");
        app.text_input_mode = RenameMode::Normal;
        app.text_input_cursor = 4; // 游標停在最後一個空白上
        let result_p = app.edit_text_buffer(
            &mut buffer_p,
            &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );
        assert_eq!(result_p, TextEditResult::Changed);
        assert!(buffer_p.starts_with("goto pasted_path"));

        // 3. Normal 模式下的 P (在游標前貼上)
        let mut buffer_cap_p = String::from("goto ");
        app.text_input_mode = RenameMode::Normal;
        app.text_input_cursor = 4; // 游標停在空白上，P 應在空白前貼上
        let result_cap_p = app.edit_text_buffer(
            &mut buffer_cap_p,
            &KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT),
        );
        assert_eq!(result_cap_p, TextEditResult::Changed);
        assert!(buffer_cap_p.starts_with("gotopasted_path"));
    }
}
