use super::*;

#[test]
/// 驗證 global search 在輸入階段不會立即掃描，按下 Enter 後才真正執行搜尋。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_global_search_filters_nested_entries() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("Readme.md"), "doc").expect("readme");
    fs::create_dir(dir.path().join("src")).expect("src");
    fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").expect("main");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("open search");

    for ch in ['r', 'e', 'a', 'd'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }

    let search = app.global_search.as_ref().expect("search");
    assert!(search.editing);
    assert_eq!(search.results.len(), 0);
    assert!(!search.searched);
    assert_eq!(
        app.status,
        "global search (insert): read (press Enter to search)"
    );

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("run search");
    wait_for_global_search(&mut app);
    let search = app.global_search.as_ref().expect("search after run");
    assert!(!search.editing);
    assert!(search.searched);
    assert_eq!(search.results.len(), 1);
    assert_eq!(search.results[0].relative_path, "docs/Readme.md");
    assert_eq!(app.status, "global search (normal): read (1)");
}

#[test]
/// 驗證 global search 提交查詢後，再按一次 Enter 會跳到選中的搜尋結果。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_global_search_enter_reveals_selected_file() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("open search");
    for ch in ['g', 'u', 'i', 'd'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock search");
    wait_for_global_search(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open result");

    assert!(app.global_search.is_none());
    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.cwd, dir.path().join("docs"));
    assert_eq!(
        pane.selected_entry().map(|entry| entry.display_name()),
        Some(String::from("guide.md"))
    );
    assert_eq!(app.status, "search opened: docs/guide.md");
}

#[test]
/// 驗證在 global search 執行中按下 Esc，會關閉介面並要求背景搜尋停止。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_global_search_escape_cancels_background_work() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("open search");
    for ch in ['g', 'u', 'i', 'd'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("start search");
    let cancelled = app
        .global_search_cancelled
        .as_ref()
        .expect("cancel flag")
        .clone();

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel search");

    assert!(app.global_search.is_none());
    assert!(app.global_search_rx.is_none());
    assert!(app.global_search_cancelled.is_none());
    assert!(cancelled.load(Ordering::Relaxed));
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證在 global search 結果列表中按下 h，會安全返回一般列表。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_global_search_h_leaves_results_list() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("open search");
    for ch in ['g', 'u', 'i', 'd'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("start search");
    wait_for_global_search(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("leave search");

    assert!(app.global_search.is_none());
    assert!(app.global_search_rx.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 `Shift+S` 會打開內容搜尋面板，而不是一般路徑搜尋。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_s_opens_content_search() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
        .expect("open content search");

    let search = app.global_search.as_ref().expect("search");
    assert_eq!(search.mode, SearchMode::Content);
    assert!(search.editing);
    assert_eq!(app.status, "content search (insert): type query and Enter");
}

#[test]
/// 驗證 `s` 與 `S` 的結果仍在串流載入時，只要列表已有內容就能立即用游標鍵與 Vim 鍵移動。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_search_lists_move_immediately_while_loading() {
    let dir = tempdir().expect("tempdir");

    for mode in [SearchMode::Path, SearchMode::Content] {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.global_search = Some(GlobalSearchState {
            pane_id: 1,
            root_dir: dir.path().to_path_buf(),
            mode,
            buffer: String::from("target"),
            editing: false,
            loading: true,
            searched: true,
            selected: 0,
            results: ["alpha.txt", "beta.txt", "gamma.txt"]
                .into_iter()
                .map(|name| GlobalSearchEntry {
                    path: dir.path().join(name),
                    relative_path: name.to_string(),
                    is_dir: false,
                    match_line_number: None,
                    match_column: None,
                    match_preview: None,
                })
                .collect(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        });

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .expect("move down while loading");
        assert_eq!(app.global_search.as_ref().expect("search").selected, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("vim move down while loading");
        assert_eq!(app.global_search.as_ref().expect("search").selected, 2);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .expect("move up while loading");
        assert_eq!(app.global_search.as_ref().expect("search").selected, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("vim move up while loading");
        assert_eq!(app.global_search.as_ref().expect("search").selected, 0);
    }
}

#[test]
/// 驗證 `s` 與 `S` 的結果面板都能按 `f` 開啟模糊 filter，並以不連續字元縮小結果。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_search_result_panels_support_fuzzy_filtering() {
    let dir = tempdir().expect("tempdir");

    for mode in [SearchMode::Path, SearchMode::Content] {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.global_search = Some(GlobalSearchState {
            pane_id: 1,
            root_dir: dir.path().to_path_buf(),
            mode,
            buffer: String::from("source"),
            editing: false,
            loading: false,
            searched: true,
            selected: 0,
            results: ["src/file_manager/app.rs", "docs/sample.txt", "README.md"]
                .into_iter()
                .map(|name| GlobalSearchEntry {
                    path: dir.path().join(name),
                    relative_path: name.to_string(),
                    is_dir: false,
                    match_line_number: None,
                    match_column: None,
                    match_preview: None,
                })
                .collect(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        });

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open result filter");
        for ch in ['f', 'm', 'a'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type fuzzy result filter");
        }

        let search = app.global_search.as_ref().expect("search");
        assert!(search.filter.editing);
        assert_eq!(search.filter.buffer, "fma");
        let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].relative_path, "src/file_manager/app.rs");
    }
}

#[test]
/// 驗證從模糊過濾後的搜尋列表按 Enter，會開啟目前可見結果而非原始索引項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_search_filter_opens_filtered_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "beta").expect("beta");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        mode: SearchMode::Path,
        buffer: String::from("txt"),
        editing: false,
        loading: false,
        searched: true,
        selected: 0,
        results: ["alpha.txt", "beta.txt"]
            .into_iter()
            .map(|name| GlobalSearchEntry {
                path: dir.path().join(name),
                relative_path: name.to_string(),
                is_dir: false,
                match_line_number: None,
                match_column: None,
                match_preview: None,
            })
            .collect(),
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
        task_id: None,
    });

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open result filter");
    for ch in ['b', 't'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type result filter");
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock result filter");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open filtered result");

    assert!(app.global_search.is_none());
    assert_eq!(
        app.panes
            .get(&1)
            .and_then(|pane| pane.selected_entry())
            .map(|entry| entry.display_name()),
        Some(String::from("beta.txt"))
    );
}

