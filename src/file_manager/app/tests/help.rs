use super::*;

#[test]
/// 驗證 help 面板中需要參數的命令，按 Enter 後會打開預填命令，而不是直接執行空參數。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_argument_command_opens_prefilled_command_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start help search");
    for ch in ['C', 't', 'r', 'l', '-', 'p'] {
        let modifiers = if ch.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), modifiers))
            .expect("type help query");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock help search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open panel command");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "panel ");
    assert_eq!(app.status, "command mode");
}

#[test]
/// 驗證 F1 說明面板可以打開，並在面板內用 `f` 進行搜尋。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_supports_filtering() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .expect("open help");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start help search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock help search");

    match app.pending_action.as_ref() {
        Some(PendingAction::HelpPanel { search, .. }) => {
            assert_eq!(search.buffer, "res");
            assert!(!search.editing);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
    let matches = help_entries("res").len();
    assert!(
        matches > 1,
        "fuzzy filter should find non-contiguous matches"
    );
    assert_eq!(app.status, format!("help: res ({matches})"));
}

#[test]
/// 驗證 help 面板搜尋輸入中的 `Tab` 不會誤套用 command hint。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_search_tab_does_not_apply_command_autocomplete() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .expect("open help");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start help search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab in help search");

    match app.pending_action.as_ref() {
        Some(PendingAction::HelpPanel {
            search, selected, ..
        }) => {
            assert_eq!(search.buffer, "re");
            assert!(search.editing);
            assert_eq!(*selected, 0);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
    assert_eq!(
        app.status,
        format!("help search: re ({})", help_entries("re").len())
    );
}

#[test]
/// 驗證 help 面板已開啟時，再按一次 `~` 會直接關閉回 normal mode。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tilde_toggles_help_panel_closed() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE))
        .expect("open help with tilde");
    app.handle_key(KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE))
        .expect("close help with tilde");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證某些終端把 `~` 回報成 `Shift+\`` 時，也能正確打開 help 面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_backtick_opens_help_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('`'), KeyModifiers::SHIFT))
        .expect("open help with shift backtick");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));
}

#[test]
/// 驗證 help 面板按下 Enter 後，會直接切到對應的互動模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_enter_executes_selected_action() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_help_panel();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute rename from help");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::Rename { .. })
    ));
}

#[test]
/// 驗證 help 面板在列表模式下按 `h` 會和 `Esc` 一樣關閉，保持與 `l` 的左右對稱操作。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_h_closes_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("close help with h");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 help 面板支援 `J / K` 與 `Ctrl-d / Ctrl-u`，讓大步長與分頁移動可在暫時列表中共用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_supports_fast_and_page_navigation() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
        .expect("help fast down");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 5),
        ref other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("help page down");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 15),
        ref other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("help page up");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 5),
        ref other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
        .expect("help fast up");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 0),
        ref other => panic!("unexpected pending action: {other:?}"),
    }
}

#[test]
/// 驗證 help 面板中的 `:delete` 會保留 `d` 快捷鍵，並透過 Enter 進入刪除確認。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_delete_entry_matches_delete_behavior() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("delete-from-help.txt");
    fs::write(&file_path, "hello").expect("file");

    let entries = help_entries("");
    let delete_entry = entries
        .iter()
        .find(|entry| entry.line.command == ":delete")
        .expect("delete help entry");
    let trash_entry = entries
        .iter()
        .find(|entry| entry.line.command == ":trash")
        .expect("trash help entry");
    let delete_index = entries
        .iter()
        .position(|entry| entry.line.command == ":delete")
        .expect("delete help index");
    assert_eq!(delete_entry.line.shortcut, "d");
    assert_eq!(trash_entry.line.shortcut, "tt");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_help_panel();

    for _ in 0..delete_index {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move to delete help entry");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute delete from help");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete { .. })
    ));
}

