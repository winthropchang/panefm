use super::*;

#[test]
/// 驗證移到 trash 的項目可以透過 restore 命令還原。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_restore_latest_from_trash_recovers_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("restore-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");
    assert!(!file_path.exists());

    app.restore_latest_from_trash().expect("restore");

    assert!(file_path.exists());
    assert_eq!(app.status, "restored restore-me.txt");
}

#[test]
/// 驗證 trash 面板可以列出項目，並透過 Enter 還原目前選到的檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_trash_panel_lists_and_restores_entry() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("panel-restore.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");

    app.open_trash_panel().expect("open trash panel");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { selected: 0, .. })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open restore confirm");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmTrashAction { .. })
    ));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm restore from panel");

    assert!(file_path.exists());
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
    assert_eq!(app.status, "restored panel-restore.txt");
}

#[test]
/// 驗證 trash 面板可用 `d` 永久刪除目前選到的項目，且會先進確認視窗。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_trash_panel_can_delete_selected_entry_permanently() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("purge-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");

    app.open_trash_panel().expect("open trash panel");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("open delete confirm");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmTrashAction { .. })
    ));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("delete selected trash entry");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
    assert_eq!(app.trash_store.list_entries().expect("list").len(), 0);
    assert_eq!(app.status, "deleted permanently purge-me.txt");
}

#[test]
/// 驗證 trash 面板在確認刪除時仍保留原本列表狀態，取消後會回到同一個 trash 面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_trash_panel_delete_confirm_cancel_returns_to_same_trash_panel() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("cancel-delete.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");

    app.open_trash_panel().expect("open trash panel");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("open delete confirm");

    let (selected, search, marked_ids, visual_anchor) =
        trash_panel_overlay_state_from_pending_action(&app.pending_action, 1)
            .expect("trash overlay state");
    assert_eq!(selected, 0);
    assert_eq!(search.buffer, "");
    assert!(marked_ids.is_empty());
    assert_eq!(visual_anchor, None);

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel delete confirm");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel {
            pane_id: 1,
            selected: 0,
            ..
        })
    ));
    assert_eq!(app.status, "delete cancelled: cancel-delete.txt");
}

#[test]
/// 驗證 trash 面板可用 `D` 永久刪除目前篩選結果的全部項目，且會先確認。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_trash_panel_shift_d_deletes_filtered_entries() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("zzzzzz-alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "alpha").expect("alpha");
    fs::write(&beta, "beta").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm alpha");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm beta");

    app.open_trash_panel().expect("open trash panel");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start trash search");
    for _ in 0..6 {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
            .expect("type unique filter");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock trash filter");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("open delete all confirm");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmTrashAction { .. })
    ));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm clear filtered trash");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
    let remaining = app.trash_store.list_entries().expect("list remaining");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].display_name, "beta.txt");
    assert_eq!(app.status, "deleted permanently zzzzzz-alpha.txt");
}

#[test]
/// 驗證 trash 面板可用 `V` 標記多個項目，並透過 `U` 一次還原。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_trash_panel_visual_mark_restore_multiple_entries() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "alpha").expect("alpha");
    fs::write(&beta, "beta").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm first");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm second");

    app.open_trash_panel().expect("open trash");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
        .expect("start visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("extend visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
        .expect("commit visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT))
        .expect("open restore all confirm");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmTrashAction { .. })
    ));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm restore marked items");

    assert!(alpha.exists());
    assert!(beta.exists());
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
    assert_eq!(app.status, "restored 2 items");
}

#[test]
/// 驗證 trash 面板在已有 `V` 標記時，按 `u` 也會一次還原全部標記項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_trash_panel_visual_mark_lower_u_restores_multiple_entries() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("lower-u-alpha.txt");
    let beta = dir.path().join("lower-u-beta.txt");
    fs::write(&alpha, "alpha").expect("alpha");
    fs::write(&beta, "beta").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm first");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm second");

    app.open_trash_panel().expect("open trash");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
        .expect("start visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("extend visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
        .expect("commit visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
        .expect("open restore confirm");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmTrashAction { .. })
    ));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm restore marked items");

    assert!(alpha.exists());
    assert!(beta.exists());
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
    assert_eq!(app.status, "restored 2 items");
}

#[test]
/// 驗證 trash 面板在已有 `V` 標記時，按 `d` 也會一次刪除全部標記項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_trash_panel_visual_mark_lower_d_deletes_multiple_entries() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("lower-d-alpha.txt");
    let beta = dir.path().join("lower-d-beta.txt");
    fs::write(&alpha, "alpha").expect("alpha");
    fs::write(&beta, "beta").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm first");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm second");

    app.open_trash_panel().expect("open trash");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
        .expect("start visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("extend visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
        .expect("commit visual mark");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("open delete confirm");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmTrashAction { .. })
    ));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm delete marked items");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
    assert_eq!(app.trash_store.list_entries().expect("list").len(), 0);
    assert_eq!(app.status, "deleted permanently 2 items");
}