#[test]
/// 驗證 `s` 與 `S` 收到新批次時只會追加到下方，不會重排既有列表或移動游標。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_search_stream_appends_without_reordering_existing_rows() {
    let dir = tempdir().expect("tempdir");
    for mode in [SearchMode::Path, SearchMode::Content] {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.global_search = Some(GlobalSearchState {
            pane_id: 1,
            root_dir: dir.path().to_path_buf(),
            mode,
            buffer: String::from("txt"),
            editing: false,
            loading: true,
            searched: true,
            selected: 0,
            results: vec![GlobalSearchEntry {
                path: dir.path().join("beta.txt"),
                relative_path: String::from("beta.txt"),
                is_dir: false,
                match_line_number: None,
                match_column: None,
                match_preview: None,
            }],
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        });
        let (sender, receiver) = std::sync::mpsc::channel();
        app.global_search_rx = Some(receiver);
        sender
            .send(GlobalSearchEvent::Chunk {
                pane_id: 1,
                query: String::from("txt"),
                entries: vec![GlobalSearchEntry {
                    path: dir.path().join("alpha.txt"),
                    relative_path: String::from("alpha.txt"),
                    is_dir: false,
                    match_line_number: None,
                    match_column: None,
                    match_preview: None,
                }],
            })
            .expect("send result chunk");

        app.poll_background_tasks();

        let search = app.global_search.as_ref().expect("search");
        assert_eq!(search.selected, 0);
        assert_eq!(search.results[0].relative_path, "beta.txt");
        assert_eq!(search.results[1].relative_path, "alpha.txt");
    }
}

#[test]
/// 驗證內容搜尋會依照檔案內容比對結果，並只回傳真正命中的檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_content_search_matches_file_contents() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("guide.md"), "release note").expect("guide");
    fs::write(dir.path().join("todo.txt"), "buy milk").expect("todo");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
        .expect("open content search");
    for ch in ['r', 'e', 'l', 'e', 'a', 's', 'e'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("run content search");
    wait_for_global_search(&mut app);

    let search = app.global_search.as_ref().expect("search");
    assert_eq!(search.mode, SearchMode::Content);
    assert!(!search.editing);
    assert!(search.searched);
    assert_eq!(search.results.len(), 1);
    assert_eq!(search.results[0].relative_path, "docs/guide.md");
    assert_eq!(app.status, "content search (normal): release (1)");
    let task = app
        .task_log
        .iter()
        .find(|task| task.kind == "search")
        .expect("search task");
    assert_eq!(task.state, TaskState::Done);
}

