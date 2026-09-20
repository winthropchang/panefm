use super::*;

#[test]
/// 驗證按下 `O` 會打開 inline `Open with` 小視窗。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_o_opens_open_picker() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
        .expect("open picker");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::OpenPicker { .. })
    ));
}

#[test]
/// 驗證按下 `Shift+Enter` 也會打開 inline `Open with` 小視窗。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_enter_opens_open_picker() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
        .expect("open picker");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::OpenPicker { .. })
    ));
}

#[test]
/// 驗證 open picker 打開後，再按一次 `O` 會直接關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_o_toggles_open_picker_closed() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
        .expect("open picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
        .expect("toggle close open picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 open picker 打開後，再按一次 `Shift+Enter` 也會直接關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_enter_toggles_open_picker_closed() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
        .expect("open picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
        .expect("toggle close open picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證自訂 open action 會出現在 Open with 面板中，並能排入外部啟動佇列。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_open_picker_includes_custom_actions() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut loaded = default_loaded_config();
    loaded
        .config
        .actions
        .open_with
        .push(CustomOpenActionConfig {
            name: "Git log".to_string(),
            scope: ActionTargetScope::Both,
            mode: ActionLaunchMode::TerminalBlocking,
            command: Some("git -C {parent} log --oneline".to_string()),
            mac_command: None,
            windows_command: Some("git -C {parent} log --oneline".to_string()),
        });

    let mut app = App::new(dir.path().to_path_buf(), loaded).expect("app");
    app.open_selected_with_picker().expect("open picker");

    match app.pending_action.as_mut() {
        Some(PendingAction::OpenPicker {
            options, selected, ..
        }) => {
            *selected = options
                .iter()
                .position(|option| option.label == "Git log")
                .expect("custom option");
        }
        other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("queue custom action");

    let launch = app.take_pending_launch().expect("launch");
    assert_eq!(launch.launch.mode, LaunchMode::TerminalBlocking);
    assert!(launch.launch.args.join(" ").contains("git -C"));
    assert!(app.status.contains("running Git log on notes.txt"));
}

#[test]
/// 驗證自訂 Open with 動作若與內建選項同名，會在原位置覆寫內建動作。
///
/// 保護目的：`plugins.toml` 是使用者客製化層；使用者定義 `Vim` 或 `Reveal`
/// 時必須採用外掛命令，不能保留內建動作，也不能在選單中顯示兩次。
fn app_open_picker_custom_actions_override_builtin_names() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello").expect("file");

    let mut loaded = default_loaded_config();
    for name in ["Vim", " reveal ", "Git log", "git LOG"] {
        loaded
            .config
            .actions
            .open_with
            .push(CustomOpenActionConfig {
                name: name.to_string(),
                scope: ActionTargetScope::Both,
                mode: ActionLaunchMode::TerminalBlocking,
                command: Some("echo {path}".to_string()),
                mac_command: None,
                windows_command: None,
            });
    }

    let app = App::new(dir.path().to_path_buf(), loaded).expect("app");
    let target = app.selected_open_target().expect("selected target");
    let options = app.open_picker_options_for_target(&target);

    assert_eq!(
        options
            .iter()
            .filter(|option| option.label.eq_ignore_ascii_case("Vim"))
            .count(),
        1
    );
    let vim = options
        .iter()
        .find(|option| option.label.eq_ignore_ascii_case("Vim"))
        .expect("Vim option");
    assert!(matches!(vim.action, OpenPickerAction::Custom(_)));
    assert_eq!(
        options
            .iter()
            .filter(|option| option.label.trim().eq_ignore_ascii_case("Reveal"))
            .count(),
        1
    );
    let reveal = options
        .iter()
        .find(|option| option.label.trim().eq_ignore_ascii_case("Reveal"))
        .expect("Reveal option");
    assert!(matches!(reveal.action, OpenPickerAction::Custom(_)));
    assert_eq!(
        options
            .iter()
            .filter(|option| option.label.eq_ignore_ascii_case("Git log"))
            .count(),
        1
    );
    let git_log = options
        .iter()
        .find(|option| option.label.eq_ignore_ascii_case("git LOG"))
        .expect("Git log option");
    let OpenPickerAction::Custom(action) = &git_log.action else {
        panic!("Git log should be a custom action");
    };
    assert_eq!(action.name, "git LOG");
}

#[test]
/// 驗證選到資料夾時，預設外部開啟會走系統開啟模式，而不是終端編輯器。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_open_directory_uses_detached_system_open() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open directory");

    let launch = app.take_pending_launch().expect("launch");
    assert_eq!(launch.launch.mode, LaunchMode::Detached);
}

#[test]
/// 驗證按下 `m` 後再按 `s`，會套用 linemode size，而不改變目前排序方式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_linemode_picker_applies_size_without_changing_sort_order() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "1234").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_sort_mode(SortMode::Modified { reverse: true });

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open linemode");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::LineModePicker { pane_id: 1 })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("apply line mode size");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.line_mode, Some(LineMode::Size));
    assert_eq!(pane.sort_mode, SortMode::Modified { reverse: true });
    assert_eq!(app.status, "linemode: size");
}

#[test]
/// 驗證 linemode 面板收到非保留鍵時，不會誤存書籤，而是維持原本面板等待合法指令。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_linemode_picker_ignores_unknown_keys() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open linemode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("ignore unknown key");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::LineModePicker { pane_id: 1 })
    );
    assert_eq!(app.status, "move / linemode: choose a key from the panel");
}

