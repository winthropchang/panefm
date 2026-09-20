use super::*;

#[test]
/// 驗證 `Ctrl+數字` 會正確轉成 pane 編號，供 pane 快速切換功能共用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn ctrl_digit_target_pane_id_maps_to_expected_panes() {
    assert_eq!(
        ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL)),
        Some(1)
    );
    assert_eq!(
        ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('9'), KeyModifiers::CONTROL)),
        Some(9)
    );
    assert_eq!(
        ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('0'), KeyModifiers::CONTROL)),
        Some(10)
    );
    assert_eq!(
        ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
        None
    );
}

#[test]
/// 驗證不帶修飾鍵的數字會正確轉成 pane 編號，供多 pane 直接切換焦點使用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn plain_digit_target_pane_id_maps_to_expected_panes() {
    assert_eq!(
        plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
        Some(1)
    );
    assert_eq!(
        plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE)),
        Some(9)
    );
    assert_eq!(
        plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE)),
        Some(10)
    );
    assert_eq!(
        plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL)),
        None
    );
}

#[test]
/// 驗證 `Ctrl+p` 會打開 command UI，並預先填入 `panel ` 方便直接輸入目標編號。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_p_opens_prefilled_panel_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .expect("open prefilled panel command");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "panel ");
    assert_eq!(app.status, "command mode");
}

#[test]
/// 驗證 `only_current_pane` 會只保留目前焦點窗格。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_only_keeps_focused_pane() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.only_current_pane();

    assert_eq!(app.ordered_pane_ids().len(), 1);
    assert_eq!(
        app.layout,
        LayoutNode::Leaf {
            pane_id: app.focused_pane
        }
    );
}

#[test]
/// 驗證啟動設定會正確套用到第一個 pane 的隱藏檔與排序偏好。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_new_applies_startup_pane_preferences() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".hidden"), "secret").expect("hidden");
    fs::write(dir.path().join("visible.txt"), "visible").expect("visible");

    let loaded = LoadedConfig {
        config: AppConfig {
            pane: crate::config::PaneConfig {
                show_hidden: true,
                default_sort: StartupSort::Size,
                default_sort_reverse: true,
                default_linemode: crate::config::StartupLinemode::Size,
            },
            ..AppConfig::default()
        },
        source: None,
        base_dir: PathBuf::new(),
    };

    let app = App::new(dir.path().to_path_buf(), loaded).expect("app");
    let pane = app.panes.get(&1).expect("pane");

    assert!(pane.show_hidden);
    assert_eq!(pane.sort_mode, SortMode::Size { reverse: true });
    assert_eq!(pane.line_mode, Some(LineMode::Size));
    assert_eq!(pane.visible_indices.len(), 2);
}

#[test]
/// 驗證新分割出來的 pane 會繼承原 pane 的顯示隱藏檔與排序方式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_split_inherits_pane_preferences() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".hidden"), "secret").expect("hidden");
    fs::write(dir.path().join("visible.txt"), "visible").expect("visible");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    {
        let pane = app.panes.get_mut(&1).expect("pane");
        pane.set_show_hidden(true);
        pane.set_sort_mode(SortMode::Modified { reverse: true });
        pane.set_line_mode(LineMode::Permissions);
    }

    app.split_current(SplitDirection::Vertical).expect("split");

    let pane = app.panes.get(&2).expect("new pane");
    assert!(pane.show_hidden);
    assert_eq!(pane.sort_mode, SortMode::Modified { reverse: true });
    assert_eq!(pane.line_mode, Some(LineMode::Permissions));
    assert_eq!(pane.visible_indices.len(), 2);
}

#[test]
/// 驗證按下 `w` 會打開 panel 操作面板，讓第二個按鍵可視化選擇。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_w_opens_window_picker() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open window picker");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::WindowPicker { pane_id: 1 })
    );
    assert_eq!(app.status, "panel: choose h/j/k/l/c/o/t/d from the panel");
}

