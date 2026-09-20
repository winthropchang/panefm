use super::*;

#[test]
/// 驗證 command 補全的切換快捷鍵支援多種常見 terminal 回報格式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn command_suggestion_navigation_accepts_terminal_variants() {
    assert_eq!(
        command_suggestion_navigation(&KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT)),
        Some(super::SuggestionNavigation::Next)
    );
    assert_eq!(
        command_suggestion_navigation(&KeyEvent::new(KeyCode::Char('N'), KeyModifiers::CONTROL)),
        Some(super::SuggestionNavigation::Next)
    );
    assert_eq!(
        command_suggestion_navigation(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        Some(super::SuggestionNavigation::Next)
    );
    assert_eq!(
        command_suggestion_navigation(&KeyEvent::new(KeyCode::Char('p'), KeyModifiers::SHIFT)),
        Some(super::SuggestionNavigation::Previous)
    );
    assert_eq!(
        command_suggestion_navigation(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Some(super::SuggestionNavigation::Previous)
    );
}

#[test]
/// 驗證 command mode 也會把 `Shift+6` 正規化成 `^`，避免 regex 指令難以輸入。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_accepts_shifted_caret_symbol() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    app.handle_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::SHIFT))
        .expect("type caret");

    assert_eq!(app.command_buffer, "^");
}

#[test]
/// 驗證 command mode 遇到看起來像路徑的輸入時，Enter 會直接執行，不會先套用補全建議。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_enter_executes_path_like_input_instead_of_autocomplete() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let target = std::path::PathBuf::from("C:/nonexistent_test_path_12345/");
    app.command_mode = true;
    app.command_buffer = target.to_string_lossy().into_owned();

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute path-like input");

    assert!(!app.command_mode);
    assert!(app.command_buffer.is_empty());
    assert!(
        app.status.contains("C:/nonexistent_test_path_12345/")
            || app.status.contains("C:\\nonexistent_test_path_12345\\")
    );
}

#[test]
/// 驗證直接輸入絕對路徑也能跳到目標目錄，不必一定寫 `:goto`。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_bare_path_command_changes_directory() {
    let dir = tempdir().expect("tempdir");
    let docs = dir.path().join("docs");
    fs::create_dir(&docs).expect("docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command(&docs.display().to_string())
        .expect("bare path command");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
}

#[test]
/// 驗證 command mode 在輸入路徑時，會改成列出目前目錄下的路徑候選。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn command_suggestions_switch_to_path_completion_candidates() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("draft.md"), "draft").expect("draft");

    let suggestions = command_suggestions_for_buffer(Some(dir.path()), "goto d");

    assert!(!suggestions.is_empty());
    assert_eq!(suggestions[0].command, "goto docs/");
    assert_eq!(suggestions[0].display_command, "docs/");
    assert!(suggestions[0].shortcut.is_empty());
    assert!(suggestions[0].description.is_empty());
}

#[test]
/// 驗證 UNC 路徑輸入期間不會建立需要讀取網路目錄的即時補全候選。
///
/// 保護目的：command suggestion 會在按鍵與 render 階段反覆計算；若對
/// `//server/share` 呼叫 `read_dir`，Windows 遇到失聯主機時會凍結整個 TUI。
/// 網路路徑必須等 Enter 後交給背景 goto，不能在使用者仍輸入時碰檔案系統。
fn command_suggestions_do_not_scan_unc_paths() {
    let dir = tempdir().expect("tempdir");

    let unc_suggestions =
        command_suggestions_for_buffer(Some(dir.path()), "goto //192.0.2.10/share");
    let smb_suggestions =
        command_suggestions_for_buffer(Some(dir.path()), "goto smb://192.0.2.10/share");

    assert!(unc_suggestions.is_empty());
    assert!(smb_suggestions.is_empty());
}

#[test]
/// 驗證 command mode 在路徑補全模式下按 Tab，會直接把目前候選補進輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_tab_autocompletes_path_candidate() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    for ch in "goto d".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type path command");
    }

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("autocomplete path");

    assert_eq!(app.command_buffer, "goto docs/");
}

#[test]
/// 驗證多個路徑候選存在時，第一次 Tab 會先補到最長共同前綴。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_tab_completes_longest_common_path_prefix_first() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::create_dir(dir.path().join("downloads")).expect("downloads");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    for ch in "goto d".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type path command");
    }

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("complete common prefix");

    assert_eq!(app.command_buffer, "goto do");
}

#[test]
/// 驗證共同前綴補滿後，連按 Tab 會在同一組路徑候選間輪流切換。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_tab_cycles_path_candidates_after_common_prefix() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::create_dir(dir.path().join("downloads")).expect("downloads");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    for ch in "goto do".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type path command");
    }

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("cycle to first candidate");
    let first = app.command_buffer.clone();

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("cycle to second candidate");
    let second = app.command_buffer.clone();

    assert_eq!(first, "goto docs/");
    assert_eq!(second, "goto downloads/");
}

