use super::*;

#[test]
/// 驗證按下 `b` 會先打開書籤功能面板，再用 `a` 自動分配代號存書籤。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_picker_saves_with_auto_key() {
    let dir = tempdir().expect("tempdir");
    let docs = dir.path().join("docs");
    fs::create_dir(&docs).expect("docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .go_to_path(&docs)
        .expect("go docs");

    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("open bookmark picker");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::BookmarkPicker { pane_id: 1 })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("add bookmark");

    assert_eq!(app.status, format!("bookmark [a] = {}", docs.display()));
    assert!(
        fs::read_to_string(dir.path().join("bookmark.toml"))
            .expect("bookmark file")
            .contains("a =")
    );
}

#[test]
/// 驗證仍可用 `'{key}` 直接跳回既有書籤，保留快速單鍵 workflow。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_direct_jump_still_works() {
    let dir = tempdir().expect("tempdir");
    let docs = dir.path().join("docs");
    let src = dir.path().join("src");
    fs::create_dir(&docs).expect("docs");
    fs::create_dir(&src).expect("src");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!("a = \"{}\"\n", docs.to_string_lossy().replace('\\', "/")),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .go_to_path(&src)
        .expect("go src");

    app.handle_key(KeyEvent::new(KeyCode::Char('\''), KeyModifiers::NONE))
        .expect("start bookmark jump");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("jump bookmark");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
    assert_eq!(app.status, "jumped to bookmark [a]");
}

#[test]
/// 驗證 `bookmark.toml` 中既有的書籤可以在啟動後直接用命令跳轉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_jump_command_uses_bookmark_file() {
    let dir = tempdir().expect("tempdir");
    let docs = dir.path().join("docs");
    fs::create_dir(&docs).expect("docs");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!("d = \"{}\"\n", docs.to_string_lossy().replace('\\', "/")),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("bookmark jump d")
        .expect("jump command");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
    assert_eq!(app.status, "jumped to bookmark [d]");
}

#[test]
/// 驗證經由 `goto smb://...` 進入中文 SMB 目錄後，書籤檔仍保存 encoded URI，狀態列則顯示可讀中文。
/// 保護目的：避免改善 Bookmark UI 時把解碼後文字寫回檔案，導致重新啟動後無法可靠跳轉 SMB。
fn app_bookmark_set_persists_smb_location_after_goto() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_docs = mount_root.join("shared").join("網路事業部").join("otto");
    fs::create_dir_all(&share_docs).expect("share docs");
    let encoded = "smb://192.0.2.10/shared/%E7%B6%B2%E8%B7%AF%E4%BA%8B%E6%A5%AD%E9%83%A8/otto";

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root(encoded, &mount_root)
        .expect("goto smb");
    app.set_bookmark('s').expect("set bookmark");

    let bookmark_file =
        fs::read_to_string(dir.path().join("bookmark.toml")).expect("bookmark file");
    assert!(bookmark_file.contains(encoded));
    assert_eq!(
        app.status,
        "bookmark [s] = smb://192.0.2.10/shared/網路事業部/otto"
    );
}

#[test]
/// 驗證 Bookmark 彈窗與其模糊 filter 都使用解碼後的中文 SMB 路徑。
/// 保護目的：確保使用者看得到並能以中文搜尋書籤，同時列表背後仍保留可供跳轉的原始 target。
fn bookmark_list_displays_and_filters_decoded_smb_path() {
    let encoded = "smb://192.0.2.10/shared/%E7%B6%B2%E8%B7%AF%E4%BA%8B%E6%A5%AD%E9%83%A8/otto";
    let entries = vec![BookmarkEntry {
        key: 's',
        target: BookmarkTarget::SmbLocation(encoded.to_string()),
    }];

    let lines = bookmark_panel_lines(entries.clone());
    assert_eq!(lines[0].path, "smb://192.0.2.10/shared/網路事業部/otto");
    let filtered = filtered_bookmark_entries(entries, "網事");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].target.as_storage_value(), encoded);
}