#[test]
/// 驗證內容搜尋按下 Enter 只會跳到檔案，不會強制切進 preview。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_content_search_enter_reveals_selected_file() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(
        dir.path().join("docs").join("notes.txt"),
        "zero\nmatch one\nmiddle\nmatch two\nend\n",
    )
    .expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
        .expect("open content search");
    for ch in ['m', 'a', 't', 'c', 'h'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("run content search");
    wait_for_global_search(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open search result");

    assert!(app.global_search.is_none());
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(
        pane.selected_entry().map(|entry| entry.display_name()),
        Some(String::from("notes.txt"))
    );
    assert_eq!(pane.cwd, dir.path().join("docs"));
    assert_eq!(app.status, "search opened: docs/notes.txt");
}

#[test]
/// 驗證內容搜尋按下 Right 也只會跳到檔案，與 Enter / l 行為一致。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_content_search_right_reveals_selected_file() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("notes.txt"), "alpha\nbeta\n").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_content_search().expect("open content search");
    for ch in ['b', 'e', 't', 'a'] {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type query");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("run content search");
    wait_for_global_search(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .expect("open by right");

    assert!(app.global_search.is_none());
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(
        pane.selected_entry().map(|entry| entry.display_name()),
        Some(String::from("notes.txt"))
    );
    assert_eq!(pane.cwd, dir.path().join("docs"));
    assert_eq!(app.status, "search opened: docs/notes.txt");
}

#[test]
/// 驗證在搜尋尚未完成前直接開啟結果，背景 search task 會被正確標記為取消。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_opening_search_result_cancels_running_search_task() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("target.txt"), "target\n").expect("target");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "search",
        String::from("content search: target"),
        format!("root: {}", dir.path().display()),
        vec![dir.path().display().to_string()],
        None,
    );
    app.active_global_search_task_id = Some(task_id);

    let search = GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        mode: SearchMode::Content,
        buffer: String::from("target"),
        editing: false,
        loading: true,
        searched: true,
        selected: 0,
        results: vec![GlobalSearchEntry {
            path: dir.path().join("target.txt"),
            relative_path: String::from("target.txt"),
            is_dir: false,
            match_line_number: Some(1),
            match_column: Some(1),
            match_preview: Some(String::from("target")),
        }],
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
        task_id: Some(task_id),
    };

    app.open_global_search_result(search)
        .expect("open search result");

    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.state, TaskState::Cancelled);
    assert_eq!(task.detail, "stopped after opening a result");
    assert!(app.global_search.is_none());
    assert!(app.global_search_rx.is_none());
    assert!(app.active_global_search_task_id.is_none());
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(
        app.panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .map(|entry| entry.display_name()),
        Some(String::from("target.txt"))
    );
    assert_eq!(app.status, "search opened: target.txt");
}

#[test]
/// 驗證列表模式按下 `/` 後會即時套用 find-next，並可在 Enter 後用 `n/N` 跳轉命中項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_list_find_supports_lock_and_navigation() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("alps.txt"), "b").expect("alps");
    fs::write(dir.path().join("beta.txt"), "c").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("open list find");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type a");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("type l");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.selected_entry().expect("selected").name, "alpha.txt");
    assert_eq!(pane.list_find_match_indices(), vec![0, 1]);
    assert_eq!(app.status, "find next: al (2)");

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock list find");
    assert!(app.list_find.is_none());
    assert_eq!(app.status, "find next locked: al (2)");

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("next match");
    assert_eq!(
        app.panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "alps.txt"
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT))
        .expect("previous match");
    assert_eq!(
        app.panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "alpha.txt"
    );
}

#[test]
/// 驗證列表模式的 find-next 在鎖定後按下 `Esc`，會清除目前 pane 的高亮結果。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_list_find_escape_clears_active_query() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("open list find");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock list find");
    assert!(app.panes.get(&1).expect("pane").list_find_query().is_some());

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("clear list find");
    assert!(app.panes.get(&1).expect("pane").list_find_query().is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證重新按下 `/` 打開 list find 時，不會沿用上一輪輸入的查詢文字。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_reopening_list_find_starts_with_empty_buffer() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("open list find");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock query");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("clear query");

    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("reopen list find");

    assert_eq!(
        app.list_find,
        Some(ListFindState {
            pane_id: 1,
            buffer: String::new(),
        })
    );
    assert!(app.panes.get(&1).expect("pane").list_find_query().is_none());
    assert_eq!(app.status, "find next: type query");
}

#[test]
/// 驗證 normal mode 支援像 Vim 一樣用數字前綴配合 `j` 一次移動多格。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_count_prefix_moves_list_cursor_by_multiple_rows() {
    let dir = tempdir().expect("tempdir");
    for index in 0..8 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE))
        .expect("count");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");

    assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
    assert!(app.pending_count.is_none());
}

#[test]
/// 驗證 count prefix 可以搭配 `gg` 與 `G` 跳到指定列表位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_count_prefix_supports_absolute_jumps() {
    let dir = tempdir().expect("tempdir");
    for index in 0..8 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE))
        .expect("count for gg");
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("first g");
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("second g");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 4);

    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .expect("count for G");
    app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE))
        .expect("shift g");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 1);
}

#[test]
/// 驗證 count prefix 可以搭配 list find 的 `n` 一次跳過多個命中結果。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_count_prefix_supports_list_find_navigation() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("alps.txt"), "b").expect("alps");
    fs::write(dir.path().join("algae.txt"), "c").expect("algae");
    fs::write(dir.path().join("beta.txt"), "d").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("open list find");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type a");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("type l");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock find");
    assert_eq!(
        app.panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "algae.txt"
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .expect("count");
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("jump matches");

    assert_eq!(
        app.panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "alps.txt"
    );
    assert!(app.pending_count.is_none());
}