#[test]
/// 驗證 command mode 按下 Tab 時，會直接採用目前最接近的命令提示。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_tab_autocompletes_closest_command_suggestion() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    for ch in "zo".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type zo");
    }

    let suggestions = command_suggestions(&app.command_buffer);
    assert!(!suggestions.is_empty());
    assert_eq!(suggestions[0].command, "zoxide");
    assert_eq!(suggestions[0].shortcut, "Z");
    assert_eq!(app.command_suggestion_selected, 0);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("autocomplete closest suggestion");

    assert_eq!(app.command_buffer, "zoxide");
    assert_eq!(app.command_suggestion_selected, 0);
}

#[test]
/// 驗證 command mode 會接受不同終端送出的候選切換事件格式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_cycles_autocomplete_accepts_terminal_variants() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("type t");

    let suggestions = command_suggestions(&app.command_buffer);
    assert!(!suggestions.is_empty());

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("type n normally");
    assert_eq!(app.command_buffer, "tn");
    assert_eq!(app.command_suggestion_selected, 0);

    app.command_buffer = String::from("t");
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT))
        .expect("next suggestion with lowercase+shift");
    assert_eq!(
        app.command_suggestion_selected,
        1.min(suggestions.len() - 1)
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::CONTROL))
        .expect("next suggestion with uppercase ctrl");
    assert_eq!(
        app.command_suggestion_selected,
        (2).min(suggestions.len().saturating_sub(1))
    );

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("next suggestion with down");
    assert_eq!(
        app.command_suggestion_selected,
        (3).min(suggestions.len().saturating_sub(1))
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE))
        .expect("previous suggestion with uppercase char");
    assert_eq!(
        app.command_suggestion_selected,
        (2).min(suggestions.len().saturating_sub(1))
    );

    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .expect("previous suggestion with up");
    assert_eq!(
        app.command_suggestion_selected,
        (1).min(suggestions.len().saturating_sub(1))
    );
}

#[test]
/// 驗證 command mode 可先用提示切換快捷鍵選中候選，再按 Tab 套用該提示。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_tab_uses_currently_selected_suggestion() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type r");

    let suggestions = command_suggestions(&app.command_buffer);
    assert!(suggestions.len() >= 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT))
        .expect("move to next suggestion");
    let selected = app.command_suggestion_selected;

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("apply selected suggestion");

    assert_eq!(app.command_buffer, suggestions[selected].command);
    assert_eq!(app.command_suggestion_selected, selected);
}

#[test]
/// 驗證 command mode 按下 Enter 會先補齊候選，命令完整時再執行。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_enter_autocompletes_then_executes() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type r");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("autocomplete rename");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "rename");

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute rename");

    assert!(!app.command_mode);
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::Rename { .. })
    ));
}

#[test]
/// 驗證 command mode 在使用者已輸入 `goto smb://...` 時，Enter 會直接執行而不覆蓋成預設模板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_enter_executes_goto_smb_with_arguments() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    for ch in "goto smb://192.0.2.10/tfm-test-share/docs".chars() {
        let modifiers = if ch.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), modifiers))
            .expect("type command");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute command with args");

    assert!(!app.command_mode);
    assert!(app.pending_launch.is_some());
    assert!(
        app.status
            .starts_with("已請求系統掛載 SMB：smb://192.0.2.10/tfm-test-share/docs")
    );
}

#[test]
/// 驗證帶參數的指令提示只會補上 `goto ` 前綴，不會把範例參數塞進輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_command_mode_autocomplete_uses_goto_prefix_instead_of_example_arguments() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");
    for ch in "go".chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type partial command");
    }

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("autocomplete goto");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "goto ");
}

#[test]
/// 驗證 `Shift+;` 也能正確打開命令模式，避免不同終端的事件格式造成 `:` 失效。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_semicolon_opens_command_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
        .expect("open command mode");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "");
    assert_eq!(app.status, "command mode");
}

#[test]
/// 驗證某些終端直接回報 `:` 而不帶 Shift modifier 時，也能正確打開命令模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_plain_colon_opens_command_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
        .expect("open command mode");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "");
    assert_eq!(app.status, "command mode");
}

#[test]
fn command_suggestions_include_width_and_height() {
    let width_suggestions = command_suggestions("width");
    assert!(!width_suggestions.is_empty());
    assert_eq!(width_suggestions[0].command, "width ");
    assert_eq!(width_suggestions[0].shortcut, "wW");

    let height_suggestions = command_suggestions("height");
    assert!(!height_suggestions.is_empty());
    assert_eq!(height_suggestions[0].command, "height ");
    assert_eq!(height_suggestions[0].shortcut, "wH");
}

#[test]
fn command_suggestions_include_vdiff() {
    let suggestions = command_suggestions("vdiff");
    assert!(!suggestions.is_empty());
    assert_eq!(suggestions[0].command, "vdiff");
    assert_eq!(suggestions[0].shortcut, "Ctrl+d");
}