#[test]
/// 驗證 Move / LineMode 面板打開後，按 `r` 會套用 permissions 欄位顯示。
fn app_linemode_shortcut_mr_applies_permissions() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open move linemode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("apply permissions linemode");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.line_mode, Some(LineMode::Permissions));
    assert_eq!(app.status, "linemode: permissions");
}

#[test]
/// 驗證 linemode 面板的 mtime 已改成 `t`，避免和 opener `m` 衝突。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_linemode_picker_t_applies_mtime() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open linemode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("apply mtime linemode");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.line_mode, Some(LineMode::Mtime));
    assert_eq!(app.status, "linemode: mtime");
}

#[test]
/// 驗證等待 linemode 按鍵時打開 F1，離開 help 後仍能回到原本的 linemode 面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_restores_pending_linemode_picker() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open linemode");
    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .expect("open help");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close help");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::LineModePicker { pane_id: 1 })
    );
    assert_eq!(app.status, "move / linemode: choose a key from the panel");
}

#[test]
/// 驗證按下 `t` 會先打開 `t` 系列快捷鍵面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_t_opens_theme_command_picker() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("open theme command picker with t");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ThemeCommandPicker { pane_id: 1 })
    ));
    assert_eq!(app.status, "theme/trash: choose l/n/t/u from the panel");
}

#[test]
/// 驗證 `tl` 會從 `t` 系列面板打開標題為 Theme List 的主題列表。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tl_opens_theme_list() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("open t picker");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("open theme list");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ThemePicker { selected: 3, .. })
    ));
}

#[test]
/// 驗證 `tn` 會從 `t` 系列面板切換下一個主題並保存設定。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tn_cycles_theme() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("open t picker");
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("cycle theme");

    assert_eq!(app.theme_preset, ThemePreset::CatppuccinLatte);
    assert!(
        std::fs::read_to_string(dir.path().join("config.toml"))
            .expect("read config")
            .contains("theme = \"catppuccin-latte\"")
    );
}

#[test]
/// 驗證輪替主題時會切換到下一個預設值。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_cycle_theme_switches_to_next_preset() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.cycle_theme();

    assert_eq!(app.theme_preset, ThemePreset::CatppuccinLatte);
    assert_eq!(app.theme, ThemePreset::CatppuccinLatte.into());
    assert_eq!(app.status, "theme: catppuccin-latte");
}

#[test]
/// 驗證打開主題選擇視窗時，游標會落在目前主題。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_open_theme_picker_tracks_current_preset() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_theme_picker();

    assert_eq!(
        app.pending_action,
        Some(PendingAction::ThemePicker {
            selected: 3,
            original: ThemePreset::CatppuccinMocha,
        })
    );
}

#[test]
/// 驗證依主題名稱字串指定主題時會正確更新狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_set_theme_by_name_updates_theme() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.set_theme_by_name("ocean");

    assert_eq!(app.theme_preset, ThemePreset::Nord);
    assert_eq!(app.theme, ThemePreset::Nord.into());
    assert_eq!(app.status, "theme: nord");
}

#[test]
/// 驗證在主題選擇視窗按下 Enter 後會套用目前選取的主題。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_theme_picker_confirm_applies_selected_theme() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::ThemePicker {
        selected: 2,
        original: ThemePreset::CatppuccinMocha,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("apply theme");

    assert_eq!(app.theme_preset, ThemePreset::Nord);
    assert_eq!(app.theme, ThemePreset::Nord.into());
    assert_eq!(app.status, "theme: nord");
}

