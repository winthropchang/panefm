use super::*;

#[test]
/// 驗證 preview mode 中的 `/` 會打開搜尋輸入框，並在輸入時立即更新搜尋結果。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_search_opens_and_tracks_matches() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("notes.txt"),
        "alpha\nbeta\ngamma\nbeta line\n",
    )
    .expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_viewport_height(3);
    app.open_preview_focus();
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("focus preview");

    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("open preview search");
    assert!(
        app.preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
    );

    for ch in ['b', 'e', 't', 'a'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type search");
    }
    assert_eq!(app.panes.get(&1).expect("pane").preview_match_count(), 2);
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);
    assert_eq!(app.status, "preview search: beta (2)");
}

#[test]
/// 驗證在 VCS Diff 模式下按下 `/` 進行搜尋時：
/// 1. 不會跳回全文預覽，依然保持在 preview diff 模式。
/// 2. 搜尋輸入能直接在 diff 內容中查找與計算 match count。
/// 3. 預覽畫面保持顯示 diff lines。
fn app_preview_diff_mode_search_stays_in_diff_and_searches_diff_lines() {
    use ratatui::text::Line;
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("code.rs"), "fn main() {}\n").expect("write");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    // 開啟預覽並進入 diff 模式 (Ctrl+d)
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("toggle diff mode");

    let pane = app.panes.get_mut(&1).expect("pane");
    assert!(pane.preview_diff_mode);
    assert!(pane.preview_open);

    // 模擬已載入的 VCS Diff 快取
    let sample_diff_lines = vec![
        Line::from("--- a/code.rs"),
        Line::from("+++ b/code.rs"),
        Line::from("@@ -1,1 +1,3 @@"),
        Line::from("+    let diff_feature = true;"),
    ];
    let entry = pane.selected_entry().expect("entry").clone();
    if let Ok(mut guard) = pane.preview_diff_cache.lock() {
        *guard = Some((entry.path.clone(), Some(entry.modified), sample_diff_lines));
    }

    // 聚焦到 preview 區塊 (l)
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("focus preview");

    // 當前已在 diff 預覽中，按下 `/` 開啟搜尋
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("open preview search");
    assert!(
        app.preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
    );

    // 輸入搜尋字串 "diff_feature"
    for ch in "diff_feature".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type search");
    }

    // 驗證仍處於 diff 模式，且成功搜尋到 1 個 match
    let pane = app.panes.get(&1).expect("pane");
    assert!(pane.preview_diff_mode, "搜尋期間必須依然為 diff 模式");
    assert_eq!(pane.preview_match_count(), 1);

    // 預覽行依然為 diff 內容
    let rendered = pane.preview_lines(10, Theme::default_theme());
    assert!(
        rendered
            .iter()
            .any(|line| line.to_string().contains("diff_feature"))
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.to_string().contains("fn main()"))
    );
}

#[test]
/// 驗證 preview search 支援 `n/N` 跳轉命中結果，Esc 先清搜尋再離開 preview mode。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_search_navigation_and_escape_flow() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("readme.md"),
        "zero\nmatch one\nmiddle\nmatch two\nend\n",
    )
    .expect("readme");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_viewport_height(3);
    app.open_preview_focus();
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_focused(true);
    app.open_preview_search_input();
    for ch in ['m', 'a', 't', 'c', 'h'] {
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock search");

    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("next match");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("previous match by p");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("wrap to last match");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE))
        .expect("previous match by N");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("clear search");
    assert!(app.panes.get(&1).expect("pane").is_preview_active());
    assert!(!app.panes.get(&1).expect("pane").has_preview_search());
    assert_eq!(app.status, "preview search cleared");

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("leave preview focus");
    assert!(app.panes.get(&1).expect("pane").is_preview_open());
    assert!(!app.panes.get(&1).expect("pane").is_preview_focused());
    assert_eq!(app.status, "file list (preview open)");

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("leave preview");
    assert!(!app.panes.get(&1).expect("pane").is_preview_open());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 preview search 在同一行有多個命中時，`n/p` 仍會逐一輪詢每個命中位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_search_cycles_each_match_occurrence() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "tt line\nonly t here\n").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_viewport_height(4);
    app.open_preview_focus();
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_focused(true);
    app.open_preview_search_input();
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock search");

    assert_eq!(app.panes.get(&1).expect("pane").preview_match_count(), 3);
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_current_match,
        Some(0)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("next occurrence on same line");
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_current_match,
        Some(1)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("move to next line occurrence");
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_current_match,
        Some(2)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("wrap to first occurrence");
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_current_match,
        Some(0)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("wrap back to last occurrence");
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_current_match,
        Some(2)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("move back to same-line occurrence");
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_current_match,
        Some(1)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("move back to first occurrence on same line");
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_current_match,
        Some(0)
    );
}

#[test]
/// 驗證 preview search 重新打開時，不會殘留上一次輸入的查詢字串。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_search_reopen_starts_with_empty_buffer() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "alpha\nbeta\ngamma\n").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_preview_focus();
    app.open_preview_search_input();
    for ch in ['b', 'e', 't', 'a'] {
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock search");
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_search_query(),
        Some("beta")
    );

    app.open_preview_search_input();

    assert!(
        app.preview_search
            .as_ref()
            .is_some_and(|search| search.buffer.is_empty() && search.editing)
    );
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_search_query(),
        None
    );
    assert_eq!(app.status, "preview search: all");
}

#[test]
/// 驗證 preview search 輸入框中的 `Tab` 不會誤套用 command 補齊。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_search_tab_does_not_apply_command_autocomplete() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "recent\nrename\n").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_preview_focus();
    app.open_preview_search_input();
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab in preview search");

    assert!(
        app.preview_search
            .as_ref()
            .is_some_and(|search| search.buffer == "re" && search.editing)
    );
    assert_eq!(
        app.panes.get(&1).expect("pane").preview_search_query(),
        Some("re")
    );
    assert_eq!(app.status, "preview search: re (2)");
}