#[test]
/// 驗證按下 ? 鍵在 Normal 模式與 Task 面板能分別開啟對應情境的 Cheatsheet，且 Esc 能無縫返回。
fn cheatsheet_opens_context_specific_help_and_restores_state() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. 在 Normal 模式按 ? 鍵
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
        .expect("press ? in normal mode");
    match &app.pending_action {
        Some(PendingAction::HelpPanel {
            custom_title,
            custom_entries,
            ..
        }) => {
            assert!(custom_title.as_ref().unwrap().contains("Normal Mode"));
            let entries = custom_entries.as_ref().unwrap();
            assert!(entries.iter().any(|e| e.line.shortcut.contains("j / k")));
            assert!(entries.iter().any(|e| e.line.command == "rename"));
        }
        other => panic!("expected Cheatsheet HelpPanel, got {other:?}"),
    }

    // 按 Esc 退出 Cheatsheet 返回 Normal 模式
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("press Esc to exit cheatsheet");
    assert!(app.pending_action.is_none());

    // 2. 開啟 TaskPanel 後按 ? 鍵
    app.open_task_panel();
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TaskPanel { .. })
    ));

    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
        .expect("press ? in task panel");
    match &app.pending_action {
        Some(PendingAction::HelpPanel {
            custom_title,
            custom_entries,
            ..
        }) => {
            assert!(custom_title.as_ref().unwrap().contains("Task Panel"));
            let entries = custom_entries.as_ref().unwrap();
            assert!(entries.iter().any(|e| e.line.shortcut.contains("d")));
            assert!(entries.iter().any(|e| e.line.shortcut.contains("x / c")));
            assert!(entries.iter().any(|e| e.line.shortcut.contains("v / V")));
        }
        other => panic!("expected Cheatsheet HelpPanel, got {other:?}"),
    }

    // 按 ? 鍵再次關閉 Cheatsheet，必須精準返回 TaskPanel
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
        .expect("press ? to exit cheatsheet");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TaskPanel { .. })
    ));

    // 3. 驗證 :cheatsheet 與 :cheat 指令
    app.pending_action = None;
    app.execute_command("cheatsheet").expect("exec :cheatsheet");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            custom_title: Some(_),
            ..
        })
    ));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    app.execute_command("cheat").expect("exec :cheat");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            custom_title: Some(_),
            ..
        })
    ));
}

#[test]
/// 驗證不同終端回報的問號鍵位格式（如帶有 Shift 或中文全形問號）都能順利開啟 Cheatsheet。
fn cheatsheet_opens_with_various_question_mark_formats() {
    let dir = tempdir().expect("tempdir");

    // 1. Shift + ?
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT))
        .expect("press Shift+? in normal mode");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));

    // 2. Shift + /
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::SHIFT))
        .expect("press Shift+/ in normal mode");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));

    // 3. 全形問號 '？'
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('？'), KeyModifiers::NONE))
        .expect("press full-width ？ in normal mode");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));

    // 4. 單純的 '/'（無 Shift）不應觸發 cheatsheet，而是觸發 list find
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("press / in normal mode");
    assert!(!matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));
}

#[test]
/// 驗證在 GoPicker (按 g) 下按 ? 開啟 Cheatsheet 後，按 Enter 選擇項目能正確執行跳轉命令。
fn cheatsheet_from_go_picker_executes_jump_command() {
    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let downloads = home.join("Downloads");
    fs::create_dir_all(&downloads).expect("downloads");

    let original_home = std::env::var_os("HOME");
    let original_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        // 1. 先按 g 打開 GoPicker
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("open go picker");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::GoPicker { .. })
        ));

        // 2. 按 ? 打開 GoPicker 專屬 Cheatsheet
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT))
            .expect("open cheatsheet from go picker");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel { .. })
        ));

        // 3. 移動到 downloads (索引 2: documents=0, desktop=1, downloads=2)
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("down to desktop");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("down to downloads");

        // 4. 按 Enter 執行所選命令
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute downloads jump");

        // 應成功跳轉至 Downloads 目錄，且退回 Normal mode（不會卡在 GoPicker）
        assert_eq!(app.panes.get(&1).expect("pane").cwd, downloads);
        assert!(app.pending_action.is_none());
    }

    unsafe {
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }
}

