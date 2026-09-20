use super::*;

#[test]
/// 驗證按下 `Tab` 會直接進入 preview mode。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tab_opens_preview_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("open preview with tab");

    assert!(app.panes.get(&1).expect("pane").is_preview_open());
    assert!(app.panes.get(&1).expect("pane").is_preview_focused());
    assert!(app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(
        app.status,
        "preview focused (press 'h' to return to list, 'Tab' to close)"
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("return to list with h");
    assert!(!app.panes.get(&1).expect("pane").is_preview_focused());
    assert_eq!(app.status, "file list (preview open)");

    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("focus preview with l");
    assert!(app.panes.get(&1).expect("pane").is_preview_focused());
    assert!(app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(app.status, "preview focused (press 'h' to return to list)");
}

#[test]
/// 驗證 preview mode 的 `J / K` 會用固定大步長快速捲動內容。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_shift_j_and_k_scroll_by_large_step() {
    let dir = tempdir().expect("tempdir");
    let content = (0..20)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.path().join("notes.txt"), content).expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_viewport_height(4);
    app.open_preview_focus();
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("focus preview");

    app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
        .expect("preview fast down");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 5);

    app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
        .expect("preview fast up");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
}

#[test]
/// 驗證進入 preview mode 後，`j/k` 會改成捲動 preview，Esc 會離開該模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_mode_scrolls_and_exits_cleanly() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("notes.txt"),
        "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
    )
    .expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_viewport_height(4);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("open preview");
    assert!(app.panes.get(&1).expect("pane").is_preview_open());
    assert!(app.panes.get(&1).expect("pane").is_preview_focused());
    assert!(app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(
        app.status,
        "preview focused (press 'h' to return to list, 'Tab' to close)"
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("cursor down");
    assert_eq!(app.panes.get(&1).expect("pane").preview_cursor, 1);
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

    // 游標在可視範圍 (4 行) 內自由移動，超出邊界時才捲動視窗
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("cursor down");
    }
    assert_eq!(app.panes.get(&1).expect("pane").preview_cursor, 4);
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

    // 向上移動回頂部，視窗捲回 0
    for _ in 0..4 {
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("cursor up");
    }
    assert_eq!(app.panes.get(&1).expect("pane").preview_cursor, 0);
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("leave preview focus");
    assert!(app.panes.get(&1).expect("pane").is_preview_open());
    assert!(!app.panes.get(&1).expect("pane").is_preview_focused());
    assert_eq!(app.status, "file list (preview open)");

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("close preview");
    assert!(!app.panes.get(&1).expect("pane").is_preview_open());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證按下 `Tab` 會切換 preview mode，再按一次同樣的鍵會回到一般列表。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_mode_toggles_with_tab() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "preview").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("open preview");
    assert!(app.panes.get(&1).expect("pane").is_preview_open());
    assert!(app.panes.get(&1).expect("pane").is_preview_focused());
    assert_eq!(
        app.status,
        "preview focused (press 'h' to return to list, 'Tab' to close)"
    );

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("toggle preview off");
    assert!(!app.panes.get(&1).expect("pane").is_preview_open());
    assert!(!app.panes.get(&1).expect("pane").is_preview_focused());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 preview mode 支援半頁捲動與 `gg/G` 的上下端跳轉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_preview_mode_supports_paging_and_boundary_jumps() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("readme.md"),
        "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n",
    )
    .expect("readme");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_viewport_height(4);
    app.open_preview_focus();
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("focus preview");

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        .expect("page down");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 4);

    app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE))
        .expect("bottom");
    let bottom_scroll = app.panes.get(&1).expect("pane").preview_scroll;
    assert!(bottom_scroll > 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("pending g");
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("top");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        .expect("page down again");
    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL))
        .expect("page up");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

    // 驗證 Ctrl+d 在 preview mode 下切換 diff 模式
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("toggle diff");
    assert!(app.panes.get(&1).expect("pane").preview_diff_mode);
}