#[test]
/// 驗證主題選擇視窗也遵守核心 `h/l` 規則：`l` 套用、`h` 關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_theme_picker_supports_h_and_l_core_navigation() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::ThemePicker {
        selected: 2,
        original: ThemePreset::CatppuccinMocha,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("close theme picker");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "theme picker cancelled");

    app.pending_action = Some(PendingAction::ThemePicker {
        selected: 2,
        original: ThemePreset::CatppuccinMocha,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("apply theme with l");
    assert_eq!(app.theme_preset, ThemePreset::Nord);
    assert_eq!(app.status, "theme: nord");
}

#[test]
/// 驗證主題選擇視窗支援 `j/k` 上下移動，且索引會停在有效範圍內。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_theme_picker_supports_j_and_k_navigation() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::ThemePicker {
        selected: 3,
        original: ThemePreset::CatppuccinMocha,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::ThemePicker {
            selected: 4,
            original: ThemePreset::CatppuccinMocha,
        })
    );
    assert_eq!(app.theme, ThemePreset::CatppuccinLatte.into());
    assert_eq!(app.theme_preset, ThemePreset::CatppuccinMocha);

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("move up");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::ThemePicker {
            selected: 3,
            original: ThemePreset::CatppuccinMocha,
        })
    );
    assert_eq!(app.theme, ThemePreset::CatppuccinMocha.into());
}

#[test]
/// 驗證主題選擇視窗支援 `Ctrl-d/u` 半頁移動，方便快速瀏覽完整主題清單。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_theme_picker_supports_ctrl_page_navigation() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::ThemePicker {
        selected: 0,
        original: ThemePreset::CatppuccinMocha,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("move one page down");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::ThemePicker {
            selected: 10,
            original: ThemePreset::CatppuccinMocha,
        })
    );
    assert_eq!(app.theme, ThemePreset::MonokaiPro.into());

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("move one page up");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::ThemePicker {
            selected: 0,
            original: ThemePreset::CatppuccinMocha,
        })
    );
    assert_eq!(app.theme, ThemePreset::Dracula.into());
}

#[test]
/// 驗證即時預覽後按下 Esc 會還原開啟列表前的主題，且不會修改已保存的主題。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_theme_picker_cancel_restores_original_theme() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let original = app.theme_preset;

    app.open_theme_picker();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("preview next theme");
    assert_ne!(app.theme, Theme::from(original));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel theme preview");

    assert!(app.pending_action.is_none());
    assert_eq!(app.theme, Theme::from(original));
    assert_eq!(app.theme_preset, original);
    assert_eq!(app.config.ui.theme_preset, original);
}

#[test]
/// 驗證排序面板可用 `h` 關閉，避免和整體核心操作規則不一致。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_sort_picker_h_closes_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_sort_picker();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("close sort picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "sort cancelled");
}

#[test]
/// 驗證排序面板打開後，再按一次 `,` 會直接關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_sort_picker_comma_toggles_closed() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_sort_picker();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
        .expect("toggle close sort picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "sort cancelled");
}

#[test]
/// 驗證按下 `,` 後可以用排序面板快捷鍵套用排序模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_sort_picker_applies_selected_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("small.txt"), "a").expect("small");
    fs::write(dir.path().join("large.txt"), "abcdef").expect("large");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
        .expect("open sort picker");
    assert_eq!(
        app.pending_action,
        Some(PendingAction::SortPicker { pane_id: 1 })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("sort by size");
    assert_eq!(app.status, "sort: size");
    assert_eq!(
        app.panes.get(&1).expect("pane").sort_mode,
        SortMode::Size { reverse: false }
    );

    app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
        .expect("open sort picker again");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE))
        .expect("sort by modified reverse");
    assert_eq!(app.status, "sort: modified (reverse)");
    assert_eq!(
        app.panes.get(&1).expect("pane").sort_mode,
        SortMode::Modified { reverse: true }
    );
}

#[test]
/// 驗證 sort panel 也接受 `m + Shift` 這類終端事件，正確套用反向排序。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_sort_picker_shift_m_applies_reverse_modified() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
        .expect("open sort picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::SHIFT))
        .expect("sort by modified reverse");

    assert_eq!(app.status, "sort: modified (reverse)");
    assert_eq!(
        app.panes.get(&1).expect("pane").sort_mode,
        SortMode::Modified { reverse: true }
    );
}