#[test]
/// 驗證 Cheatsheet 支援以 f 鍵開啟搜尋並在情境清單內進行模糊過濾。
fn cheatsheet_search_filters_within_context_entries() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_task_panel();
    app.open_cheatsheet_from_current();

    // 在 Task Cheatsheet 內按 f 搜尋 "cancel"
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("type c");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type a");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("type n");

    match &app.pending_action {
        Some(PendingAction::HelpPanel {
            search,
            custom_entries,
            ..
        }) => {
            assert_eq!(search.buffer, "can");
            let filtered =
                filter_custom_help_entries(custom_entries.as_ref().unwrap(), &search.buffer);
            assert!(!filtered.is_empty());
            assert!(filtered.iter().any(|e| e.line.description.contains("取消")));
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
/// 驗證在全域搜尋（s/S）面板中按下 ? 鍵只會顯示搜尋專屬快捷鍵，不會出現 create(a) 或 delete(d)。
fn cheatsheet_in_global_search_shows_only_search_navigation_keys() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 模擬開啟 global search (s)
    app.open_global_search().expect("open global search");
    assert!(app.global_search.is_some());

    // 在 global search 狀態下按 ? 鍵
    app.open_cheatsheet_from_current();

    match &app.pending_action {
        Some(PendingAction::HelpPanel {
            custom_title,
            custom_entries,
            ..
        }) => {
            let title = custom_title.as_ref().unwrap();
            assert!(
                title.contains("Global Search"),
                "title should be Global Search, got: {title}"
            );
            let entries = custom_entries.as_ref().unwrap();
            let commands: Vec<&str> = entries.iter().map(|e| e.line.command.as_str()).collect();

            // 確保包含搜尋導覽與預覽快捷鍵
            assert!(commands.contains(&"move"));
            assert!(commands.contains(&"open"));
            assert!(commands.contains(&"filter"));
            assert!(commands.contains(&"re-edit"));
            assert!(commands.contains(&"exit"));

            // 確保「絕對不包含」無法在搜尋面板執行的 normal 模式指令
            assert!(
                !commands.contains(&"create"),
                "cheatsheet should NOT contain create"
            );
            assert!(
                !commands.contains(&"trash"),
                "cheatsheet should NOT contain trash"
            );
            assert!(
                !commands.contains(&"delete!"),
                "cheatsheet should NOT contain delete!"
            );
            assert!(
                !commands.contains(&"rename"),
                "cheatsheet should NOT contain rename"
            );
            assert!(
                !commands.contains(&"paste"),
                "cheatsheet should NOT contain paste"
            );
            assert!(
                !commands.contains(&"undo"),
                "cheatsheet should NOT contain undo"
            );
        }
        other => panic!("expected Cheatsheet HelpPanel, got {other:?}"),
    }

    // 按 Esc 退出 Cheatsheet，必須精準回復 global_search
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("press Esc to exit cheatsheet");
    assert!(
        app.global_search.is_some(),
        "global_search state should be restored"
    );
}

#[test]
/// 驗證所有 ContextHelpKind 都擁有專屬、非空的 Cheatsheet 定義。
fn cheatsheet_covers_all_context_kinds() {
    let all_kinds = [
        ContextHelpKind::Normal,
        ContextHelpKind::GlobalSearch,
        ContextHelpKind::ListFind,
        ContextHelpKind::TaskPanel,
        ContextHelpKind::TrashPanel,
        ContextHelpKind::DiffMatrix,
        ContextHelpKind::VisualSelection,
        ContextHelpKind::BookmarkPicker,
        ContextHelpKind::BookmarkList,
        ContextHelpKind::ZoxideList,
        ContextHelpKind::WindowPicker,
        ContextHelpKind::WindowResize,
        ContextHelpKind::SortPicker,
        ContextHelpKind::GoPicker,
        ContextHelpKind::LineModePicker,
        ContextHelpKind::ThemePicker,
        ContextHelpKind::CommandMode,
        ContextHelpKind::Filter,
        ContextHelpKind::Preview,
        ContextHelpKind::ToolPanel,
        ContextHelpKind::RegexRename,
        ContextHelpKind::Rename,
        ContextHelpKind::CreateEntry,
        ContextHelpKind::ConfirmAction,
        ContextHelpKind::CopyPicker,
        ContextHelpKind::OpenPicker,
    ];

    for kind in all_kinds {
        let (title, entries) = context_cheatsheet_entries(kind);
        assert!(!title.is_empty(), "title for {kind:?} must not be empty");
        assert!(
            !entries.is_empty(),
            "entries for {kind:?} must not be empty"
        );
        for entry in &entries {
            assert!(
                !entry.line.command.is_empty(),
                "command in {kind:?} must not be empty"
            );
            assert!(
                !entry.line.shortcut.is_empty(),
                "shortcut in {kind:?} must not be empty"
            );
            assert!(
                !entry.line.description.is_empty(),
                "description in {kind:?} must not be empty"
            );
        }
    }
}