#[test]
/// 驗證三個 panel 可以各自打開 preview，且關閉其中一個不會影響另外兩個。
/// 保護目的：防止 preview 開關退回 `App` 全域單一狀態，造成後開啟的 panel 關掉
/// 其他 panel 已顯示的 preview。
fn app_preview_mode_is_scoped_to_its_own_pane() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "beta").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    assert_eq!(app.focused_pane, 2);

    app.open_preview_focus();
    assert!(app.panes.get(&2).expect("panel 2").is_preview_open());

    app.focus_pane_by_id(1);
    app.open_preview_focus();
    assert!(app.panes.get(&1).expect("panel 1").is_preview_open());
    assert!(app.panes.get(&2).expect("panel 2").is_preview_open());

    app.split_current(SplitDirection::Horizontal)
        .expect("split third panel");
    app.open_preview_focus();
    assert_eq!(app.focused_pane, 2);
    assert!(app.panes.get(&1).expect("panel 1").is_preview_open());
    assert!(app.panes.get(&2).expect("panel 2").is_preview_open());
    assert!(app.panes.get(&3).expect("panel 3").is_preview_open());

    app.focus_pane_by_id(3);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("close only panel 3 preview");
    assert!(app.panes.get(&1).expect("panel 1").is_preview_open());
    assert!(app.panes.get(&2).expect("panel 2").is_preview_open());
    assert!(!app.panes.get(&3).expect("panel 3").is_preview_open());
}

#[test]
/// 驗證檔案預覽模式支援 PageDown, PageUp, Home, End 導航鍵。
fn pagedown_pageup_home_end_in_preview_mode() {
    let dir = tempdir().expect("tempdir");
    let content = (1..=200).map(|i| format!("Line {i}\n")).collect::<String>();
    fs::write(dir.path().join("doc.txt"), content).expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    if let Some(pane) = app.panes.get_mut(&1) {
        pane.set_preview_viewport_size(80, 20);
    }

    app.open_preview_focus();
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_preview_focused(true);
    assert!(app.panes.get(&1).expect("pane").is_preview_active());

    // PageDown 向下翻整頁 (20 行)
    app.handle_preview_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))
        .expect("pagedown");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 20);
    assert_eq!(app.status, "preview: page down");

    // PageUp 向上翻整頁 (20 行)
    app.handle_preview_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE))
        .expect("pageup");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
    assert_eq!(app.status, "preview: page up");

    // End 跳到最底部 (200 - 20 = 180)
    app.handle_preview_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
        .expect("end");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 180);
    assert_eq!(app.status, "preview: bottom");

    // Home 跳到最上方 (0)
    app.handle_preview_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE))
        .expect("home");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
    assert_eq!(app.status, "preview: top");
}

