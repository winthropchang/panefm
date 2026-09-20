use super::*;

#[test]
/// 驗證狀態列只會把錯誤類訊息判斷為危險色，一般通知不會被誤標紅。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn status_is_error_distinguishes_errors_from_notifications() {
    assert!(super::status_is_error("failed to open file"));
    assert!(super::status_is_error("usage: reg <pattern> <replace>"));
    assert!(super::status_is_error(
        "rename-regex: resolve conflicts before apply"
    ));
    assert!(!super::status_is_error("opened directory"));
    assert!(!super::status_is_error("rename-regex: renamed 2 items"));
    assert!(!super::status_is_error("trash cancelled: note.txt"));
}

#[test]
/// 驗證底部快捷鍵列永遠先顯示 Help，再依照移動、開啟與書籤等使用頻率排列。
/// 保護目的：避免新增命令時又依 command 定義順序插入提示，讓使用者在不知道按鍵時
/// 看不到最重要的 `~/F1 help` 入口。
fn status_shortcut_hints_keep_help_first_and_follow_usage_priority() {
    let hints = super::status_shortcut_hints();

    assert_eq!((hints[0].key, hints[0].label), ("~/F1", "help"));
    assert_eq!((hints[1].key, hints[1].label), ("hjkl", "move"));
    assert_eq!((hints[2].key, hints[2].label), ("Enter", "open"));
    assert_eq!((hints[3].key, hints[3].label), ("b", "bookmark"));
}

#[test]
/// 驗證窄 terminal 只保留能完整放下的高優先快捷鍵，並固定保留右側版本號。
/// 保護目的：多 panel 或小視窗會縮短 status bar；此測試避免重新出現只看得到半個
/// 快捷鍵名稱，並確認寬畫面使用的是目前正確的 Tab preview 與 P 覆蓋貼上提示。
fn status_shortcut_line_drops_low_priority_items_instead_of_clipping() {
    let theme = Theme::default_theme();
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let narrow = super::status_shortcut_line(31, theme, super::status_shortcut_hints());
    let narrow_text = narrow
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let wide = super::status_shortcut_line(u16::MAX, theme, super::status_shortcut_hints());
    let wide_text = wide
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(narrow_text.len(), 31);
    assert!(narrow_text.starts_with("~/F1 help"));
    assert!(narrow_text.ends_with(&version));
    assert!(wide_text.contains("Tab preview"));
    assert!(wide_text.contains("p/P paste/overwrite"));
    assert!(!wide_text.contains("P preview"));
    assert!(wide_text.ends_with(&version));

    let version_only =
        super::status_shortcut_line(version.len() as u16, theme, super::status_shortcut_hints());
    let version_only_text = version_only
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert_eq!(version_only_text, version);
}

#[test]
/// 驗證貼上錯誤會把摘要與完整診斷拆行，並保留 destination 及原始 OS error。
/// 保護目的：避免 SMB/UNC 長路徑再次把最重要的錯誤尾端截掉，導致公司環境無法除錯。
fn paste_failure_status_preserves_destination_and_os_error() {
    let destination = std::path::Path::new(r"\\server\shared\department\release\large-archive.zip");
    let error = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Access is denied. (os error 5)",
    );

    let status = super::paste_failure_status("large-archive.zip", destination, &error);
    let mut lines = status.lines();

    assert_eq!(lines.next(), Some("paste failed for large-archive.zip"));
    let detail = lines.next().expect("diagnostic detail line");
    assert!(detail.contains(destination.to_string_lossy().as_ref()));
    assert!(detail.contains("OS error: Access is denied. (os error 5)"));
    assert_eq!(lines.next(), None);
    assert!(super::status_is_error(&status));
}

#[test]
/// 驗證長錯誤會依終端寬度增加 status area，高度不足時則遵守畫面上限。
/// 保護目的：避免 layout 重構後又把 status 固定成一行，或讓錯誤區吃掉整個檔案列表。
fn status_area_height_wraps_long_errors_and_preserves_short_notifications() {
    let long_error = concat!(
        "paste failed for archive.zip\n",
        "destination: \\\\server\\shared\\department\\release\\archive.zip | ",
        "OS error: The network name cannot be found. (os error 67)"
    );

    let short_status = super::wrap_status_text("opened directory", 80);
    let wrapped_error = super::wrap_status_text(long_error, 40);
    let narrow_error = super::wrap_status_text(long_error, 20);

    assert_eq!(super::status_area_height(&short_status, 20), 1);
    assert!(super::status_area_height(&wrapped_error, 20) >= 3);
    assert_eq!(super::status_area_height(&narrow_error, 2), 2);
    assert!(wrapped_error.contains("OS error"));
}

#[test]
/// 驗證 status 換行使用 terminal cell 寬度，而不是 UTF-8 byte 或 Unicode 字元數。
/// 保護目的：公司 SMB 路徑可能包含中文，必須避免配置高度不足而截掉錯誤內容。
fn status_wrapping_accounts_for_wide_cjk_characters() {
    let wrapped = super::wrap_status_text("錯誤位置", 4);

    assert_eq!(wrapped, "錯誤\n位置");
    assert_eq!(super::status_area_height(&wrapped, 10), 2);
}

#[test]
/// 驗證缺少搜尋工具時會顯示正確搜尋類型，並引導使用者打開 status 面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn missing_search_tool_status_names_mode_and_dependency_panel() {
    assert_eq!(
        missing_search_tool_status(SearchMode::Path, "fd"),
        "global search requires fd; run :status"
    );
    assert_eq!(
        missing_search_tool_status(SearchMode::Content, "rg"),
        "content search requires rg; run :status"
    );
}

