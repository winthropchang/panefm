use super::*;

#[test]
/// 驗證空白 command UI 在 Insert 模式按第一次 Esc 就會直接關閉。
/// 保護目的：空輸入沒有文字需要進入 Vim Normal 模式修正，不應要求使用者連按兩次。
fn empty_command_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_prefilled_command("");
    app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty command input");

    assert!(!app.command_mode);
    assert!(app.command_buffer.is_empty());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證剛開啟且尚未輸入內容的 filter，第一次 Esc 會完整清除 filter 狀態。
/// 保護目的：避免輸入框雖消失，內部卻殘留一個不可見的空 filter，造成後續 Esc 流程混亂。
fn empty_filter_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_filter_input(FilterMode::Normal);
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty filter input");

    assert!(app.filter.is_none());
    assert!(!app.panes.get(&1).expect("pane").has_active_filter());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證空白 Preview Search 在第一次 Esc 就會關閉，並清除 pane 上的搜尋條件。
/// 保護目的：所有共用文字輸入 UI 都必須遵守相同的空輸入快速離開規則。
fn empty_preview_search_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "alpha").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_preview_focus();
    app.open_preview_search_input();
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty preview search");

    assert!(app.preview_search.is_none());
    assert_eq!(app.status, "preview mode");
}

#[test]
/// 驗證建立檔案的 inline 輸入框尚未輸入名稱時，第一次 Esc 就會取消建立。
/// 保護目的：rename/create 使用獨立編輯流程，也必須與共用輸入器保持一致。
fn empty_create_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.start_create_entry();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel empty create input");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "create cancelled");
}

#[test]
/// 驗證空白 rename 輸入框在 Insert 模式按第一次 Esc 就會取消改名。
/// 保護目的：rename 使用獨立編輯流程，清空原檔名後也不可殘留在無意義的 Normal 模式。
fn empty_rename_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::Rename {
        pane_id: 1,
        original_name: String::from("alpha.txt"),
        buffer: String::new(),
        cursor: 0,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel empty rename input");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "rename cancelled: alpha.txt");
}

#[test]
/// 驗證空白 list find 與 global search 都能用第一次 Esc 直接回到一般列表。
/// 保護目的：兩種搜尋使用不同外層狀態機，但都必須遵守共用輸入器的快速離開規則。
fn empty_list_and_global_search_close_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_list_find_input();
    app.handle_list_find_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty list find");
    assert!(app.list_find.is_none());

    app.open_global_search().expect("open global search");
    app.handle_global_search_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty global search");
    assert!(app.global_search.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證列表面板內剛開啟的空白搜尋框，第一次 Esc 就會收起輸入框。
/// 保護目的：Help、Trash、Task、Bookmark、Zoxide 共用此流程，修正一次即可保持一致。
fn empty_panel_search_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open panel search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty panel search");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            search: PanelSearchState {
                editing: false,
                ref buffer,
            },
            ..
        }) if buffer.is_empty()
    ));
}

#[test]
/// 驗證在 Rename 與 CreateEntry 的 Normal 模式下，按 `q` 可以直接取消並離開。
fn q_cancels_rename_and_create_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. Rename: 輸入文字後按 Esc 進入 Normal 模式，再按 q 取消
    app.pending_action = Some(PendingAction::Rename {
        pane_id: 1,
        original_name: String::from("alpha.txt"),
        buffer: String::from("new_alpha.txt"),
        cursor: 4,
        mode: RenameMode::Normal,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels rename in normal mode");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "rename cancelled: alpha.txt");

    // 2. CreateEntry: 輸入文字後按 Esc 進入 Normal 模式，再按 q 取消
    app.pending_action = Some(PendingAction::CreateEntry {
        pane_id: 1,
        buffer: String::from("some_dir/"),
        cursor: 4,
        mode: RenameMode::Normal,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels create in normal mode");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "create cancelled");
}

#[test]
/// 驗證在全域搜尋輸入框中輸入文字後，按 Esc 進入 Normal 模式，再按 q 可以直接退出全域搜尋。
fn q_cancels_global_search_input_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_global_search().expect("open global search");
    assert!(app.global_search.is_some());
    assert!(app.global_search.as_ref().unwrap().editing);

    // 輸入 'k' 與 'j'
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("type k");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("type j");
    assert_eq!(app.global_search.as_ref().unwrap().buffer, "kj");
    assert_eq!(app.text_input_mode, RenameMode::Insert);

    // 按 Esc 進入 Normal 模式
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("press esc");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert!(app.global_search.is_some());
    assert!(app.global_search.as_ref().unwrap().editing);

    // 在 Normal 模式下按 q 退出全域搜尋
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("press q");
    assert!(app.global_search.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證在全域搜尋結果瀏覽清單中，按 `q` 可以直接關閉全域搜尋回到一般模式。
fn q_exits_global_search_results_and_preview() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. 搜尋結果瀏覽清單中按 q
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        task_id: None,
        mode: SearchMode::Path,
        buffer: String::from("test"),
        results: vec![GlobalSearchEntry {
            path: dir.path().join("test.txt"),
            relative_path: String::from("test.txt"),
            is_dir: false,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        }],
        selected: 0,
        editing: false,
        searched: true,
        loading: false,
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
    });

    app.handle_global_search_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels global search");
    assert!(app.global_search.is_none());
    assert_eq!(app.status, "normal mode");

    // 2. 在內容搜尋預覽模式下按 q
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        task_id: None,
        mode: SearchMode::Content,
        buffer: String::from("demo"),
        results: vec![GlobalSearchEntry {
            path: dir.path().join("test.txt"),
            relative_path: String::from("test.txt"),
            is_dir: false,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        }],
        selected: 0,
        editing: false,
        searched: true,
        loading: false,
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
    });
    if let Some(pane) = app.panes.get_mut(&1) {
        pane.set_preview_active(true);
    }

    app.handle_global_search_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q exits preview in content search");
    assert!(app.global_search.is_some());
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
}