#[test]
/// 驗證 SMB 書籤在跳轉時會自動走 SMB 掛載／進入流程，成功後直接切到目標目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_jump_to_smb_bookmark_enters_target() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_docs = mount_root.join("shared").join("docs");
    fs::create_dir_all(&share_docs).expect("share docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.jump_to_bookmark_target_with_mount_root(
        1,
        's',
        &BookmarkTarget::SmbLocation(String::from("smb://192.0.2.10/shared/docs")),
        &mount_root,
    )
    .expect("jump smb bookmark");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, share_docs);
    assert_eq!(app.status, "jumped to bookmark [s]");
}

#[test]
/// 驗證 `:bookmark list` 會打開彈窗，並可用 Enter 跳到選中的書籤。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_list_popup_opens_and_jumps() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!(
            "a = \"{}\"\nb = \"{}\"\n",
            alpha.to_string_lossy().replace('\\', "/"),
            beta.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("bookmark list").expect("open list");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::BookmarkList {
            pane_id: 1,
            selected: 0,
            mode: BookmarkListMode::Jump,
            ..
        })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open bookmark");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, beta);
    assert_eq!(app.status, "jumped to bookmark [b]");
}

#[test]
/// 驗證書籤列表會綁在開啟它的 pane 上，從第二個 pane 打開時也只影響第二個 pane。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_list_is_scoped_to_focused_pane() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!(
            "a = \"{}\"\nb = \"{}\"\n",
            alpha.to_string_lossy().replace('\\', "/"),
            beta.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    assert_eq!(app.focused_pane, 2);

    app.execute_command("bookmark list").expect("open list");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::BookmarkList {
            pane_id: 2,
            selected: 0,
            mode: BookmarkListMode::Jump,
            ..
        })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open bookmark");

    assert_eq!(app.panes.get(&2).expect("pane").cwd, beta);
    assert_ne!(app.panes.get(&1).expect("pane").cwd, beta);
}

#[test]
/// 驗證按下 `b` 再按 `d` 會進入刪除列表，並可按對應書籤鍵直接刪除。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_delete_mode_removes_entry_by_matching_key() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!(
            "a = \"{}\"\nb = \"{}\"\n",
            alpha.to_string_lossy().replace('\\', "/"),
            beta.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("open bookmark picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("open delete list");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::BookmarkList {
            pane_id: 1,
            selected: 0,
            mode: BookmarkListMode::Delete,
            ..
        })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("delete bookmark by key");

    assert_eq!(app.status, "bookmark [b] deleted");
    let content = fs::read_to_string(dir.path().join("bookmark.toml")).expect("bookmark file");
    assert!(content.contains("a = "));
    assert!(!content.contains("b = "));
}

#[test]
/// 驗證書籤刪除列表可用游標移動後按 Enter 刪除選中的書籤。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_delete_mode_removes_selected_entry_with_enter() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!(
            "a = \"{}\"\nb = \"{}\"\n",
            alpha.to_string_lossy().replace('\\', "/"),
            beta.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("bookmark delete")
        .expect("open delete list");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("delete selected bookmark");

    assert_eq!(app.status, "bookmark [b] deleted");
    let content = fs::read_to_string(dir.path().join("bookmark.toml")).expect("bookmark file");
    assert!(content.contains("a = "));
    assert!(!content.contains("b = "));
}

#[test]
/// 驗證按下 `b` 再按 `D` 會直接清空全部書籤。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_picker_can_delete_all_bookmarks() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    fs::create_dir(&alpha).expect("alpha");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!("a = \"{}\"\n", alpha.to_string_lossy().replace('\\', "/")),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("open bookmark picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("delete all bookmarks");

    assert_eq!(app.status, "all bookmarks deleted");
    assert!(
        app.bookmark_store.list().is_empty(),
        "bookmark store should be empty after clear"
    );
}