#[test]
/// 完整的端到端 (E2E) 使用者旅程測試：
/// 模擬清單模式背景預熱 -> 按 Tab 進入預覽 -> 立即用 j/Down/PageDown 捲動 -> 用 [/] 切換檔案 -> 按 Tab 退出。
/// 此測試防範「加入新功能或背景非同步導致游標與捲動卡死」的迴歸問題。
fn test_user_journey_prefetch_tab_preview_and_scroll_responsiveness() {
    let dir = tempdir().expect("tempdir");
    let file_guide = dir.path().join("01_guide.md");
    let file_short = dir.path().join("02_short.toml");
    let file_code = dir.path().join("03_code.rs");

    // 01_guide: 400 行
    let guide_content = (1..=400)
        .map(|i| format!("# Guide line {i}\n"))
        .collect::<String>();
    // 02_short: 15 行
    let short_content = (1..=15)
        .map(|i| format!("key_{i} = \"val\"\n"))
        .collect::<String>();
    // 03_code: 100 行
    let code_content = (1..=100)
        .map(|i| format!("fn line_{i}() {{}}\n"))
        .collect::<String>();

    fs::write(&file_guide, guide_content).expect("write guide");
    fs::write(&file_short, short_content).expect("write short");
    fs::write(&file_code, code_content).expect("write code");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    if let Some(pane) = app.panes.get_mut(&1) {
        pane.set_preview_viewport_size(80, 25);
        // 游標停在 01_guide.md
        pane.move_to_visible_index(0);
        assert_eq!(pane.selected_entry().unwrap().name, "01_guide.md");
        // 觸發清單模式下的背景投機性預熱
        pane.prefetch_current_and_adjacent_previews(true);
    }

    // 等待背景預熱執行緒跑完（包括當前項與相鄰短檔案）
    let file_guide_clone = file_guide.clone();
    let file_short_clone = file_short.clone();
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        let pane = app.panes.get(&1).unwrap();
        let guard = pane.preview_content_cache.lock().unwrap();
        if guard.contains(&file_guide_clone, None, 80)
            && guard.contains(&file_short_clone, None, 80)
        {
            break;
        }
    }

    // 步驟 1：使用者按下 Tab 鍵開啟右側預覽視窗，按 l 進入預覽操作
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab enters preview");
    assert!(
        app.panes.get(&1).unwrap().is_preview_open(),
        "預覽模式必須處於開啟狀態"
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("l focuses preview");
    assert!(
        app.panes.get(&1).unwrap().is_preview_focused(),
        "預覽焦點必須處於開啟狀態"
    );

    // 步驟 2：使用者立即按下 j 鍵向下移動游標
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("j scroll");
    assert_eq!(
        app.panes.get(&1).unwrap().preview_cursor,
        1,
        "按下 j 後游標必須立即向下移動至第 1 行"
    );
    assert_eq!(
        app.panes.get(&1).unwrap().preview_scroll,
        0,
        "在 viewport 內移動游標時不應產生非預期視窗位移"
    );

    // 步驟 3：使用者按下方向鍵 Down 繼續向下移動游標
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("Down scroll");
    assert_eq!(
        app.panes.get(&1).unwrap().preview_cursor,
        2,
        "按下 Down 後游標必須移動至第 2 行"
    );
    assert_eq!(app.panes.get(&1).unwrap().preview_scroll, 0);

    // 步驟 4：使用者按下 PageDown 翻頁
    app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))
        .expect("PageDown");
    assert_eq!(
        app.panes.get(&1).unwrap().preview_scroll,
        25,
        "PageDown 應向下翻 25 行 (0 + 25 = 25)"
    );
    assert_eq!(
        app.panes.get(&1).unwrap().preview_cursor,
        27,
        "PageDown 游標同步向下位移 25 行 (2 + 25 = 27)"
    );

    // 步驟 5：使用者按下 ] 鍵在預覽中切換到下一個檔案 (02_short.toml)
    app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE))
        .expect("] next file");
    assert_eq!(
        app.panes.get(&1).unwrap().selected_entry().unwrap().name,
        "02_short.toml"
    );
    assert_eq!(
        app.panes.get(&1).unwrap().preview_scroll,
        0,
        "切換檔案後 preview_scroll 必須重設為 0"
    );

    // 步驟 6：在 15 行的 short.toml 嘗試捲動（viewport 為 25，max_scroll 為 0）
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("j on short file");
    assert_eq!(
        app.panes.get(&1).unwrap().preview_cursor,
        1,
        "短檔案游標仍可自由移動"
    );
    assert_eq!(
        app.panes.get(&1).unwrap().preview_scroll,
        0,
        "短檔案不應溢出捲動"
    );

    // 步驟 7：使用者按下 [ 鍵切換回 01_guide.md
    app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE))
        .expect("[ prev file");
    assert_eq!(
        app.panes.get(&1).unwrap().selected_entry().unwrap().name,
        "01_guide.md"
    );
    assert_eq!(
        app.panes.get(&1).unwrap().preview_scroll,
        0,
        "切換回 guide 後 preview_scroll 必須為 0"
    );

    // 步驟 8：切換回 guide 後再次捲動，驗證捲動依舊立即可用
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("j after switch back");
    assert_eq!(
        app.panes.get(&1).unwrap().preview_cursor,
        1,
        "切換回大檔案後 j 依然能正常移動游標"
    );
    assert_eq!(
        app.panes.get(&1).unwrap().preview_scroll,
        0,
        "未達 viewport 邊界不捲動"
    );

    // 步驟 9：使用者按下 Tab 鍵退出預覽模式回到檔案列表
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab exits preview");
    assert!(
        !app.panes.get(&1).unwrap().is_preview_open(),
        "Tab 必須關閉預覽回到 normal mode"
    );
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證在預覽模式下按下 `]` 與 `[` 可以上下切換檔案，且自動重設捲動位置為 0。
fn preview_mode_bracket_keys_navigate_files() {
    let dir = tempdir().expect("tempdir");
    let file_a = dir.path().join("a.txt");
    let file_b = dir.path().join("b.txt");
    let file_c = dir.path().join("c.txt");
    fs::write(
        &file_a,
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\n",
    )
    .expect("write a");
    fs::write(&file_b, "content b\n").expect("write b");
    fs::write(&file_c, "content c\n").expect("write c");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    if let Some(pane) = app.panes.get_mut(&app.focused_pane) {
        pane.set_preview_viewport_height(4);
    }
    app.open_preview_focus();
    app.panes
        .get_mut(&app.focused_pane)
        .unwrap()
        .set_preview_focused(true);
    assert!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .is_preview_active()
    );
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "a.txt"
    );

    // 捲動當前檔案內容
    app.current_pane_mut().unwrap().scroll_preview_down(2);
    assert_eq!(app.panes.get(&app.focused_pane).unwrap().preview_scroll, 2);

    // 按下 `]` 切換到下一個檔案 b.txt
    app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE))
        .expect("handle ]");
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "b.txt"
    );
    assert_eq!(app.panes.get(&app.focused_pane).unwrap().preview_scroll, 0);
    assert!(app.status.contains("b.txt"));

    // 再按 `]` 切換到 c.txt
    app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE))
        .expect("handle ]");
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "c.txt"
    );

    // 在末尾再按 `]`，維持在 c.txt
    app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE))
        .expect("handle ]");
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "c.txt"
    );

    // 按下 `[` 切換回 b.txt
    app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE))
        .expect("handle [");
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "b.txt"
    );

    // 按下 `[` 切換回 a.txt
    app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE))
        .expect("handle [");
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "a.txt"
    );

    // 在頂部再按 `[`，維持在 a.txt
    app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE))
        .expect("handle [");
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "a.txt"
    );

    // 測試數字前綴：2] 跳過 2 個檔案直接到 c.txt
    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .expect("handle 2");
    app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE))
        .expect("handle ]");
    assert_eq!(
        app.panes
            .get(&app.focused_pane)
            .unwrap()
            .selected_entry()
            .unwrap()
            .name,
        "c.txt"
    );
}