#[test]
/// 驗證 `wt` 只使用目前 active panel 的 cwd 建立新終端請求。
///
/// 保護目的：多 panel 時不可誤用第一個 panel 或 PaneFM 啟動目錄，否則終端會開錯位置。
fn app_wt_opens_terminal_in_active_panel_directory() {
    let dir = tempdir().expect("tempdir");
    let first_dir = dir.path().join("first");
    let second_dir = dir.path().join("second");
    fs::create_dir(&first_dir).expect("first dir");
    fs::create_dir(&second_dir).expect("second dir");
    let mut app = App::new(first_dir, default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical)
        .expect("second panel");
    app.current_pane_mut().expect("active panel").cwd = second_dir.clone();
    app.current_pane_mut()
        .expect("active panel")
        .reload()
        .expect("reload second");

    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("window picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("terminal action");

    let queued = app.take_pending_launch().expect("terminal launch");
    let path_is_active = queued
        .launch
        .args
        .iter()
        .any(|arg| arg == &second_dir.display().to_string())
        || matches!(
            queued.launch.mode,
            LaunchMode::NewTerminal { ref current_dir } if current_dir == &second_dir
        );
    assert!(path_is_active);
    assert_eq!(
        app.status,
        format!("opening terminal: {}", second_dir.display())
    );
}

#[test]
/// 驗證 `wc` 會關閉目前 panel。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_window_picker_wc_closes_current_panel() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    assert_eq!(app.focused_pane, 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open window picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("close current panel");

    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.focused_pane, 1);
    assert_eq!(app.status, "closed panel 2");
}

#[test]
/// 驗證 `:panel <id>` 會把焦點直接切到指定 panel。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_panel_command_focuses_target_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.split_current(SplitDirection::Vertical).expect("split");
    assert_eq!(app.focused_pane, 2);
    app.focus_pane_by_id(1);
    assert_eq!(app.focused_pane, 1);

    app.execute_command("panel 2").expect("focus panel 2");

    assert_eq!(app.focused_pane, 2);
    assert_eq!(app.status, "focused panel 2");
}

#[test]
/// 驗證 `Ctrl+數字` 可直接切換焦點 panel。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_digit_focuses_target_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.split_current(SplitDirection::Vertical).expect("split");
    assert_eq!(app.focused_pane, 2);
    app.focus_pane_by_id(1);
    assert_eq!(app.focused_pane, 1);

    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::CONTROL))
        .expect("focus panel 2");

    assert_eq!(app.focused_pane, 2);
    assert_eq!(app.status, "focused panel 2");
}

#[test]
/// 驗證多 panel 時直接按數字鍵，也能快速把焦點切到指定 panel。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_plain_digit_focuses_target_panel_when_multiple_panels_exist() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.split_current(SplitDirection::Vertical).expect("split");
    assert_eq!(app.focused_pane, 2);
    app.focus_pane_by_id(1);
    assert_eq!(app.focused_pane, 1);

    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
        .expect("focus panel 2");

    assert_eq!(app.focused_pane, 2);
    assert_eq!(app.status, "focused panel 2");
}

#[test]
/// 驗證 `Ctrl+0` 會對應到 panel 10，讓雙位數前的最後一個快捷鍵也可直接使用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_zero_focuses_tenth_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    for _ in 0..9 {
        app.split_current(SplitDirection::Vertical).expect("split");
    }
    assert_eq!(app.focused_pane, 10);
    app.focus_pane_by_id(1);

    app.handle_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::CONTROL))
        .expect("focus panel 10");

    assert_eq!(app.focused_pane, 10);
    assert_eq!(app.status, "focused panel 10");
}

