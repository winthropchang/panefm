use super::*;

#[test]
/// 驗證 filter 第一次 Esc 只進入 Normal 模式，輸入框與過濾結果都會保留。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_filter_first_escape_enters_normal_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type filter");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("enter normal mode");

    let pane = app.panes.get(&1).expect("pane");
    let visible_names: Vec<String> = pane
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();

    assert_eq!(
        visible_names,
        vec![String::from("alpha.txt"), String::from("beta.txt")]
    );
    assert!(app.filter.as_ref().is_some_and(|filter| filter.editing));
    assert_eq!(app.text_input_mode, RenameMode::Normal);
}

#[test]
/// 驗證一般 Filter 與 Preview Search 都會畫在其狀態所屬的左側 Panel 內。
/// 保護目的：避免繪圖重構時重新使用全畫面 `frame.area()`，導致多 Panel 的輸入框
/// 跑到整個 terminal 右上角，或覆蓋其他 Panel。
fn app_filter_inputs_render_inside_their_target_panel() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.focus_pane_by_id(1);
    assert_eq!(app.focused_pane, 1);

    app.open_filter_input(FilterMode::Normal);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| {
            let _ = app.render(frame);
        })
        .expect("render panel filter");
    let filter_x = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .position(|cell| cell.symbol() == "F")
        .map(|index| index % 80)
        .expect("Filter title");
    assert!(filter_x < 40, "Filter must stay in panel 1");

    app.filter = None;
    app.open_preview_focus();
    app.open_preview_search_input();
    terminal
        .draw(|frame| {
            let _ = app.render(frame);
        })
        .expect("render preview search");
    let preview_search_x = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .enumerate()
        .find_map(|(index, cell)| (cell.symbol() == "P" && index / 80 < 4).then_some(index % 80))
        .expect("Preview Search title");
    assert!(preview_search_x < 40, "Preview Search must stay in panel 1");
}

#[test]
/// 驗證第二次 Esc 收起 filter 輸入框，第三次 Esc 才清掉已鎖定的 filter。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_filter_escape_flow_locks_then_clears_filter() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("type filter");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("enter normal mode");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close input");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("clear filter");

    let pane = app.panes.get(&1).expect("pane");
    let visible_names: Vec<String> = pane
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();

    assert_eq!(
        visible_names,
        vec![String::from("alpha.txt"), String::from("beta.txt")]
    );
    assert!(app.filter.is_none());
    assert!(!pane.has_active_filter());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證連續重新開啟 filter 時，不會殘留上一輪輸入的關鍵字。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_reopening_filter_starts_with_empty_buffer() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("type filter");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close input");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("clear filter");

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("reopen filter");

    assert_eq!(
        app.filter,
        Some(FilterState {
            pane_id: 1,
            buffer: String::new(),
            editing: true,
            mode: FilterMode::Normal,
        })
    );
    assert_eq!(app.status, "filter [normal]: all (Tab to switch)");
}

#[test]
/// 驗證 filter 輸入框中的 `Tab` 不會被當成 command 補齊，而是切換為模糊過濾模式（Fuzzy）。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_filter_input_tab_does_not_apply_command_autocomplete() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("recent.txt"), "a").expect("recent");
    fs::write(dir.path().join("rename.txt"), "b").expect("rename");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab in filter");

    assert_eq!(
        app.filter,
        Some(FilterState {
            pane_id: 1,
            buffer: String::from("re"),
            editing: true,
            mode: FilterMode::Fuzzy,
        })
    );
    assert_eq!(app.status, "filter [fuzzy]: re");
}

#[test]
/// 驗證一般檔案列表的 `f` 預設以逐詞包含過濾，不接受不連續字元命中。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，意外讓大型目錄回到昂貴的模糊排序。
fn app_file_list_filter_uses_all_terms_as_contiguous_substrings() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("file-manager-app.rs"), "app").expect("app");
    fs::write(dir.path().join("sample.txt"), "sample").expect("sample");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    for ch in ['f', 'i', 'l', 'e', ' ', 'a', 'p', 'p'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type filter");
    }

    let visible: Vec<String> = app
        .panes
        .get(&1)
        .expect("pane")
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();
    assert_eq!(visible, vec![String::from("file-manager-app.rs")]);
}

#[test]
/// 驗證包含 `~` 符號的特殊檔名在一般過濾模式下能被精確連續字串比對，不會被誤當成模式切換語法。
fn app_file_list_filter_matches_literal_tilde_in_filenames() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("~backup.rs"), "app").expect("app");
    fs::write(dir.path().join("sample.txt"), "sample").expect("sample");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    for ch in ['~', 'b', 'a', 'c', 'k'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type filter with tilde");
    }
    assert_eq!(app.filter.as_ref().expect("filter").buffer, "~back");

    let visible: Vec<String> = app
        .panes
        .get(&1)
        .expect("pane")
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();
    assert_eq!(visible, vec![String::from("~backup.rs")]);
}

#[test]
/// 驗證按下 `f` 開啟一般過濾、按下 `F` 開啟模糊搜尋過濾，且在過濾輸入框內按 `Tab` 可無縫切換模式。
fn filter_f_and_shift_f_open_normal_and_fuzzy_modes_and_tab_toggles() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("libpanefm-6510b5220d8becac.rlib"), "bin").expect("write1");
    fs::write(dir.path().join("terminal_file_manager.d"), "bin").expect("write2");
    fs::write(dir.path().join("other_file.txt"), "bin").expect("write3");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. 按下 'f' 開啟一般過濾模式
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("press f");
    assert!(app.filter.is_some());
    assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Normal);
    assert!(app.status.contains("filter [normal]"));

    // 輸入 "pnefm"（非連續子字串，一般模式下不會命中）
    for c in ['p', 'n', 'e', 'f', 'm'] {
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .expect("type char");
    }
    assert_eq!(
        app.panes[&1].visible_indices.len(),
        0,
        "一般模式連續子字串比對不應命中"
    );

    // 2. 按下 Tab 切換為模糊過濾模式（Fuzzy）
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("press tab");
    assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Fuzzy);
    assert!(app.status.contains("filter [fuzzy]"));
    assert_eq!(
        app.panes[&1].visible_indices.len(),
        1,
        "模糊搜尋應命中 libpanefm-*.rlib"
    );
    assert_eq!(
        app.panes[&1].entries[app.panes[&1].visible_indices[0]].name,
        "libpanefm-6510b5220d8becac.rlib"
    );

    // 3. 再次按 Tab 切回一般模式
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("press tab back");
    assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Normal);
    assert_eq!(app.panes[&1].visible_indices.len(), 0);

    // 關閉當前 filter
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    app.panes.get_mut(&1).unwrap().clear_filter();
    app.filter = None;

    // 4. 按下 'F' (Shift+F) 直接開啟模糊搜尋過濾模式
    app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT))
        .expect("press Shift+F");
    assert!(app.filter.is_some());
    assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Fuzzy);
    assert!(app.status.contains("filter [fuzzy]"));

    // 輸入 "tfm" 模糊搜尋
    for c in ['t', 'f', 'm'] {
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .expect("type char");
    }
    assert!(
        app.panes[&1]
            .visible_indices
            .iter()
            .any(|idx| { app.panes[&1].entries[*idx].name == "terminal_file_manager.d" }),
        "模糊搜尋 'tfm' 必須命中 terminal_file_manager.d"
    );
}