#[test]
/// 驗證書籤功能面板打開後，再按一次 `b` 會直接關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_picker_b_toggles_closed() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("open bookmark picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("toggle close bookmark picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證按下 `Z` 會直接打開 zoxide 目錄列表。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_z_opens_zoxide_list() {
    let dir = tempdir().expect("tempdir");
    let docs = dir.path().join("docs");
    fs::create_dir(&docs).expect("docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.go_to_path_and_track(1, &docs).expect("go docs");

    app.handle_key(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT))
        .expect("open zoxide list");

    assert!(matches!(
        &app.pending_action,
        Some(PendingAction::ZoxideList {
            pane_id: 1,
            selected: 0,
            entries,
            ..
        }) if !entries.is_empty()
    ));
}

#[test]
/// 驗證書籤列表支援 `f` 搜尋，並可直接打開過濾後唯一保留的書籤。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bookmark_list_supports_filtering() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");
    fs::write(
        dir.path().join("bookmark.toml"),
        format!(
            "a = \"{}\"\nb = \"{}\"\n",
            alpha.to_string_lossy().replace('\\', "/"),
            beta.to_string_lossy().replace('\\', "/")
        ),
    )
    .expect("bookmark file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_bookmark_list();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start bookmark filter");
    for ch in ['b', 'e', 't', 'a'] {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type bookmark query");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock bookmark filter");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open filtered bookmark");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, beta);
    assert_eq!(app.status, "jumped to bookmark [b]");
}

#[test]
/// 驗證在不同工作目錄開啟 PaneFM 時，書籤與設定檔會寫入執行檔/基礎目錄，而不是寫入瀏覽目錄。
/// 保護目的：避免在無寫入權限目錄（例如 System32）或隨機目錄啟動時，將程式資料寫入工作目錄造成錯誤或檔案污染。
fn app_writes_bookmarks_and_configs_to_base_dir_not_cwd() {
    let app_dir = tempdir().expect("app tempdir");
    let browse_dir = tempdir().expect("browse tempdir");

    let loaded = crate::config::load_config(app_dir.path()).expect("load config");
    assert_eq!(loaded.base_dir, app_dir.path());

    let mut app = App::new(browse_dir.path().to_path_buf(), loaded).expect("app");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, browse_dir.path());

    // 新增書籤
    app.add_bookmark_with_auto_key(1).expect("add bookmark");

    // 驗證書籤檔案寫入到 app_dir，且 browse_dir 中沒有產生 bookmark.toml
    let app_bookmark_path = app_dir.path().join("bookmark.toml");
    let browse_bookmark_path = browse_dir.path().join("bookmark.toml");
    assert!(
        app_bookmark_path.exists(),
        "bookmark.toml should be written in app_dir"
    );
    assert!(
        !browse_bookmark_path.exists(),
        "bookmark.toml should NOT be written in browse_dir"
    );

    // 切換主題並持久化
    app.set_theme_by_name("nord");

    // 驗證設定檔寫入到 app_dir，且 browse_dir 中沒有產生 config.toml
    let app_config_path = app_dir.path().join("config.toml");
    let browse_config_path = browse_dir.path().join("config.toml");
    assert!(
        app_config_path.exists(),
        "config.toml should be written in app_dir"
    );
    assert!(
        !browse_config_path.exists(),
        "config.toml should NOT be written in browse_dir"
    );
}

#[test]
/// 驗證 zoxide 列表支援 `f` 搜尋，並可跳到過濾後唯一保留的目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_zoxide_list_supports_filtering() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.go_to_path_and_track(1, &alpha).expect("go alpha");
    app.go_to_path_and_track(1, &beta).expect("go beta");
    app.open_zoxide_list();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start zoxide filter");
    for ch in ['b', 'e', 't', 'a'] {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type zoxide query");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock zoxide filter");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open filtered zoxide path");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, beta);
    assert_eq!(app.status, format!("jumped via zoxide: {}", beta.display()));
}