#[test]
/// 驗證 `w h/j/k/l` 會依方向在左下上右建立新的 pane。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_w_leader_splits_in_four_directions() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open w leader");
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("split left");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2]);
    assert_eq!(app.focused_pane, 1);
    assert_eq!(app.status, "split left");

    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open w leader");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("split down");
    assert_eq!(app.focused_pane, 2);
    assert_eq!(app.status, "split down");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open w leader");
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("split up");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2]);
    assert_eq!(app.focused_pane, 1);
    assert_eq!(app.status, "split up");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open w leader");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("split right");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2]);
    assert_eq!(app.focused_pane, 2);
    assert_eq!(app.status, "split right");
}

#[test]
/// 驗證 `Ctrl+s` / `Ctrl+v` 仍可作為分割 alias 使用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_split_shortcuts_create_expected_panes() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
        .expect("ctrl-s split");
    assert_eq!(app.panes.len(), 2);
    assert_eq!(app.focused_pane, 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL))
        .expect("ctrl-v split");
    assert_eq!(app.panes.len(), 3);
    assert_eq!(app.focused_pane, 3);
}

#[test]
/// 驗證建立項目後，所有開啟相同目錄的 panel 都會同步看到新項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_mutation_refreshes_sibling_panels_with_same_directory() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    let first_panel = app.ordered_pane_ids()[0];
    let second_panel = app.ordered_pane_ids()[1];

    app.confirm_create_entry(second_panel, "shared.txt")
        .expect("create shared file");

    for pane_id in [first_panel, second_panel] {
        let pane = app.panes.get(&pane_id).expect("pane");
        assert!(
            pane.entries
                .iter()
                .any(|entry| entry.display_name() == "shared.txt")
        );
    }
}

#[test]
/// 驗證 Finder／Explorer 在 PaneFM 外部新增或刪除檔案後，所有顯示該目錄的
/// panel 都會刷新，而且原本游標指向的檔案不會因排序位置改變而跳走。
/// 保護目的：避免 watcher 只更新 active panel，或 reload 只保留舊索引而選錯檔案。
fn external_directory_change_refreshes_every_matching_panel_and_keeps_selection() {
    let dir = tempdir().expect("tempdir");
    let original = dir.path().join("middle.txt");
    fs::write(&original, "original").expect("original file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    for pane in app.panes.values_mut() {
        pane.select_path(&original);
    }

    let external = dir.path().join("ahead.txt");
    fs::write(&external, "created outside PaneFM").expect("external create");
    app.reload_watched_directories(&std::collections::BTreeSet::from([dir
        .path()
        .to_path_buf()]))
        .expect("watcher refresh after create");

    for pane in app.panes.values() {
        assert!(pane.entries.iter().any(|entry| entry.path == external));
        assert_eq!(
            pane.selected_entry().map(|entry| &entry.path),
            Some(&original)
        );
    }

    fs::remove_file(&external).expect("external delete");
    app.reload_watched_directories(&std::collections::BTreeSet::from([dir
        .path()
        .to_path_buf()]))
        .expect("watcher refresh after delete");
    for pane in app.panes.values() {
        assert!(!pane.entries.iter().any(|entry| entry.path == external));
        assert_eq!(
            pane.selected_entry().map(|entry| &entry.path),
            Some(&original)
        );
    }
}

#[test]
fn window_picker_w_and_h_open_prefilled_commands() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::WindowPicker { pane_id: 1 });

    // 按下 Shift+W (wW) 開啟 :width
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT))
        .expect("wW");
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "width ");

    // 重設 command mode
    app.command_mode = false;
    app.command_buffer.clear();

    // 按下 Shift+H (wH) 開啟 :height
    app.pending_action = Some(PendingAction::WindowPicker { pane_id: 1 });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT))
        .expect("wH");
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "height ");
}