#[test]
/// 驗證檔案預覽模式 (Tab preview) 按 `q` 或 `h` 關閉預覽回到檔案列表。
fn q_exits_file_preview_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "hello world").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_preview_focus();
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_focused(true);
    assert!(app.panes.get(&1).expect("pane").is_preview_active());

    app.handle_preview_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q exits preview focus");
    assert!(app.panes.get(&1).expect("pane").is_preview_open());
    assert!(!app.panes.get(&1).expect("pane").is_preview_focused());
    assert_eq!(app.status, "file list (preview open)");

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("Tab closes preview");
    assert!(!app.panes.get(&1).expect("pane").is_preview_open());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證視覺選取模式 (Visual Selection) 按 `q` 可以直接取消。
fn q_cancels_visual_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "hello").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("start visual selection");
    assert!(app.visual_selection.is_some());

    app.handle_visual_selection_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels visual selection");
    assert!(app.visual_selection.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證確認刪除與確認覆蓋對話框按 `q` 可以直接取消。
fn q_cancels_confirmation_dialogs() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. ConfirmDelete
    app.pending_action = Some(PendingAction::ConfirmDelete {
        pane_id: 1,
        target_name: String::from("important.txt"),
        permanent: true,
        warning_message: None,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels delete");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "delete cancelled: important.txt");

    // 2. ConfirmPasteOverwrite
    app.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
        pane_id: 1,
        target_name: String::from("target.txt"),
        entry_count: 1,
        operation: ClipboardOperation::Copy,
        conflicts: Vec::new(),
        current_index: 0,
        selected_option: 0,
        decisions: Vec::new(),
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels overwrite");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "paste cancelled: target.txt");
}

#[test]
/// 驗證 CommandMode 在 Normal 模式下按 `q` 可以直接退出。
fn q_cancels_command_mode_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_prefilled_command("rename new_name");
    assert!(app.command_mode);

    // 按 Esc 切換到 Normal 模式
    app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc enters normal mode");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert!(app.command_mode);

    // Normal 模式下按 q 關閉 command mode
    app.handle_command_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes command mode");
    assert!(!app.command_mode);
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 ListFind 在 Normal 模式下按 `q` 可以直接取消。
fn q_cancels_list_find_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_list_find_input();
    assert!(app.list_find.is_some());

    // 輸入 'a'
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type a");
    assert_eq!(app.list_find.as_ref().unwrap().buffer, "a");

    // 按 Esc 進入 Normal 模式
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc enters normal mode");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert!(app.list_find.is_some());

    // Normal 模式下按 q 關閉 list find
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes list find");
    assert!(app.list_find.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證各面板內部搜尋框（TaskPanel, TrashPanel, BookmarkList, ZoxideList）在 Normal 模式下按 `q` 關閉搜尋框。
fn q_cancels_panel_search_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. TaskPanel 內部搜尋
    app.text_input_mode = RenameMode::Insert;
    app.pending_action = Some(PendingAction::TaskPanel {
        pane_id: 1,
        selected: 0,
        search: PanelSearchState {
            buffer: String::from("copy"),
            editing: true,
        },
        marked_ids: Vec::new(),
        visual_anchor: None,
    });
    // Esc -> Normal
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    // q -> 關閉 search.editing
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes task panel search");
    if let Some(PendingAction::TaskPanel { search, .. }) = &app.pending_action {
        assert!(!search.editing);
    } else {
        panic!("expected TaskPanel");
    }

    // 2. TrashPanel 內部搜尋
    app.text_input_mode = RenameMode::Insert;
    app.pending_action = Some(PendingAction::TrashPanel {
        pane_id: 1,
        selected: 0,
        search: PanelSearchState {
            buffer: String::from("del"),
            editing: true,
        },
        marked_ids: Vec::new(),
        visual_anchor: None,
    });
    // Esc -> Normal
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    // q -> 關閉 search.editing
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes trash panel search");
    if let Some(PendingAction::TrashPanel { search, .. }) = &app.pending_action {
        assert!(!search.editing);
    } else {
        panic!("expected TrashPanel");
    }
}