#[test]
/// 驗證檔名與內容搜尋標題會明確顯示用途及實際使用的外部工具。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn search_panel_titles_name_search_tool() {
    assert_eq!(
        SearchMode::Path.panel_title(true),
        " Global search file by fd "
    );
    assert_eq!(
        SearchMode::Content.panel_title(true),
        " Global search content by rg "
    );
    assert_eq!(
        SearchMode::Content.panel_title(false),
        " Global search content by rg "
    );
}

#[test]
/// 驗證 `:status` 會在目前 focus panel 顯示外部工具狀態，且 Enter 可關閉查詢面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_status_command_opens_dependency_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.execute_command("status").expect("open status panel");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ToolPanel {
            pane_id: 1,
            selected: 0
        })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("close status panel");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "dependency panel closed");
}

#[test]
/// 驗證底部快捷鍵列會依據目前畫面／模式動態調整，且前兩個提示一律固定為 Help 與 Cheatsheet。
fn active_status_shortcut_hints_adapts_to_current_view_and_always_starts_with_help() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. Normal mode
    let hints = app.active_status_shortcut_hints();
    assert_eq!(hints[0].key, "~/F1");
    assert_eq!(hints[0].label, "help");
    assert_eq!(hints[1].key, "?");
    assert_eq!(hints[1].label, "cheat");
    let keys: Vec<&str> = hints.iter().map(|h| h.key).collect();
    assert!(keys.contains(&"hjkl"));
    assert!(keys.contains(&"Enter"));
    assert!(keys.contains(&"y"));
    assert!(keys.contains(&"v"));

    // 2. Command mode (:)
    app.execute_command("").unwrap();
    app.command_mode = true;
    let cmd_hints = app.active_status_shortcut_hints();
    assert_eq!(cmd_hints[0].key, "~/F1");
    assert_eq!(cmd_hints[0].label, "help");
    assert_eq!(cmd_hints[1].key, "?");
    assert_eq!(cmd_hints[1].label, "cheat");
    let cmd_keys: Vec<&str> = cmd_hints.iter().map(|h| h.key).collect();
    assert_eq!(cmd_keys, vec!["~/F1", "?", "Enter", "Tab", "Esc"]);
    app.command_mode = false;

    // 3. Task panel mode
    app.open_task_panel();
    let task_hints = app.active_status_shortcut_hints();
    assert_eq!(task_hints[0].key, "~/F1");
    assert_eq!(task_hints[0].label, "help");
    assert_eq!(task_hints[1].key, "?");
    assert_eq!(task_hints[1].label, "cheat");
    let task_keys: Vec<&str> = task_hints.iter().map(|h| h.key).collect();
    assert!(task_keys.contains(&"v"));
    assert!(task_keys.contains(&"Space"));
    assert!(task_keys.contains(&"d"));
    assert!(task_keys.contains(&"D"));
    assert!(task_keys.contains(&"x/c"));

    // 4. Trash panel mode
    let _ = app.open_trash_panel();
    let trash_hints = app.active_status_shortcut_hints();
    assert_eq!(trash_hints[0].key, "~/F1");
    assert_eq!(trash_hints[0].label, "help");
    assert_eq!(trash_hints[1].key, "?");
    assert_eq!(trash_hints[1].label, "cheat");
    let trash_keys: Vec<&str> = trash_hints.iter().map(|h| h.key).collect();
    assert!(trash_keys.contains(&"u"));
    assert!(trash_keys.contains(&"U"));
    assert!(trash_keys.contains(&"d"));
    assert!(trash_keys.contains(&"D"));

    // 5. Diff Matrix mode
    app.pending_action = Some(PendingAction::DiffMatrix(
        crate::file_manager::diff::DiffMatrixState::new_loading(
            vec![1, 2],
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            vec!["1".into(), "2".into()],
        ),
    ));
    let diff_hints = app.active_status_shortcut_hints();
    assert_eq!(diff_hints[0].key, "~/F1");
    assert_eq!(diff_hints[0].label, "help");
    assert_eq!(diff_hints[1].key, "?");
    assert_eq!(diff_hints[1].label, "cheat");
    let diff_keys: Vec<&str> = diff_hints.iter().map(|h| h.key).collect();
    assert!(diff_keys.contains(&"Enter"));
    assert!(diff_keys.contains(&"i"));
    assert!(diff_keys.contains(&"."));
    assert!(diff_keys.contains(&"r"));

    // 6. Window picker (w)
    app.pending_action = Some(PendingAction::WindowPicker { pane_id: 1 });
    let win_hints = app.active_status_shortcut_hints();
    assert_eq!(win_hints[0].key, "~/F1");
    assert_eq!(win_hints[0].label, "help");
    assert_eq!(win_hints[1].key, "?");
    assert_eq!(win_hints[1].label, "cheat");
    let win_keys: Vec<&str> = win_hints.iter().map(|h| h.key).collect();
    assert!(win_keys.contains(&"s/v"));
    assert!(win_keys.contains(&"d"));
    assert!(win_keys.contains(&"t"));

    // 7. Visual selection mode
    app.pending_action = None;
    app.visual_selection = Some(super::VisualSelectionState {
        pane_id: 1,
        anchor: 0,
        current: 1,
    });
    let visual_hints = app.active_status_shortcut_hints();
    assert_eq!(visual_hints[0].key, "~/F1");
    assert_eq!(visual_hints[0].label, "help");
    assert_eq!(visual_hints[1].key, "?");
    assert_eq!(visual_hints[1].label, "cheat");
    let visual_keys: Vec<&str> = visual_hints.iter().map(|h| h.key).collect();
    assert!(visual_keys.contains(&"j/k"));
    assert!(visual_keys.contains(&"y"));
    assert!(visual_keys.contains(&"x"));
    assert!(visual_keys.contains(&"d"));
}