#[test]
/// 驗證 pane 動態重編號（先上下再左右，Column-Major 空間順序）：
/// 1. 左右分割：左側為 1，右側為 2。
/// 2. 上下分割：上方為 1，下方為 2。
/// 3. 2x2 格狀視窗：左上為 1、左下為 2、右上為 3、右下為 4。
/// 4. 左單欄 + 右雙欄：左側為 1、右上為 2、右下為 3。
/// 5. 左雙欄 + 右單欄：左上為 1、左下為 2、右側為 3。
/// 6. 中途關閉 pane 時動態重算，維持 1..=N 連續無斷號。
fn app_dynamic_pane_renumbering_scenarios() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    assert_eq!(app.ordered_pane_ids(), vec![1]);

    // 情境 1：左右分割 -> 左側 1，右側 2
    app.split_current(SplitDirection::Vertical)
        .expect("split vertical");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2]);
    assert_eq!(app.focused_pane, 2);

    // 關閉右側回到單一 pane
    app.close_current_pane();
    assert_eq!(app.ordered_pane_ids(), vec![1]);
    assert_eq!(app.focused_pane, 1);

    // 情境 2：上下分割 -> 上方 1，下方 2
    app.split_current(SplitDirection::Horizontal)
        .expect("split horizontal");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2]);
    assert_eq!(app.focused_pane, 2);

    // 回到單一 pane
    app.only_current_pane();
    assert_eq!(app.ordered_pane_ids(), vec![1]);

    // 情境 4：左單欄 + 右雙欄 -> 左側為 1、右上為 2、右下為 3
    app.split_current(SplitDirection::Vertical)
        .expect("split vertical");
    app.split_current(SplitDirection::Horizontal)
        .expect("split right horizontally");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2, 3]);
    assert_eq!(app.focused_pane, 3);

    // 回到單一 pane
    app.only_current_pane();
    assert_eq!(app.ordered_pane_ids(), vec![1]);

    // 情境 5：左雙欄 + 右單欄 -> 左上為 1、左下為 2、右側為 3
    app.split_current(SplitDirection::Vertical)
        .expect("split vertical");
    app.focus_pane_by_id(1);
    app.split_current(SplitDirection::Horizontal)
        .expect("split left horizontally");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2, 3]);
    assert_eq!(app.focused_pane, 2);

    // 情境 3：2x2 格狀視窗 -> 左上 1、左下為 2、右上為 3、右下為 4
    app.focus_pane_by_id(3);
    app.split_current(SplitDirection::Horizontal)
        .expect("split right horizontally");
    assert_eq!(app.ordered_pane_ids(), vec![1, 2, 3, 4]);
    assert_eq!(app.focused_pane, 4);

    // 情境 6：中途關閉 pane，維持 1..=N 連續重編號
    app.focus_pane_by_id(2);
    app.close_current_pane();
    assert_eq!(app.ordered_pane_ids(), vec![1, 2, 3]);
    assert_eq!(app.panes.len(), 3);
    assert!(app.panes.contains_key(&1));
    assert!(app.panes.contains_key(&2));
    assert!(app.panes.contains_key(&3));
    assert!(!app.panes.contains_key(&4));
}

#[test]
/// 驗證當目標 panel 已經擁有焦點時，再次切換焦點為 no-op，不會覆寫 status 或觸發冗餘重繪。
fn focus_pane_by_id_noop_when_already_focused() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    assert_eq!(app.focused_pane, 2);

    app.status = String::from("custom status");
    // 目標已是 pane 2，不應改變 status
    app.focus_pane_by_id(2);
    assert_eq!(app.focused_pane, 2);
    assert_eq!(app.status, "custom status");

    // 切到 pane 1，status 應更新為 focused panel 1
    app.focus_pane_by_id(1);
    assert_eq!(app.focused_pane, 1);
    assert_eq!(app.status, "focused panel 1");

    // 再次按 1 聚焦 pane 1，維持原狀
    app.status = String::from("preserve status");
    app.focus_pane_by_id(1);
    assert_eq!(app.focused_pane, 1);
    assert_eq!(app.status, "preserve status");
}