#[test]
/// 驗證從 trash 面板按 F1 打開 help 後，按 Esc 會回到原本的 trash 列表。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_from_trash_returns_to_trash_on_escape() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("from-trash-help.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");

    app.open_trash_panel().expect("open trash");
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .expect("open help from trash");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close help");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
}

#[test]
/// 驗證從 trash 打開 help 並執行 `:trash undo` 後，會回到最近的 trash 列表上下文。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_enter_from_trash_executes_undo_and_returns_to_trash() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("undo-via-help.txt");
    fs::write(&file_path, "hello").expect("file");
    let undo_index = help_entries("")
        .iter()
        .position(|entry| entry.line.command == ":trash undo")
        .expect("trash undo help entry");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");
    assert!(!file_path.exists());

    app.open_trash_panel().expect("open trash");
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .expect("open help from trash");
    for _ in 0..undo_index {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move to trash undo help entry");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute trash undo from help");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { .. })
    ));
    assert!(file_path.exists());
    assert_eq!(app.status, "restored undo-via-help.txt");
}

#[test]
/// 驗證 `tt` 會直接進入 Trash 列表，不再多開一層選單。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tt_opens_trash_panel_directly() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("open t picker");
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("open trash panel");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TrashPanel { pane_id: 1, .. })
    ));
}

#[test]
/// 驗證在 Trash 面板中永久刪除單一檔案時，會同步刪除 undoBackup 下對應的備份。
fn app_trash_delete_permanently_syncs_with_undo_backup() {
    let dir = tempdir().expect("tempdir");
    let backup_dir = crate::file_manager::undo_backup::resolve_undo_backup_dir();
    fs::create_dir_all(&backup_dir).expect("create backup dir");

    let backup_file = backup_dir.join(format!("Icon.png-{}-1.backup", std::process::id()));
    fs::write(&backup_file, "backup bytes").expect("write backup");
    assert!(backup_file.exists());

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let test_file = dir.path().join("Icon.png");
    fs::write(&test_file, "trashed bytes").expect("write test file");
    app.trash_store
        .trash_path(&test_file, "Icon.png")
        .expect("trash file");

    let entries = app.trash_store.list_entries().expect("list");
    assert_eq!(entries.len(), 1);
    let target_id = entries[0].id.clone();

    // 在 Trash 面板中永久刪除該項目
    app.delete_trash_ids_in_panel(
        1,
        &[target_id],
        PanelSearchState::default(),
        0,
        "Icon.png",
        1,
    )
    .expect("delete in panel");

    // 驗證 Trash 已清空，且 undoBackup 下的 Icon.png 備份也被同步刪除
    assert_eq!(app.trash_store.list_entries().expect("list").len(), 0);
    assert!(!backup_file.exists());
}

#[test]
/// 驗證 trash 確認視窗會記住原本所屬的 panel，讓 UI 能畫回同一個列表內。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn trash_confirm_panel_id_returns_source_panel() {
    let action = TrashConfirmAction::DeleteFromPanel {
        pane_id: 7,
        target_ids: vec![String::from("trash-id")],
        search: PanelSearchState {
            buffer: String::from("demo"),
            editing: false,
        },
        selected: 2,
    };

    assert_eq!(trash_confirm_panel_id(&action), Some(7));
}

#[test]
/// 驗證 trash 確認視窗也能還原底層列表需要的搜尋與標記狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn trash_confirm_overlay_state_preserves_trash_context() {
    let pending = PendingAction::ConfirmTrashAction {
        action: TrashConfirmAction::RestoreFromPanel {
            pane_id: 3,
            target_ids: vec![String::from("trash-id")],
            search: PanelSearchState {
                buffer: String::from("abc"),
                editing: false,
            },
            selected: 2,
        },
        target_name: String::from("alpha.txt"),
        entry_count: 1,
        marked_ids: vec![String::from("trash-id"), String::from("trash-id-2")],
        visual_anchor: Some(1),
    };

    let (selected, search, marked_ids, visual_anchor) =
        trash_panel_overlay_state_from_pending_action(&Some(pending), 3).expect("overlay state");

    assert_eq!(selected, 2);
    assert_eq!(search.buffer, "abc");
    assert!(!search.editing);
    assert_eq!(marked_ids.len(), 2);
    assert_eq!(visual_anchor, Some(1));
}