#[test]
fn test_preview_scroll_preserved_across_background_directory_load_and_watcher_reload() {
    let dir = tempdir().expect("tempdir");
    let test_file = dir.path().join("large.rs");
    let mut file_content = String::new();
    for i in 1..=100 {
        file_content.push_str(&format!("fn line_{i}() {{ println!(\"{i}\"); }}\n"));
    }
    fs::write(&test_file, file_content).expect("write large.rs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let pane = app.panes.get_mut(&1).unwrap();
    pane.set_preview_viewport_size(80, 10);

    // 開啟預覽模式 (Tab) 並進入預覽操作 (l)
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab preview");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("focus preview");
    assert!(app.panes[&1].is_preview_active());

    // 捲動 15 行
    app.panes.get_mut(&1).unwrap().scroll_preview_down(15);
    assert_eq!(app.panes[&1].preview_scroll, 15);

    // 模擬背景目錄載入 Batch 到達
    let entries = app.panes[&1].entries.clone();
    app.panes
        .get_mut(&1)
        .unwrap()
        .extend_entries(entries.clone());
    assert_eq!(
        app.panes[&1].preview_scroll, 15,
        "Batch 到達時 preview_scroll 不得重設為 0"
    );

    // 模擬背景目錄載入 Complete 到達
    app.panes
        .get_mut(&1)
        .unwrap()
        .replace_entries_presorted(entries, None);
    assert_eq!(
        app.panes[&1].preview_scroll, 15,
        "Complete 到達時 preview_scroll 不得重設為 0"
    );

    // 模擬 watcher 觸發目錄 reload
    app.panes.get_mut(&1).unwrap().reload().expect("reload");
    assert_eq!(
        app.panes[&1].preview_scroll, 15,
        "Watcher reload 時 preview_scroll 不得重設為 0"
    );

    // 模擬背景語法高亮完成 update_complete
    let full_lines = vec![ratatui::text::Line::from("test"); 100];
    if let Ok(mut guard) = app.panes[&1].preview_content_cache.lock() {
        guard.update_complete(&test_file, None, full_lines);
    }
    assert_eq!(
        app.panes[&1].preview_scroll, 15,
        "語法高亮完成時 preview_scroll 仍必須維持在 15"
    );
}

#[test]
/// 驗證左右分割預覽工作流程 (方案 b)：
/// 1. Tab 開啟 preview (預覽開、焦點在左側檔案列表)
/// 2. 在檔案列表中按 j/k 移動游標，右側 preview 即時更新
/// 3. 在檔案上按 l 進入 preview 焦點 (左側 border muted，右側 border focused)
/// 4. 在 preview 焦點中按 j/k 捲動 preview 內容
/// 5. 在 preview 焦點中按 h 回到檔案列表，preview 保持開啟
/// 6. 在資料夾上按 l 進入子資料夾，preview 保持開啟且即時預覽子目錄新項目
/// 7. 按 Tab 徹底關閉 preview
fn test_side_by_side_preview_workflow_with_tab_l_and_h() {
    let dir = tempdir().expect("tempdir");
    let sub = dir.path().join("subdir");
    fs::create_dir(&sub).expect("create subdir");
    fs::write(sub.join("subfile.txt"), "sub content").expect("subfile");

    let file_a = dir.path().join("a_file.txt");
    let content_a = (1..=50)
        .map(|i| format!("Line A {i}\n"))
        .collect::<String>();
    fs::write(&file_a, content_a).expect("file a");

    let _file_b = dir.path().join("b_file.txt");
    fs::write(dir.path().join("b_file.txt"), "File B content\n").expect("file b");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let pane = app.panes.get_mut(&1).unwrap();
    pane.set_preview_viewport_size(80, 10);

    // 初始狀態：preview 未開啟
    assert!(!app.panes[&1].is_preview_open());
    assert!(!app.panes[&1].is_preview_focused());

    // 1. 按 Tab 開啟預覽，焦點直接進入右側 preview
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab opens preview");
    assert!(app.panes[&1].is_preview_open(), "preview 應開啟");
    assert!(
        app.panes[&1].is_preview_focused(),
        "焦點應直接進入右側 preview"
    );
    assert_eq!(
        app.status,
        "preview focused (press 'h' to return to list, 'Tab' to close)"
    );

    // 2. 按 h 退出 preview 焦點回到檔案列表
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("h returns to list");
    assert!(!app.panes[&1].is_preview_focused());
    assert_eq!(app.status, "file list (preview open)");

    // 在檔案列表中移動 j，右側預覽即時更新
    let initial_selected = app.panes[&1].selected;
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("j moves in list");
    assert_eq!(app.panes[&1].selected, initial_selected + 1);
    assert!(app.panes[&1].is_preview_open());
    assert!(!app.panes[&1].is_preview_focused());

    // 移動游標定位到 a_file.txt
    let target_idx = app.panes[&1]
        .entries
        .iter()
        .position(|e| e.name == "a_file.txt")
        .expect("find a_file.txt");
    app.panes
        .get_mut(&1)
        .unwrap()
        .move_to_visible_index(target_idx);
    assert_eq!(app.panes[&1].selected_entry().unwrap().name, "a_file.txt");

    // 3. 在檔案上按 l 進入 preview 焦點
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("l enters preview focus");
    assert!(app.panes[&1].is_preview_open(), "preview 仍應開啟");
    assert!(app.panes[&1].is_preview_focused(), "preview 應取得焦點");
    assert!(
        app.panes[&1].is_preview_active(),
        "preview 應處於 active 狀態"
    );
    assert_eq!(app.status, "preview focused (press 'h' to return to list)");

    // 4. 在 preview 焦點中按 j 移動預覽游標
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("j moves preview cursor");
    assert_eq!(app.panes[&1].preview_cursor, 1);
    assert_eq!(app.panes[&1].preview_scroll, 0);
    assert_eq!(
        app.panes[&1].selected_entry().unwrap().name,
        "a_file.txt",
        "檔案選取不可變"
    );

    // 5. 在 preview 焦點中按 h 回到檔案列表，preview 保持開啟
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("h returns to list");
    assert!(app.panes[&1].is_preview_open(), "preview 仍應保持開啟");
    assert!(!app.panes[&1].is_preview_focused(), "焦點應回到檔案列表");
    assert_eq!(app.status, "file list (preview open)");

    // 6. 移動游標至 subdir 資料夾，按 l 應進入子資料夾
    let sub_idx = app.panes[&1]
        .entries
        .iter()
        .position(|e| e.name == "subdir")
        .expect("find subdir");
    app.panes
        .get_mut(&1)
        .unwrap()
        .move_to_visible_index(sub_idx);
    assert!(app.panes[&1].selected_entry().unwrap().is_dir);

    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("l enters directory");
    assert!(app.panes[&1].cwd.ends_with("subdir"), "應進入 subdir");
    assert!(
        app.panes[&1].is_preview_open(),
        "進入子目錄後 preview 仍應保持開啟"
    );

    // 7. 按 Tab 徹底關閉預覽
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab closes preview");
    assert!(!app.panes[&1].is_preview_open(), "preview 應關閉");
    assert!(!app.panes[&1].is_preview_focused());
    assert_eq!(app.status, "normal mode");
}
