use super::*;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

use tempfile::tempdir;

use super::{
    App, BACKGROUND_FILE_JOB_THRESHOLD_BYTES, BookmarkListMode, ClipboardEntry, ClipboardOperation,
    ClipboardState, DirectoryLoadEvent, DirectoryLoadJob, FilterState, GlobalSearchState,
    ListFindState, PanelSearchState, PendingAction, RegexRenameOutcome, RenameMode, SearchMode,
    TaskRecord, TaskState, TrashConfirmAction, VisualSelectionState, bookmark_panel_lines,
    command_suggestion_navigation, command_suggestions, command_suggestions_for_buffer,
    ctrl_digit_target_pane_id, filtered_bookmark_entries, filtered_global_search_entries,
    help_entries, is_probably_network_or_external_path, is_windows_drive_path,
    key_matches_ctrl_letter, key_matches_ctrl_shift_letter, key_matches_letter_any_case,
    key_matches_plain_letter, key_matches_shifted_letter, looks_like_navigation_path,
    missing_search_tool_status, paste_should_run_in_background, plain_digit_target_pane_id,
    query_zoxide_directories, rename_basename_cursor, rename_next_word_start,
    rename_previous_word_start, rename_word_end, task_progress_label, trash_confirm_panel_id,
    trash_panel_overlay_state_from_pending_action, typed_char_from_key, visible_job_badge_paths,
};
use crate::{
    config::{
        ActionLaunchMode, ActionTargetScope, AppConfig, CustomOpenActionConfig, LoadedConfig,
        StartupSort,
    },
    file_manager::{
        bookmark::{BookmarkEntry, BookmarkTarget},
        layout::{LayoutNode, SplitDirection},
        open::{LaunchMode, OpenPickerAction},
        pane::{FilterMode, LineMode, PaneState, SortMode},
        search::{GlobalSearchEntry, GlobalSearchEvent},
    },
    theme::{Theme, ThemePreset},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{collections::BTreeSet, fs, sync::mpsc, thread, time::Duration};

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

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 建立測試專用預設設定，避免每個 App 案例重複準備 config 與來源路徑。
fn default_loaded_config() -> LoadedConfig {
    LoadedConfig {
        config: AppConfig::default(),
        source: None,
    }
}

/// 輪詢測試中的背景搜尋直到完成，並設定 timeout 防止失敗時無限等待。
fn wait_for_global_search(app: &mut App) {
    for _ in 0..50 {
        app.poll_background_tasks();
        if app
            .global_search
            .as_ref()
            .is_some_and(|search| search.searched && !search.loading)
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("global search did not complete in time");
}

/// 輪詢測試中的大型檔案工作直到全部完成，避免測試直接依賴執行緒排程速度。
///
/// 保護目的：paste/compress/extract 已移出主執行緒；測試必須驗證 task 完成事件
/// 確實回到 App，而不是以固定 sleep 掩蓋偶發競態。
fn wait_for_file_jobs(app: &mut App) {
    for _ in 0..200 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("background file job did not complete in time");
}

#[test]
/// 驗證文字輸入 helper 會把 `Shift+6` 這類終端事件正規化成真正的符號字元。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn typed_char_from_key_normalizes_shifted_symbols() {
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('6'), KeyModifiers::SHIFT)),
        Some('^')
    );
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('-'), KeyModifiers::SHIFT)),
        Some('_')
    );
    assert_eq!(
        typed_char_from_key(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT)),
        Some('A')
    );
}

#[test]
/// 驗證功能型按鍵 helper 會接受常見的 terminal 事件變體。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn key_normalization_helpers_accept_terminal_variants() {
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        'h'
    ));
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        'j'
    ));
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        'k'
    ));
    assert!(key_matches_plain_letter(
        &KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        'l'
    ));
    assert!(key_matches_shifted_letter(
        &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT),
        'N'
    ));
    assert!(key_matches_shifted_letter(
        &KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE),
        'N'
    ));
    assert!(key_matches_ctrl_letter(
        &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        'p'
    ));
    assert!(key_matches_ctrl_letter(
        &KeyEvent::new(
            KeyCode::Char('P'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        'p'
    ));
    assert!(key_matches_ctrl_shift_letter(
        &KeyEvent::new(
            KeyCode::Char('A'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        'a'
    ));
    assert!(key_matches_letter_any_case(
        &KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        'y'
    ));
    assert!(key_matches_letter_any_case(
        &KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE),
        'y'
    ));
}

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
/// 驗證 normal mode 按下 `R` 會打開預填好的 `rename-regex ` 命令輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_r_opens_prefilled_rename_regex_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
        .expect("open prefilled rename-regex command");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "rename-regex ");
    assert_eq!(app.status, "command mode");
}

#[test]
/// 驗證 normal mode 按下第一個 `g` 會先打開 `g` 系列命令面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_g_opens_go_picker() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("open go picker");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::GoPicker { pane_id: 1 })
    ));
    assert_eq!(app.status, "go: choose g/t/d/k from the panel");
}

#[test]
/// 驗證 normal mode 按下 `gt` 會先經過 `g` 面板，再打開預填好的 `goto ` 命令輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_gt_opens_prefilled_goto_command() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("pending g");
    app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
        .expect("open prefilled goto command");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "goto ");
    assert_eq!(app.status, "command mode");
}

#[test]
/// 驗證 `gd` 會直接切到使用者的 Documents 目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_gd_jumps_to_documents_directory() {
    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let documents = home.join("Documents");
    fs::create_dir_all(&documents).expect("documents");

    let original_home = std::env::var_os("HOME");
    let original_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    let result = (|| {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("open go picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("jump documents");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, documents);
    })();

    unsafe {
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }

    result
}

#[test]
/// 驗證 `gk` 會直接切到使用者的 Desktop 目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_gk_jumps_to_desktop_directory() {
    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let dir = tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let desktop = home.join("Desktop");
    fs::create_dir_all(&desktop).expect("desktop");

    let original_home = std::env::var_os("HOME");
    let original_userprofile = std::env::var_os("USERPROFILE");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
    }

    let result = (|| {
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("open go picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("jump desktop");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, desktop);
    })();

    unsafe {
        match original_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
    }

    result
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
/// 驗證 `:goto <path>` 會讓目前 pane 跳到指定子目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_goto_command_changes_to_target_directory() {
    let dir = tempdir().expect("tempdir");
    let docs = dir.path().join("docs");
    fs::create_dir(&docs).expect("docs");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("goto docs").expect("goto command");

    assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
    assert_eq!(app.status, format!("jumped to path: {}", docs.display()));
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
/// 驗證 Windows 磁碟機路徑會被當成絕對路徑，而不是相對於目前目錄拼接。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn windows_drive_path_is_recognized_as_absolute_like_path() {
    assert!(is_windows_drive_path("C:/"));
    assert!(is_windows_drive_path("D:\\work"));
    assert!(looks_like_navigation_path("R:/repo"));
    assert!(!is_windows_drive_path("docs/readme"));
}

#[test]
/// 驗證 Windows UNC 與 macOS `/Volumes` 目的地都會被視為背景傳輸目標。
///
/// 保護目的：兩個正式支援平台使用不同網路路徑形式；若任一形式漏判，大檔案貼上
/// 就可能退回主執行緒並再次凍結 TUI。
fn network_destination_detection_covers_windows_and_macos() {
    assert!(is_probably_network_or_external_path(std::path::Path::new(
        "//server/share"
    )));
    assert!(is_probably_network_or_external_path(std::path::Path::new(
        "/Volumes/company/share"
    )));
    assert!(!is_probably_network_or_external_path(std::path::Path::new(
        "/Users/otto/Documents"
    )));
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
/// 驗證 UNC goto 的 loader 即使尚未完成，啟動函式仍會立即把控制權交回 TUI。
///
/// 保護目的：公司網路主機不存在或 SMB 回應緩慢時，Windows 檔案系統呼叫可能
/// 等待很久；此測試用 channel 人為暫停 worker，確認主執行緒仍可用 Esc 取消，
/// 且取消後晚到的結果不會覆蓋原本 panel。
fn app_unc_goto_runs_in_background_and_escape_discards_late_result() {
    let dir = tempdir().expect("tempdir");
    let original_cwd = dir.path().to_path_buf();
    let mut app = App::new(original_cwd.clone(), default_loaded_config()).expect("app");
    let target = std::path::PathBuf::from("//192.0.2.10/share");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    app.start_network_goto_with(target, move |mut pane, target| {
        started_tx.send(()).expect("report worker started");
        release_rx.recv().expect("release worker");
        pane.cwd = target;
        Ok(pane)
    })
    .expect("start background goto");

    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("background loader should start without blocking caller");
    assert!(app.active_network_goto_task_id.is_some());

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel network goto");
    release_tx.send(()).expect("finish stale worker");
    thread::sleep(Duration::from_millis(10));
    app.poll_background_tasks();

    assert!(app.active_network_goto_task_id.is_none());
    assert_eq!(app.panes.get(&1).expect("pane").cwd, original_cwd);
    assert!(matches!(
        app.task_log.last().map(|task| task.state),
        Some(TaskState::Cancelled)
    ));
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
/// 驗證一般列表模式下，方向鍵會走和 `hjkl` 相同的移動與進出目錄邏輯。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_normal_mode_arrow_keys_map_to_vim_movement() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    fs::create_dir(&alpha).expect("alpha");
    fs::create_dir(&beta).expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let initial_cwd = app.panes.get(&1).expect("pane").cwd.clone();

    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .expect("move down");
    let selected_after_down = app
        .panes
        .get(&1)
        .expect("pane")
        .selected_entry()
        .expect("selected")
        .path
        .clone();
    assert_eq!(selected_after_down, beta);

    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .expect("move up");
    let selected_after_up = app
        .panes
        .get(&1)
        .expect("pane")
        .selected_entry()
        .expect("selected")
        .path
        .clone();
    assert_eq!(selected_after_up, alpha);

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .expect("enter directory");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, alpha);

    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("go parent");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, initial_cwd);
}

#[test]
/// 驗證一般列表模式用 `l` / `Left` / `Right` 切換目錄後，zoxide 也會同步學習這些位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_normal_mode_directory_navigation_updates_zoxide() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha");
    fs::create_dir(&alpha).expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
        .expect("enter directory");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, alpha);

    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
        .expect("go parent");
    assert_eq!(app.panes.get(&1).expect("pane").cwd, dir.path());

    let tracked = query_zoxide_directories().expect("query zoxide");
    assert!(
        tracked.iter().any(|path| path == &alpha),
        "expected zoxide to contain {}",
        alpha.display()
    );
    assert!(
        tracked.iter().any(|path| path == dir.path()),
        "expected zoxide to contain {}",
        dir.path().display()
    );
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
            },
            ..AppConfig::default()
        },
        source: None,
    };

    let app = App::new(dir.path().to_path_buf(), loaded).expect("app");
    let pane = app.panes.get(&1).expect("pane");

    assert!(pane.show_hidden);
    assert_eq!(pane.sort_mode, SortMode::Size { reverse: true });
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
    }

    app.split_current(SplitDirection::Vertical).expect("split");

    let pane = app.panes.get(&2).expect("new pane");
    assert!(pane.show_hidden);
    assert_eq!(pane.sort_mode, SortMode::Modified { reverse: true });
    assert_eq!(pane.visible_indices.len(), 2);
}

#[test]
/// 驗證刪除確認流程在確認後會真正刪除選取項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_delete_confirmation_removes_selected_entry() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("delete-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete { .. })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm delete");

    assert!(!file_path.exists());
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "trashed delete-me.txt");
}

#[test]
/// 驗證刪除確認視窗再次按 `d` 會關閉視窗，而不會執行刪除或移入 trash。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_delete_confirmation_d_closes_without_deleting() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("keep-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("close delete confirmation");

    assert!(file_path.exists());
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "trash cancelled: keep-me.txt");
}

#[test]
/// 驗證兩個開在同一目錄的 panel，其中一個刪除檔案後，另一個也會同步刷新列表。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_delete_refreshes_other_panels_in_same_directory() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("shared.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.focus_pane_by_id(1);

    app.start_delete_confirmation(false);
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm trash");

    assert!(!file_path.exists());
    for pane_id in [1, 2] {
        let pane = app.panes.get(&pane_id).expect("pane");
        assert!(
            pane.entries.iter().all(|entry| entry.path != file_path),
            "panel {pane_id} still shows deleted file"
        );
    }
}

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
/// 驗證 `:tasks` 會打開目前 pane 的任務面板，且空清單時狀態訊息正確。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tasks_command_opens_task_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.execute_command("tasks").expect("open tasks");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TaskPanel {
            pane_id: 1,
            selected: 0,
            ..
        })
    ));
    assert_eq!(app.status, "tasks: empty");
}

#[test]
/// 驗證一般外部開啟會建立 task，並在主事件迴圈回報成功後標記完成。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_open_task_is_created_and_can_finish() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("queue open");

    assert_eq!(app.task_log.len(), 1);
    let task = app.task_log.last().expect("task");
    assert_eq!(task.kind, "open");
    assert_eq!(task.state, TaskState::Running);

    let queued = app.take_pending_launch().expect("queued launch");
    let task_id = queued.task_id;
    app.finish_launch_task(task_id, Ok(()));

    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.state, TaskState::Done);
    assert_eq!(task.detail, "completed");
    assert!(task.finished_at_unix_ms.is_some());
}

#[test]
/// 驗證 `z` 打開 fzf jump 時會建立 task，取消後也會正確標成 cancelled。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_fzf_jump_task_is_created_and_cancelled() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_fzf_jump();

    assert_eq!(app.task_log.len(), 1);
    let request = app.take_pending_fzf_jump().expect("fzf request");
    let task_id = request.task_id;
    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.kind, "jump");
    assert_eq!(task.state, TaskState::Running);

    app.apply_fzf_jump_selection(request, None);

    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.state, TaskState::Cancelled);
    assert_eq!(task.detail, "fzf cancelled");
}

#[test]
/// 驗證在一般列表按下 Enter 會依預設外部開啟規則排入文字編輯器啟動。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_enter_queues_default_open_for_text_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("default open");

    let launch = app.take_pending_launch().expect("launch");
    let expected = if std::env::var("EDITOR")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        LaunchMode::TerminalBlocking
    } else {
        LaunchMode::Detached
    };
    assert_eq!(launch.launch.mode, expected);
    assert_eq!(app.status, "opening notes.txt with editor");
}

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
/// 驗證按下 `Tab` 會直接進入 preview mode。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tab_opens_preview_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("open preview with tab");

    assert!(app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(app.status, "preview mode");
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
    assert_eq!(app.status, "linemode: choose a key from the panel");
}

#[test]
/// 驗證 linemode 面板打開後，再按一次 `m` 會直接關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_linemode_picker_m_toggles_closed() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("open linemode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
        .expect("toggle close linemode");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
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
    assert_eq!(app.status, "linemode: choose a key from the panel");
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
/// 驗證 task 面板支援 `f` 搜尋，且來源／目的地也能用來篩選任務。
/// 保護目的：檔案操作完成後，使用者常只記得 share 或目錄名稱；若搜尋只比對標題，
/// 新增的診斷位置雖然看得到，卻無法在長期歷史中快速找回。
fn app_task_panel_supports_filtering() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.task_log.push(TaskRecord {
        id: 1,
        pane_id: 1,
        kind: String::from("search"),
        title: String::from("alpha task"),
        detail: String::from("first detail"),
        source_locations: Vec::new(),
        destination_location: None,
        state: TaskState::Done,
        progress_percent: None,
        completed_bytes: None,
        total_bytes: None,
        started_at_unix_ms: 0,
        finished_at_unix_ms: Some(1),
    });
    app.task_log.push(TaskRecord {
        id: 2,
        pane_id: 1,
        kind: String::from("search"),
        title: String::from("beta task"),
        detail: String::from("second detail"),
        source_locations: vec![String::from("/source/report.txt")],
        destination_location: Some(String::from("/share/beta-destination")),
        state: TaskState::Running,
        progress_percent: None,
        completed_bytes: None,
        total_bytes: None,
        started_at_unix_ms: 2,
        finished_at_unix_ms: None,
    });

    app.open_task_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start task filter");
    for ch in "destination".chars() {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            .expect("type task query");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock task filter");

    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel {
            selected, search, ..
        }) => {
            assert_eq!(*selected, 0);
            assert_eq!(search.buffer, "destination");
            assert!(!search.editing);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
    assert_eq!(
        app.status,
        "tasks: 1/1 (d delete, D clear all, v visual, Space mark, x/c cancel, f search)"
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("show filtered task detail");
    assert_eq!(app.status, "task 2 [search] second detail");
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
/// 驗證 help 面板中需要參數的命令，按 Enter 後會打開預填命令，而不是直接執行空參數。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_argument_command_opens_prefilled_command_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start help search");
    for ch in ['C', 't', 'r', 'l', '-', 'p'] {
        let modifiers = if ch.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), modifiers))
            .expect("type help query");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock help search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("open panel command");

    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "panel ");
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
/// 驗證 F1 說明面板可以打開，並在面板內用 `f` 進行搜尋。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_supports_filtering() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .expect("open help");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start help search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("lock help search");

    match app.pending_action.as_ref() {
        Some(PendingAction::HelpPanel { search, .. }) => {
            assert_eq!(search.buffer, "res");
            assert!(!search.editing);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
    let matches = help_entries("res").len();
    assert!(
        matches > 1,
        "fuzzy filter should find non-contiguous matches"
    );
    assert_eq!(app.status, format!("help: res ({matches})"));
}

#[test]
/// 驗證 help 面板搜尋輸入中的 `Tab` 不會誤套用 command hint。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_search_tab_does_not_apply_command_autocomplete() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
        .expect("open help");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("start help search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("type query");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("tab in help search");

    match app.pending_action.as_ref() {
        Some(PendingAction::HelpPanel {
            search, selected, ..
        }) => {
            assert_eq!(search.buffer, "re");
            assert!(search.editing);
            assert_eq!(*selected, 0);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
    assert_eq!(
        app.status,
        format!("help search: re ({})", help_entries("re").len())
    );
}

#[test]
/// 驗證 help 面板已開啟時，再按一次 `~` 會直接關閉回 normal mode。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_tilde_toggles_help_panel_closed() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE))
        .expect("open help with tilde");
    app.handle_key(KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE))
        .expect("close help with tilde");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證某些終端把 `~` 回報成 `Shift+\`` 時，也能正確打開 help 面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_backtick_opens_help_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('`'), KeyModifiers::SHIFT))
        .expect("open help with shift backtick");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel { .. })
    ));
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
/// 驗證按下 `T` 會直接打開目前 pane 的 task 面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_t_opens_task_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .expect("open tasks with T");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::TaskPanel {
            pane_id: 1,
            selected: 0,
            ..
        })
    ));
    assert_eq!(app.status, "tasks: empty");
}

#[test]
/// 驗證 help 面板按下 Enter 後，會直接切到對應的互動模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_enter_executes_selected_action() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_help_panel();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute rename from help");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::Rename { .. })
    ));
}

#[test]
/// 驗證 help 面板在列表模式下按 `h` 會和 `Esc` 一樣關閉，保持與 `l` 的左右對稱操作。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_h_closes_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("close help with h");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 normal mode 的 `J / K` 會用固定大步長快速移動列表游標。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_j_and_k_move_by_large_step() {
    let dir = tempdir().expect("tempdir");
    for index in 0..12 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
        .expect("fast down");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
    assert_eq!(app.status, "fast down: 5");

    app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
        .expect("fast up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(app.status, "fast up: 5");
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

    app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
        .expect("preview fast down");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 5);

    app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
        .expect("preview fast up");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
}

#[test]
/// 驗證 help 面板支援 `J / K` 與 `Ctrl-d / Ctrl-u`，讓大步長與分頁移動可在暫時列表中共用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_supports_fast_and_page_navigation() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
        .expect("help fast down");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 5),
        ref other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("help page down");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 15),
        ref other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("help page up");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 5),
        ref other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
        .expect("help fast up");
    match app.pending_action {
        Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 0),
        ref other => panic!("unexpected pending action: {other:?}"),
    }
}

#[test]
/// 驗證 help 面板中的 `:delete` 會保留 `d` 快捷鍵，並透過 Enter 進入刪除確認。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_help_panel_delete_entry_matches_delete_behavior() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("delete-from-help.txt");
    fs::write(&file_path, "hello").expect("file");

    let entries = help_entries("");
    let delete_entry = entries
        .iter()
        .find(|entry| entry.line.command == ":delete")
        .expect("delete help entry");
    let trash_entry = entries
        .iter()
        .find(|entry| entry.line.command == ":trash")
        .expect("trash help entry");
    let delete_index = entries
        .iter()
        .position(|entry| entry.line.command == ":delete")
        .expect("delete help index");
    assert_eq!(delete_entry.line.shortcut, "d");
    assert_eq!(trash_entry.line.shortcut, "tt");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_help_panel();

    for _ in 0..delete_index {
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move to delete help entry");
    }
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("execute delete from help");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete { .. })
    ));
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
/// 驗證打開重新命名視窗時，會帶入目前選取項目的原名稱與預設輸入值。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_start_rename_opens_dialog_with_selected_name() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_rename();

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("alpha.txt"),
            cursor: 5,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證在重新命名視窗按下 Enter 後會套用新的檔名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_confirm_updates_selected_entry() {
    let dir = tempdir().expect("tempdir");
    let old_path = dir.path().join("alpha.txt");
    let new_path = dir.path().join("beta.txt");
    fs::write(&old_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::Rename {
        pane_id: 1,
        original_name: String::from("alpha.txt"),
        buffer: String::from("beta.txt"),
        cursor: 4,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("rename");

    assert!(!old_path.exists());
    assert!(new_path.exists());
    assert_eq!(app.status, "renamed alpha.txt -> beta.txt");
}

#[test]
/// 驗證 `:rename-regex` 會打開預覽面板，並正確標示 ready / unchanged。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_regex_command_opens_preview_panel() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.md"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("mark second");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");

    app.execute_command("rename-regex '^(.*)\\.txt$' '$1.md'")
        .expect("open regex rename");

    match app.pending_action.as_ref() {
        Some(PendingAction::RegexRename { previews, .. }) => {
            assert_eq!(previews.len(), 2);
            assert_eq!(previews[0].new_name, "alpha.md");
            assert_eq!(previews[0].outcome, RegexRenameOutcome::Ready);
            assert_eq!(previews[1].new_name, "beta.md");
            assert_eq!(previews[1].outcome, RegexRenameOutcome::Unchanged);
        }
        other => panic!("unexpected pending action: {other:?}"),
    }
}

#[test]
/// 驗證 regex 批次改名在按下 Enter 後會一次套用所有 ready 項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_regex_preview_applies_ready_entries() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "a").expect("alpha");
    fs::write(&beta, "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("mark second");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.execute_command("rename-regex '^(.*)\\.txt$' 'file_$1.md'")
        .expect("open regex rename");

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("apply regex rename");

    assert!(!alpha.exists());
    assert!(!beta.exists());
    assert!(dir.path().join("file_alpha.md").exists());
    assert!(dir.path().join("file_beta.md").exists());
    assert_eq!(app.status, "rename-regex: renamed 2 items");
}

#[test]
/// 驗證從命令輸入介面送出 `reg` 後，預覽面板再次按 Enter 會實際完成改名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_regex_rename_command_ui_enter_applies_preview() {
    let dir = tempdir().expect("tempdir");
    let source = dir.path().join("alpha.txt");
    let target = dir.path().join("alpha.md");
    fs::write(&source, "a").expect("source");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
        .expect("open regex command");
    assert!(app.command_mode);
    app.command_buffer = String::from("reg '^(.*)\\.txt$' '$1.md'");

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("submit regex command");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::RegexRename { .. })
    ));

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("apply regex preview");

    assert!(!source.exists());
    assert!(target.exists());
    assert_eq!(app.status, "rename-regex: renamed 1 item");
}

#[test]
/// 驗證 regex 批次改名若會撞名，會標示 conflict，且 Enter 不會直接套用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_rename_regex_preview_blocks_conflicts() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "a").expect("alpha");
    fs::write(&beta, "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("mark second");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.execute_command("rename-regex '^(.*)\\.txt$' 'same.txt'")
        .expect("open regex rename");

    match app.pending_action.as_ref() {
        Some(PendingAction::RegexRename { previews, .. }) => {
            assert!(
                previews
                    .iter()
                    .all(|preview| preview.outcome == RegexRenameOutcome::Conflict)
            );
        }
        other => panic!("unexpected pending action: {other:?}"),
    }

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("try apply conflicting rename");

    assert!(alpha.exists());
    assert!(beta.exists());
    assert_eq!(app.status, "rename-regex: resolve conflicts before apply");
}

#[test]
/// 驗證 rename 預設游標會停在副檔名前，方便優先修改主檔名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_basename_cursor_stops_before_extension() {
    assert_eq!(rename_basename_cursor("alpha.txt"), 5);
    assert_eq!(rename_basename_cursor("archive.tar.gz"), 11);
    assert_eq!(rename_basename_cursor(".gitignore"), 10);
    assert_eq!(rename_basename_cursor("folder"), 6);
}

#[test]
/// 驗證 rename 可以在 insert 與 normal 模式之間切換，並保留游標位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_mode_switches_between_insert_and_normal() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_rename();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("switch to normal");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("alpha.txt"),
            cursor: 5,
            mode: RenameMode::Normal,
        })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("move left");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("back to insert");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("alpha.txt"),
            cursor: 4,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證 rename 的 Vim 單字移動會依照檔名分隔符正確跳轉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_word_motion_helpers_follow_filename_segments() {
    let name = "my-long_file.txt";

    assert_eq!(rename_next_word_start(name, 0), 3);
    assert_eq!(rename_next_word_start(name, 3), 8);
    assert_eq!(rename_next_word_start(name, 8), 13);

    assert_eq!(rename_previous_word_start(name, 13), 8);
    assert_eq!(rename_previous_word_start(name, 8), 3);
    assert_eq!(rename_previous_word_start(name, 3), 0);

    assert_eq!(rename_word_end(name, 0), 1);
    assert_eq!(rename_word_end(name, 3), 6);
    assert_eq!(rename_word_end(name, 8), 11);
    assert_eq!(rename_word_end(name, 12), 15);
}

#[test]
/// 驗證 rename 的 normal 模式支援 `w`、`b`、`e`、`a`、`A` 這些 Vim 風格操作。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn rename_normal_mode_supports_vim_word_motions_and_insert_shortcuts() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("my-long_file.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.start_rename();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("switch to normal");

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
        .expect("move to previous word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("move to next word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("move to word end");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("append after cursor");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("my-long_file.txt"),
            buffer: String::from("my-long_file.txt"),
            cursor: 16,
            mode: RenameMode::Insert,
        })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("back to normal");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE))
        .expect("jump to start");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("jump to next word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
        .expect("jump to end of word");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("append inside basename");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("my-long_file.txt"),
            buffer: String::from("my-long_file.txt"),
            cursor: 7,
            mode: RenameMode::Insert,
        })
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("back to normal again");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE))
        .expect("append at end");

    assert_eq!(
        app.pending_action,
        Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("my-long_file.txt"),
            buffer: String::from("my-long_file.txt"),
            cursor: 16,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證 `y` 複製後可以用 `p` 把檔案貼到另一個目錄，且來源會保留。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_and_paste_preserves_source_file() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("alpha.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy");

    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.operation),
        Some(ClipboardOperation::Copy)
    );
    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.entries.len()),
        Some(1)
    );

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("paste");

    assert!(source_file.exists());
    assert!(target_dir.join("alpha.txt").exists());
    assert_eq!(app.status, "pasted copy: 1 item");
}

#[test]
/// 驗證大型檔案貼上會先建立背景 task，而不是在按下 `p` 時同步完成。
///
/// 保護目的：大 ZIP 或 SMB 傳輸可能需要數分鐘；測試以 sparse 8 MiB 檔觸發
/// 背景門檻，確認主處理函式先返回、完成事件仍會刷新列表並建立 Undo 歷史。
fn app_large_paste_runs_as_background_task_and_records_completion() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("large.zip");
    fs::File::create(&source_file)
        .expect("large source")
        .set_len(BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
        .expect("size source");

    let mut app = App::new(source_dir, default_loaded_config()).expect("app");
    app.copy_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.paste_into_focused_pane().expect("queue paste");

    assert!(!app.file_job_receivers.is_empty());
    assert!(app.status.contains("in background"));
    wait_for_file_jobs(&mut app);

    assert_eq!(
        fs::metadata(target_dir.join("large.zip"))
            .expect("target metadata")
            .len(),
        BACKGROUND_FILE_JOB_THRESHOLD_BYTES
    );
    assert_eq!(app.status, "pasted copy: 1 item");
    assert_eq!(app.operation_history.len(), 1);
    assert!(matches!(
        app.task_log.last().map(|task| task.state),
        Some(TaskState::Done)
    ));
    let task = app.task_log.last().expect("background paste task");
    assert_eq!(
        task.source_locations,
        vec![source_file.display().to_string()],
        "背景貼上必須永久保存實際來源，不可只留下完成訊息"
    );
    assert_eq!(
        task.destination_location,
        Some(target_dir.display().to_string()),
        "背景貼上完成後仍必須能辨識目的目錄"
    );
    assert_eq!(
        app.task_log.last().and_then(|task| task.completed_bytes),
        app.task_log.last().and_then(|task| task.total_bytes),
        "背景貼上完成後已處理 byte 必須等於總 byte，不能停在最後一次中途回報"
    );
}

#[test]
/// 驗證 PaneFM 正常關閉時會把尚未完成的 task 標成 `Interrupted` 並保存到檔案。
///
/// 保護目的：大型本機或 SMB copy 可能執行超過半小時；若使用者關閉程式，舊版
/// 記憶體 task 會完全消失。此測試確保關閉後仍可追查開始時間、進度與中斷原因，
/// 且不會錯誤顯示為仍在執行。
fn app_shutdown_persists_running_tasks_as_interrupted() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "paste",
        String::from("copy large.zip"),
        String::from("destination: share"),
        vec![String::from("/source/large.zip")],
        Some(String::from("/destination/share")),
    );
    app.update_task_progress(task_id, 42, 100);

    app.prepare_for_shutdown().expect("persist shutdown");
    let tasks = super::load_task_history(&app.task_history_path).expect("load history");
    let task = tasks.last().expect("persisted task");

    assert_eq!(task.state, TaskState::Interrupted);
    assert_eq!(task.progress_percent, Some(42));
    assert_eq!(task.completed_bytes, Some(42));
    assert_eq!(task.total_bytes, Some(100));
    assert!(task.finished_at_unix_ms.is_some());
    assert!(task.detail.contains("interrupted when PaneFM closed"));
}

#[test]
/// 驗證啟動時會載入上次 task 歷史，並修正來不及正常關閉的 `Running` 紀錄。
///
/// 保護目的：使用者可能直接關閉 terminal 或系統終止程序，導致關閉 hook 沒機會
/// 執行。下次啟動必須把磁碟上最後一次 RUNNING 快照轉為 `Interrupted`，不能讓 task
/// 面板永久顯示不存在的工作，也不能自動重複覆寫目的檔案。
fn app_startup_recovers_unclean_running_task_history() {
    let dir = tempdir().expect("tempdir");
    let history_path = super::task_history_file_path(dir.path(), None);
    super::save_task_history(
        &history_path,
        &[TaskRecord {
            id: 12,
            pane_id: 8,
            kind: String::from("paste"),
            title: String::from("copy build"),
            detail: String::from("destination: share"),
            source_locations: vec![String::from("/source/build")],
            destination_location: Some(String::from("/destination/share")),
            state: TaskState::Running,
            progress_percent: Some(37),
            completed_bytes: Some(37),
            total_bytes: Some(100),
            started_at_unix_ms: 1_700_000_000_000,
            finished_at_unix_ms: None,
        }],
    )
    .expect("seed history");

    let app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task = app.task_log.last().expect("recovered task");

    assert_eq!(task.state, TaskState::Interrupted);
    assert_eq!(
        task.pane_id, 1,
        "舊 session 的 task 必須出現在目前可見 panel"
    );
    assert_eq!(app.next_task_id, 13);
    assert!(app.status.contains("recovered 1 interrupted task"));
}

#[test]
/// 驗證來源只要是目錄就必須直接判定為背景貼上，不能用目錄本身的 metadata 大小
/// 代表內部內容。測試以 sparse file 表示超過 1 GiB 的大型 build 目錄，不實際寫入
/// 或複製 1 GiB 資料。
///
/// 保護目的：過去 `target/` 雖然包含大量資料，目錄 metadata 卻只有數百 bytes，
/// 因而被錯放到 UI thread 同步複製，造成整個 TUI 卡死。
fn directory_paste_is_background_even_when_directory_metadata_is_small() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("target");
    let destination_dir = dir.path().join("destination");
    fs::create_dir(&source_dir).expect("source directory");
    fs::create_dir(&destination_dir).expect("destination directory");
    fs::File::create(source_dir.join("large-build-output.bin"))
        .expect("sparse source")
        .set_len(1024 * 1024 * 1024 + 1)
        .expect("sparse source size");
    let clipboard = ClipboardState {
        entries: vec![ClipboardEntry {
            source_path: source_dir,
            display_name: String::from("target"),
        }],
        operation: ClipboardOperation::Copy,
    };

    assert!(paste_should_run_in_background(&clipboard, &destination_dir));
}

#[test]
/// 驗證背景進度採用最新 byte 估算，且 task 面板不再只顯示百分比。
///
/// 保護目的：目錄只走訪一次時總量會逐步增加，畫面必須能從早期估算校正成最新
/// 比例，否則前幾個檔案完成後可能長時間錯誤停在 99%。
fn task_progress_uses_latest_dynamic_estimate_and_is_visible_in_panel() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "paste",
        String::from("copy large.zip"),
        String::from("destination: target"),
        vec![String::from("/source/large.zip")],
        Some(String::from("/destination/target")),
    );

    app.update_task_progress(task_id, 60, 100);
    app.update_task_progress(task_id, 20, 100);
    let lines = super::task_panel_lines(&app.task_log, &[]);

    assert_eq!(app.task_log[0].progress_percent, Some(20));
    assert_eq!(lines[0].state, "RUNNING");
    assert_eq!(app.task_log[0].completed_bytes, Some(20));
    assert_eq!(app.task_log[0].total_bytes, Some(100));
    assert_eq!(lines[0].progress, "20B / 100B");
    assert_eq!(lines[0].finished_at, "--:--:--");

    app.finish_task(task_id, TaskState::Done, String::from("completed"));
    assert_eq!(app.task_log[0].progress_percent, Some(100));
    assert_eq!(app.task_log[0].completed_bytes, Some(100));
    let lines = super::task_panel_lines(&app.task_log, &[]);
    assert_ne!(lines[0].finished_at, "--:--:--");
}

#[test]
/// 驗證 task byte 會依數量級切換單位，且不再輸出百分比。
///
/// 保護目的：大型本機與 SMB 傳輸即使百分比長時間不變，使用者仍要從 byte 數判斷
/// 工作是否前進；格式重構不能把資訊退回只有 `31%`。
fn task_progress_label_uses_compact_byte_units_without_percentage() {
    let mut task = TaskRecord {
        id: 1,
        pane_id: 1,
        kind: String::from("paste"),
        title: String::from("copy project"),
        detail: String::new(),
        source_locations: vec![String::from("/source/project")],
        destination_location: Some(String::from("/destination")),
        state: TaskState::Running,
        progress_percent: Some(31),
        completed_bytes: Some(25_589_858_714),
        total_bytes: Some(82_893_350_912),
        started_at_unix_ms: 0,
        finished_at_unix_ms: None,
    };

    assert_eq!(super::task_progress_label(&task), "23.8G / 77.2G");
    assert!(!super::task_progress_label(&task).contains('%'));
    task.completed_bytes = None;
    task.total_bytes = None;
    assert_eq!(super::task_progress_label(&task), "-");
}

#[test]
/// 驗證 `ms` 會在背景遞迴計算每個直接子目錄的檔案總 byte。
///
/// 保護目的：舊版只顯示直接子項目數量，而且同步讀取會拖慢 SMB；這個測試確保
/// linemode 立即啟動 worker、主執行緒可持續 poll，最後得到真正內容大小。
fn size_linemode_scans_recursive_directory_bytes_in_background() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    let sibling = dir.path().join("sibling");
    fs::create_dir_all(child.join("nested")).expect("nested");
    fs::create_dir(&sibling).expect("sibling");
    fs::write(child.join("one.bin"), vec![0u8; 7]).expect("first file");
    fs::write(child.join("nested/two.bin"), vec![0u8; 11]).expect("second file");
    fs::write(child.join(".hidden.bin"), vec![0u8; 5]).expect("hidden file");
    fs::write(child.join(".gitignore"), "ignored.bin\n").expect("ignore rules");
    fs::write(child.join("ignored.bin"), vec![0u8; 17]).expect("ignored file");
    fs::write(sibling.join("three.bin"), vec![0u8; 13]).expect("third file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size)
        .expect("enable size linemode");
    assert!(!app.directory_size_jobs.is_empty());
    assert!(
        app.panes[&1]
            .entries
            .iter()
            .filter(|entry| entry.is_dir)
            .all(|entry| entry.directory_size == Some(0) && !entry.directory_size_complete),
        "啟動背景掃描的當下就要顯示 ~0B，不可長時間停在省略號"
    );
    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_size_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == child)
        .expect("child entry");
    assert_eq!(entry.directory_size, Some(52));
    assert!(entry.directory_size_complete);
    let sibling_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == sibling)
        .expect("sibling entry");
    assert_eq!(sibling_entry.directory_size, Some(13));
    assert!(sibling_entry.directory_size_complete);
    assert!(app.directory_size_jobs.is_empty());
}

#[test]
/// 驗證目錄清單分批載入期間啟用 `ms`，完整清單抵達後會以全部目錄重啟容量掃描。
///
/// 保護目的：舊流程會保留只看過首批項目的同 cwd 掃描，導致稍後加入的目錄右側
/// 永久空白。測試刻意讓舊掃描留在工作表中，再送入含新目錄的完成事件，確保舊
/// worker 被取消、新目錄立即顯示部分值，且最後能取得正確容量。
fn completed_directory_load_restarts_size_scan_for_late_entries() {
    let dir = tempdir().expect("tempdir");
    let early = dir.path().join("early");
    let late = dir.path().join("晚到的目錄");
    fs::create_dir(&early).expect("early directory");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size)
        .expect("enable size linemode");
    let old_scan_cancelled = Arc::clone(&app.directory_size_jobs[&1].cancelled);

    fs::create_dir(&late).expect("late directory");
    fs::write(late.join("payload.bin"), vec![0u8; 31]).expect("late payload");
    let complete_entries = PaneState::new(dir.path().to_path_buf())
        .expect("reload complete entries")
        .entries;
    let (sender, receiver) = mpsc::channel();
    sender
        .send(DirectoryLoadEvent {
            pane_id: 1,
            cwd: dir.path().to_path_buf(),
            selected_path: None,
            result: Ok(super::DirectoryLoadProgress::Complete(complete_entries)),
        })
        .expect("complete directory event");
    app.directory_load_jobs.insert(
        1,
        DirectoryLoadJob {
            cwd: dir.path().to_path_buf(),
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    );

    app.poll_directory_load_jobs();

    assert!(old_scan_cancelled.load(Ordering::Relaxed));
    assert_eq!(app.directory_size_jobs[&1].cwd, dir.path());
    let late_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == late)
        .expect("late entry");
    assert_eq!(late_entry.directory_size, Some(0));
    assert!(!late_entry.directory_size_complete);

    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_size_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let late_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == late)
        .expect("late entry after scan");
    assert_eq!(late_entry.directory_size, Some(31));
    assert!(late_entry.directory_size_complete);
}

#[test]
/// 驗證 `ms` 啟用期間進入下一層目錄，會立即替新列表重新啟動容量掃描。
///
/// 保護目的：舊流程只在首次切換 linemode 時排程；使用 `l` 進入已顯示容量的目錄後，
/// 舊工作因 cwd 不符被取消，新目錄卻永久顯示 `...`。這個測試固定新工作必須綁定
/// 子目錄、子目錄中的資料夾立即顯示部分值，最後取得正確 byte 數。
fn size_linemode_restarts_scan_after_entering_directory() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    let nested = child.join("nested");
    fs::create_dir_all(&nested).expect("nested directory");
    fs::write(nested.join("payload.bin"), vec![0u8; 29]).expect("payload");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size)
        .expect("enable size linemode");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter child directory");

    assert_eq!(app.panes[&1].cwd, child);
    assert!(app.directory_load_jobs.contains_key(&1));
    assert!(
        app.panes[&1].entries.is_empty(),
        "首次進入時應先交還事件迴圈，不能同步等待目錄 I/O"
    );
    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_load_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(app.directory_load_jobs.is_empty());
    assert_eq!(app.directory_size_jobs[&1].cwd, child);
    let nested_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == nested)
        .expect("nested entry");
    assert_eq!(nested_entry.directory_size, Some(0));
    assert!(!nested_entry.directory_size_complete);

    for _ in 0..200 {
        app.poll_background_tasks();
        if app.directory_size_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let nested_entry = app.panes[&1]
        .entries
        .iter()
        .find(|entry| entry.path == nested)
        .expect("nested entry after scan");
    assert_eq!(nested_entry.directory_size, Some(29));
    assert!(nested_entry.directory_size_complete);
    assert!(app.directory_size_jobs.is_empty());

    // 回到父目錄再立刻重進時，已完成的清單必須在按鍵處直接由快取恢復；背景
    // refresh 仍會校正磁碟最新狀態，但畫面不能再次短暫變成空白。
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("return to parent");
    assert!(
        app.panes[&1]
            .entries
            .iter()
            .any(|entry| entry.path == child)
    );
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("re-enter cached child");
    assert!(
        app.panes[&1]
            .entries
            .iter()
            .any(|entry| entry.path == nested)
    );
}

#[test]
/// 驗證 `ms` 背景容量在 200ms 邊界才傳回下一份部分結果。
///
/// 保護目的：太慢會讓大目錄長時間看不到變化，太快則會對每個檔案發送事件並
/// 影響鍵盤操作；測試固定 199ms 不更新、200ms 立即更新的規格。
fn directory_size_partial_updates_use_two_hundred_millisecond_interval() {
    assert!(!super::should_report_directory_size(199, 0));
    assert!(super::should_report_directory_size(200, 0));
    assert!(!super::should_report_directory_size(399, 200));
    assert!(super::should_report_directory_size(400, 200));
}

#[test]
/// 驗證離開 size linemode 會取消該 panel 的背景掃描。
///
/// 保護目的：使用者可能快速切換模式或目錄；若舊 worker 不取消，會持續讀磁碟／SMB
/// 並把晚到結果寫進新的畫面。
fn leaving_size_linemode_cancels_panel_scan() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("child")).expect("child");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.apply_line_mode(1, LineMode::Size).expect("enable size");
    app.apply_line_mode(1, LineMode::None)
        .expect("disable size");

    assert!(app.directory_size_jobs.is_empty());
}

#[test]
/// 驗證背景貼上刷新目的根目錄時，也會更新已經進入其子目錄的 panel。
///
/// 保護目的：使用者可能在大型 copy 尚未完成時進入新建立的 `target/`；舊流程只
/// 刷新目的父目錄，子目錄 panel 會一直保持空白直到整批結束。測試同時確認目的
/// 樹以外的 panel 不會被無關進度反覆 reload。
fn background_destination_refresh_updates_open_descendant_panels_only() {
    let dir = tempdir().expect("tempdir");
    let destination = dir.path().join("destination");
    let child = destination.join("target");
    let unrelated = dir.path().join("unrelated");
    fs::create_dir_all(&child).expect("destination child");
    fs::create_dir(&unrelated).expect("unrelated dir");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .insert(2, PaneState::new(child.clone()).expect("child panel"));
    app.panes.insert(
        3,
        PaneState::new(unrelated.clone()).expect("unrelated panel"),
    );
    fs::write(child.join("copied.txt"), b"visible").expect("copied file");
    fs::write(unrelated.join("not-reloaded.txt"), b"hidden").expect("unrelated file");

    app.reload_panes_in_tree(&destination)
        .expect("refresh destination tree");

    assert!(
        app.panes[&2]
            .entries
            .iter()
            .any(|entry| entry.name == "copied.txt")
    );
    assert!(
        app.panes[&3]
            .entries
            .iter()
            .all(|entry| entry.name != "not-reloaded.txt")
    );
}

#[test]
/// 驗證 worker 會保留原始 byte 變化，而不是只在整數百分比改變時回報。
///
/// 保護目的：若每個 1 MiB buffer 都排入 channel，數十 GB 傳輸會讓主執行緒忙於
/// 處理重複資料；時間節流由 worker 負責，這裡確保完全相同的快照不會重送。
fn progress_events_preserve_byte_changes_and_skip_exact_duplicates() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut last_progress = None;

    super::send_progress_if_changed(&sender, 7, 10, 1_000, &mut last_progress);
    super::send_progress_if_changed(&sender, 7, 10, 1_000, &mut last_progress);
    super::send_progress_if_changed(&sender, 7, 11, 1_000, &mut last_progress);

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(last_progress, Some((11, 1_000)));
    assert!(matches!(
        events.last(),
        Some(super::FileJobEvent::Progress {
            task_id: 7,
            completed_bytes: 11,
            total_bytes: 1_000,
        })
    ));
}

#[test]
/// 驗證單次走訪的動態總量增加時，task 會保留新的 byte 分母。
///
/// 保護目的：目錄 producer 會一邊發現檔案、一邊複製；若只允許百分比增加，前幾個
/// 小檔完成時若只保留百分比，後續增加總量會掩蓋真實進度。
fn progress_events_follow_dynamic_discovered_total() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut last_progress = None;

    super::send_progress_if_changed(&sender, 8, 90, 100, &mut last_progress);
    super::send_progress_if_changed(&sender, 8, 90, 1_000, &mut last_progress);

    let events = receiver.try_iter().collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(last_progress, Some((90, 1_000)));
    assert!(matches!(
        events.last(),
        Some(super::FileJobEvent::Progress {
            task_id: 8,
            completed_bytes: 90,
            total_bytes: 1_000,
        })
    ));
}

#[test]
/// 驗證 `x` 剪下後可以用 `p` 移動檔案，且剪貼簿會在成功後清空。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_cut_and_paste_moves_file_and_clears_clipboard() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("beta.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .expect("cut");

    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.operation),
        Some(ClipboardOperation::Cut)
    );
    assert_eq!(
        app.clipboard.as_ref().map(|entry| entry.entries.len()),
        Some(1)
    );

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("paste");

    assert!(!source_file.exists());
    assert!(target_dir.join("beta.txt").exists());
    assert!(app.clipboard.is_none());
    assert_eq!(app.status, "moved: 1 item");
}

#[test]
/// 驗證一般模式按下 `u` 會復原最近一次 Copy，來源保留且目的檔移入 Trash。
///
/// 保護目的：確保操作歷史不只底層可用，實際快捷鍵也能完成使用者最常見的貼錯復原。
fn app_u_undoes_latest_copy_paste() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "hello").expect("source file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.copy_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir;
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.paste_into_focused_pane().expect("paste");
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
        .expect("undo shortcut");

    assert!(source_file.exists());
    assert!(!target_file.exists());
    assert_eq!(app.status, "undid copy: 1 items");
    assert_eq!(
        app.trash_store.list_entries().expect("trash entries").len(),
        1
    );
}

#[test]
/// 驗證 `:undo` 可以把 cut/paste 的目的檔搬回原來源位置。
///
/// 保護目的：命令與快捷鍵必須呼叫相同 Undo 核心，避免兩套入口行為不一致。
fn app_undo_command_reverses_cut_paste() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("beta.txt");
    let target_file = target_dir.join("beta.txt");
    fs::write(&source_file, "hello").expect("source file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.cut_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir;
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.paste_into_focused_pane().expect("paste");
    app.execute_command("undo").expect("undo command");

    assert!(source_file.exists());
    assert!(!target_file.exists());
    assert_eq!(app.status, "undid move: 1 items");
}

#[test]
/// 驗證覆蓋貼上後執行 Undo，會恢復目的地原內容而不是只移除新檔。
///
/// 保護目的：覆蓋是最高風險操作，必須證明隱藏備份已接入 App 的完整流程。
fn app_undo_overwrite_paste_restores_previous_target() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    fs::write(source_dir.join("same.txt"), "new").expect("new source");
    let target_file = target_dir.join("same.txt");
    fs::write(&target_file, "old").expect("old target");

    let mut app = App::new(source_dir, default_loaded_config()).expect("app");
    app.copy_selected();
    app.current_pane_mut().expect("pane").cwd = target_dir;
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.paste_into_focused_pane_with_overwrite()
        .expect("overwrite paste");
    app.undo_latest_file_operation().expect("undo overwrite");

    assert_eq!(
        fs::read_to_string(target_file).expect("restored target"),
        "old"
    );
    assert_eq!(app.status, "undid copy: 1 items");
}

#[test]
/// 驗證按下 `Space` 會切換目前選取項目的標記狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_space_toggles_mark_on_selected_entry() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .expect("mark selected");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 1);
    assert_eq!(app.status, "marked alpha.txt");

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .expect("unmark selected");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
    assert_eq!(app.status, "unmarked alpha.txt");
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
    assert_eq!(app.ordered_pane_ids(), vec![2, 1]);
    assert_eq!(app.focused_pane, 2);
    assert_eq!(app.status, "split left");

    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open w leader");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("split down");
    assert_eq!(app.focused_pane, 3);
    assert_eq!(app.status, "split down");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("open w leader");
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("split up");
    assert_eq!(app.ordered_pane_ids(), vec![2, 1]);
    assert_eq!(app.focused_pane, 2);
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
/// 驗證按下 `Ctrl-r` 會反轉目前所有可見項目的標記狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_r_inverts_visible_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .expect("invert marks");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);
    assert_eq!(app.status, "inverted visible marks (+2, -0, total 2)");

    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
        .expect("invert marks again");
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
    assert_eq!(app.status, "inverted visible marks (+0, -2, total 0)");
}

#[test]
/// 驗證按下 `Y` / `X` 可以清掉目前內部剪貼簿中的 copy / cut 狀態。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_y_and_shift_x_clear_clipboard_state() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy");
    app.handle_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT))
        .expect("clear copied items");
    assert!(app.clipboard.is_none());
    assert_eq!(app.status, "cleared copied items");

    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .expect("cut");
    app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("clear cut items");
    assert!(app.clipboard.is_none());
    assert_eq!(app.status, "cleared cut items");
}

#[test]
/// 驗證按下 `P` 會以覆蓋模式貼上，而不是自動產生 `copy` 檔名。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_p_pastes_with_overwrite_when_clipboard_exists() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "from source").expect("source file");
    fs::write(&target_file, "from target").expect("target file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy");

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");
    app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT))
        .expect("overwrite paste");

    assert_eq!(
        fs::read_to_string(&target_file).expect("target content"),
        "from source"
    );
    assert!(!target_dir.join("alpha copy.txt").exists());
    assert_eq!(app.status, "pasted copy with overwrite: 1 item");
}

#[test]
/// 驗證按下 `D` 後確認，會直接永久刪除目前選取項目而不是丟進 trash。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_d_deletes_selected_entry_permanently() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("delete-me.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("start permanent delete");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete {
            permanent: true,
            ..
        })
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm permanent delete");

    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(!file_path.exists());
    assert!(
        app.trash_store
            .list_entries()
            .expect("trash entries")
            .is_empty()
    );
    assert_eq!(app.status, "deleted permanently delete-me.txt");
}

#[test]
/// 驗證 `:move <path>` 會把目前選取的檔案直接移到指定目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_move_command_moves_selected_entry_to_target_dir() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("gamma.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.execute_command(&format!("move {}", target_dir.display()))
        .expect("move command");

    assert!(!source_file.exists());
    assert!(target_dir.join("gamma.txt").exists());
    assert_eq!(
        app.status,
        format!("moved 1 item -> {}", target_dir.display())
    );
}

#[test]
/// 驗證 `:move-panel <id>` 會把目前選取的檔案移到指定 pane 的目錄。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_move_panel_command_moves_selected_entry_to_target_pane_dir() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source dir");
    fs::create_dir(&target_dir).expect("target dir");
    let source_file = source_dir.join("delta.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");
    app.focus_pane_by_id(1);

    app.execute_command("move-panel 2").expect("move panel");

    assert!(!source_file.exists());
    assert!(target_dir.join("delta.txt").exists());
    assert_eq!(
        app.status,
        format!("moved 1 item -> {}", target_dir.display())
    );
}

#[test]
/// 驗證 `:compress` 會把目前選取項目壓成 zip，並把游標帶到新壓縮檔。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_compress_command_creates_zip_and_reveals_result() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("notes.txt");
    fs::write(&file_path, "hello zip").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("compress").expect("compress");

    let archive_path = dir.path().join("notes.txt.zip");
    assert!(archive_path.exists());
    assert_eq!(app.status, "compressed notes.txt -> notes.txt.zip");
    assert_eq!(
        app.current_pane_mut()
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "notes.txt.zip"
    );
}

#[test]
/// 驗證 `:extract` 會解開目前選取的 zip，並將游標帶到輸出目錄。
/// 保護目的：同時確認資料夾壓縮會先排入背景 task，完成後仍能選中新 ZIP 並接續
/// 解壓，避免非阻塞重構破壞原本的完整操作流程。
fn app_extract_command_unpacks_zip_and_reveals_output() {
    let dir = tempdir().expect("tempdir");
    let folder = dir.path().join("demo");
    fs::create_dir(&folder).expect("dir");
    fs::write(folder.join("alpha.txt"), "hello").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("compress").expect("compress dir");
    assert!(
        !app.file_job_receivers.is_empty(),
        "directory compression must not block the TUI thread"
    );
    wait_for_file_jobs(&mut app);

    let archive_path = dir.path().join("demo.zip");
    assert!(archive_path.exists());

    app.execute_command("extract").expect("extract zip");

    let extracted_dir = dir.path().join("demo copy");
    assert!(extracted_dir.is_dir());
    assert!(extracted_dir.join("demo").join("alpha.txt").exists());
    assert_eq!(app.status, "extracted demo copy");
    assert_eq!(
        app.current_pane_mut()
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "demo copy"
    );
}

#[test]
/// 驗證已掛載的 SMB share 可以直接經由 `goto smb://...` 切進目前 pane。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_goto_smb_location_enters_mounted_share() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    let share_root = mount_root.join("shared");
    fs::create_dir_all(share_root.join("docs")).expect("share docs");
    fs::write(share_root.join("docs").join("report.txt"), "hello").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root("smb://192.0.2.10/shared/docs", &mount_root)
        .expect("goto smb");

    let pane = app.current_pane_mut().expect("pane");
    assert_eq!(pane.cwd, share_root.join("docs"));
    assert_eq!(app.status, "jumped to smb: smb://192.0.2.10/shared/docs");
    assert!(app.take_full_redraw_request());
    assert!(!app.take_full_redraw_request());
}

#[test]
/// 驗證尚未掛載的 SMB share 在 `goto smb://...` 時會先發出系統掛載請求。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_goto_smb_location_requests_mount_when_share_missing() {
    let dir = tempdir().expect("tempdir");
    let mount_root = dir.path().join("mounts");
    fs::create_dir_all(&mount_root).expect("mount root");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.goto_smb_location_with_mount_root("smb://192.0.2.10/shared/docs", &mount_root)
        .expect("goto smb");

    assert!(app.pending_launch.is_some());
    assert_eq!(
        app.status,
        format!(
            "已請求系統掛載 SMB：smb://192.0.2.10/shared/docs；若系統連線失敗，請檢查主機、share 名稱、網路與權限，成功後再重試。預期掛載位置：{}",
            mount_root.join("shared").join("docs").display()
        )
    );
}

#[test]
/// 驗證 `:move-panel <id>` 若指定不存在的 pane，會提示目前可用的 pane 編號。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_move_panel_command_reports_available_panes_for_unknown_target() {
    let dir = tempdir().expect("tempdir");
    let source_file = dir.path().join("epsilon.txt");
    fs::write(&source_file, "hello").expect("file");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("move-panel 9").expect("move panel");

    assert!(source_file.exists());
    assert_eq!(app.status, "unknown panel 9. available: 1");
}

#[test]
/// 驗證按下 `o` 後會打開建立新檔案的 inline 輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_start_create_entry_opens_inline_editor() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.start_create_entry();

    assert_eq!(
        app.pending_action,
        Some(PendingAction::CreateEntry {
            pane_id: 1,
            buffer: String::new(),
            cursor: 0,
            mode: RenameMode::Insert,
        })
    );
}

#[test]
/// 驗證命令模式可以直接建立一般檔案與結尾 `/` 的資料夾。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_create_commands_create_entries_without_inline_prompt() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.execute_command("create alpha.txt")
        .expect("create file");
    assert!(dir.path().join("alpha.txt").exists());
    assert_eq!(app.status, "created file: alpha.txt");

    app.execute_command("create docs/").expect("create dir");
    assert!(dir.path().join("docs").is_dir());
    assert_eq!(app.status, "created directory: docs/");
}

#[test]
/// 驗證建立流程的 inline 輸入框在 Enter 後會真的建立檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_create_file_confirm_creates_entry() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::CreateEntry {
        pane_id: 1,
        buffer: String::from("draft.md"),
        cursor: 8,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("create file");

    assert!(dir.path().join("draft.md").exists());
    assert_eq!(app.status, "created file: draft.md");
}

#[test]
/// 驗證建立流程支援巢狀路徑，會先補齊父目錄再建立檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_create_nested_file_from_inline_prompt() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::CreateEntry {
        pane_id: 1,
        buffer: String::from("test/gg.txt"),
        cursor: 11,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("create nested file");

    assert!(dir.path().join("test").is_dir());
    assert!(dir.path().join("test").join("gg.txt").exists());
    assert_eq!(app.status, "created file: test/gg.txt");
}

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
/// 驗證按下 `.` 後會顯示隱藏檔，並可與 filter 一起使用。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_toggle_hidden_reveals_hidden_entries_and_works_with_filter() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".secret"), "s").expect("hidden");
    fs::write(dir.path().join("alpha.txt"), "a").expect("normal");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let initial_names: Vec<String> = app
        .panes
        .get(&1)
        .expect("pane")
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();
    assert_eq!(initial_names, vec![String::from("alpha.txt")]);

    app.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
        .expect("toggle hidden");
    assert_eq!(app.status, "showing hidden files");

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open filter");
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
        .expect("filter hidden");

    let filtered_names: Vec<String> = app
        .panes
        .get(&1)
        .expect("pane")
        .visible_entries()
        .into_iter()
        .map(|entry| entry.display_name())
        .collect();
    assert_eq!(filtered_names, vec![String::from(".secret")]);
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
    assert!(app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(app.status, "preview mode");

    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("scroll down");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("scroll up");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("leave preview");
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
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
    assert!(app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(app.status, "preview mode");

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("toggle preview off");
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
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

    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("page down");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE))
        .expect("bottom");
    let bottom_scroll = app.panes.get(&1).expect("pane").preview_scroll;
    assert!(bottom_scroll > 2);

    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("pending g");
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
        .expect("top");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("page down again");
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("page up");
    assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
}

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
        .expect("leave preview");
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
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
    assert!(app.panes.get(&2).expect("panel 2").is_preview_active());

    app.focus_pane_by_id(1);
    app.open_preview_focus();
    assert!(app.panes.get(&1).expect("panel 1").is_preview_active());
    assert!(app.panes.get(&2).expect("panel 2").is_preview_active());

    app.split_current(SplitDirection::Horizontal)
        .expect("split third panel");
    app.open_preview_focus();
    assert_eq!(app.focused_pane, 3);
    assert!(app.panes.get(&1).expect("panel 1").is_preview_active());
    assert!(app.panes.get(&2).expect("panel 2").is_preview_active());
    assert!(app.panes.get(&3).expect("panel 3").is_preview_active());

    app.focus_pane_by_id(2);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
        .expect("close only panel 2 preview");
    assert!(app.panes.get(&1).expect("panel 1").is_preview_active());
    assert!(!app.panes.get(&2).expect("panel 2").is_preview_active());
    assert!(app.panes.get(&3).expect("panel 3").is_preview_active());
}

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
/// 驗證 task 面板中的 `x` 可以取消目前正在進行的 search task。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_task_panel_x_cancels_running_search_task() {
    let dir = tempdir().expect("tempdir");
    let cancelled = Arc::new(AtomicBool::new(false));

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "search",
        String::from("content search: needle"),
        format!("root: {}", dir.path().display()),
        vec![dir.path().display().to_string()],
        None,
    );
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        mode: SearchMode::Content,
        buffer: String::from("needle"),
        editing: false,
        loading: true,
        searched: false,
        selected: 0,
        results: Vec::new(),
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
        task_id: Some(task_id),
    });
    app.active_global_search_task_id = Some(task_id);
    app.global_search_cancelled = Some(cancelled.clone());
    app.open_task_panel();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        .expect("cancel task");

    let task = app
        .task_log
        .iter()
        .find(|task| task.id == task_id)
        .expect("task");
    assert_eq!(task.state, TaskState::Cancelled);
    assert!(app.global_search.is_none());
    assert!(app.global_search_rx.is_none());
    assert!(app.global_search_cancelled.is_none());
    assert!(cancelled.load(Ordering::Relaxed));
    assert_eq!(app.status, format!("cancelled task {task_id}"));
}

#[test]
/// 驗證 task 面板中的 `X` 會取消目前 panel 內所有可取消的任務。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_task_panel_shift_x_cancels_all_running_tasks() {
    let dir = tempdir().expect("tempdir");
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let task_id = app.push_task(
        1,
        "search",
        String::from("content search: needle"),
        format!("root: {}", dir.path().display()),
        vec![dir.path().display().to_string()],
        None,
    );
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        mode: SearchMode::Content,
        buffer: String::from("needle"),
        editing: false,
        loading: true,
        searched: false,
        selected: 0,
        results: Vec::new(),
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
        task_id: Some(task_id),
    });
    app.active_global_search_task_id = Some(task_id);
    app.global_search_cancelled = Some(cancelled.clone());
    app.open_task_panel();

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("cancel all tasks");

    assert_eq!(
        app.task_log
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.state),
        Some(TaskState::Cancelled)
    );
    assert!(cancelled.load(Ordering::Relaxed));
    assert_eq!(app.status, "cancelled 1 tasks");
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
/// 驗證可以用 `V` 視覺標記多個項目，並一次放進剪貼簿。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_marked_entries_copy_into_clipboard_as_batch() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy batch");

    let clipboard = app.clipboard.as_ref().expect("clipboard");
    assert_eq!(clipboard.operation, ClipboardOperation::Copy);
    assert_eq!(clipboard.entries.len(), 2);
    assert_eq!(app.status, "copied 2 items");
}

#[test]
/// 驗證 `V` 視覺標記多個項目後，刪除確認會一次刪掉整批項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_marked_entries_delete_as_batch() {
    let dir = tempdir().expect("tempdir");
    let alpha = dir.path().join("alpha.txt");
    let beta = dir.path().join("beta.txt");
    fs::write(&alpha, "a").expect("alpha");
    fs::write(&beta, "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.start_delete_confirmation(false);

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmDelete { ref target_name, .. }) if target_name == "2 items"
    ));

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("confirm delete batch");

    assert!(!alpha.exists());
    assert!(!beta.exists());
    assert_eq!(app.status, "trashed 2 items");
}

#[test]
/// 驗證 `V` 進入 visual selection 後，移動游標再按一次 `V` 會提交整段標記。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_selection_commits_range_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down again");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");

    let pane = app.panes.get(&1).expect("pane");
    assert!(app.visual_selection.is_none());
    assert_eq!(pane.marked_count(), 3);
    assert_eq!(app.status, "marked 3 items");
}

#[test]
/// 驗證小寫 `v` 可以進入、移動並結束 visual selection。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_lowercase_v_controls_visual_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("extend visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("close visual");

    assert!(app.visual_selection.is_none());
    assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);
}

#[test]
/// 驗證某些終端把 `Shift+v` 回報成 `v + Shift` 時，也能正確進入 visual selection。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_v_opens_visual_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SHIFT))
        .expect("open visual with shifted v");

    assert_eq!(
        app.visual_selection,
        Some(VisualSelectionState {
            pane_id: 1,
            anchor: 0,
            current: 0,
        })
    );
    assert_eq!(app.status, "visual: range selection");
}

#[test]
/// 驗證某些終端把 `Shift+g` 回報成 `g + Shift` 時，也能正確執行 `G` 跳到列表底部。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_shift_g_jumps_to_bottom() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT))
        .expect("jump bottom with shifted g");

    assert_eq!(app.panes.get(&1).expect("pane").selected, 2);
    assert_eq!(app.status, "jumped to bottom");
}

#[test]
/// 驗證 visual selection 按下 `Esc` 會先提交這一段範圍並離開選取模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_selection_escape_commits_current_range() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit first range");

    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move to third");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open second visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("move back");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("commit second visual");

    let pane = app.panes.get(&1).expect("pane");
    assert!(app.visual_selection.is_none());
    assert_eq!(pane.marked_count(), 3);
    assert_eq!(app.status, "marked 1 items");
}

#[test]
/// 驗證離開選取模式後再按一次 `Esc`，會清掉目前所有已提交標記。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_escape_in_normal_mode_clears_all_marks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("open visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("commit visual");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("clear all marks");

    let pane = app.panes.get(&1).expect("pane");
    assert!(app.visual_selection.is_none());
    assert_eq!(pane.marked_count(), 0);
    assert_eq!(app.status, "cleared 2 marks");
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
/// 驗證一般貼上遇到同名檔案時，會先開啟覆蓋確認視窗，使用者確認後才覆蓋。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_paste_with_conflict_requires_confirmation_before_overwrite() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "from source").expect("source file");
    fs::write(&target_file, "from target").expect("target file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy");

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open overwrite confirm");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::ConfirmPasteOverwrite { .. })
    ));
    assert_eq!(
        fs::read_to_string(&target_file).expect("target content before confirm"),
        "from target"
    );

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("confirm overwrite");

    assert_eq!(
        fs::read_to_string(&target_file).expect("target content after confirm"),
        "from source"
    );
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "pasted copy with overwrite: 1 item");
}

#[test]
/// 驗證一般貼上遇到同名檔案時，若使用者取消，會保留原檔案且不執行貼上。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_paste_with_conflict_can_be_cancelled() {
    let dir = tempdir().expect("tempdir");
    let source_dir = dir.path().join("source");
    let target_dir = dir.path().join("target");
    fs::create_dir(&source_dir).expect("source");
    fs::create_dir(&target_dir).expect("target");
    let source_file = source_dir.join("alpha.txt");
    let target_file = target_dir.join("alpha.txt");
    fs::write(&source_file, "from source").expect("source file");
    fs::write(&target_file, "from target").expect("target file");

    let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
        .expect("copy");

    app.current_pane_mut().expect("pane").cwd = target_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload target");

    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
        .expect("open overwrite confirm");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel overwrite");

    assert_eq!(
        fs::read_to_string(&target_file).expect("target content after cancel"),
        "from target"
    );
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "paste cancelled: alpha.txt");
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

#[test]
/// 驗證按下 `z` 後會建立 `fzf` 跳轉請求，並記住目前 pane 的根目錄設定。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_jump_key_queues_fzf_request() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("readme.md"), "b").expect("readme");
    fs::write(dir.path().join("report.txt"), "c").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
        .expect("open jump");

    let request = app.take_pending_fzf_jump().expect("fzf request");

    assert_eq!(request.pane_id, 1);
    assert_eq!(request.root_dir, dir.path());
    assert!(request.show_hidden);
    assert!(request.follow_links);
    assert!(app.pending_fzf_jump.is_none());
    assert_eq!(app.status, "jump: fzf loading");
}

#[test]
/// 驗證分割成多個 pane 後，在目前 focus 的 pane 按下 `z` 仍會建立 `fzf` 請求。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_jump_key_works_from_focused_split_pane() {
    let dir = tempdir().expect("tempdir");
    let left_dir = dir.path().join("left");
    let right_dir = dir.path().join("right");
    fs::create_dir(&left_dir).expect("left");
    fs::create_dir(&right_dir).expect("right");
    fs::write(left_dir.join("alpha.txt"), "a").expect("alpha");
    fs::write(right_dir.join("beta.txt"), "b").expect("beta");

    let mut app = App::new(left_dir.clone(), default_loaded_config()).expect("app");
    app.split_current(SplitDirection::Vertical).expect("split");
    app.current_pane_mut().expect("pane").cwd = right_dir.clone();
    app.current_pane_mut()
        .expect("pane")
        .reload()
        .expect("reload");

    assert_eq!(app.focused_pane, 2);
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
        .expect("open jump");

    let request = app.take_pending_fzf_jump().expect("fzf request");

    assert_eq!(request.pane_id, 2);
    assert_eq!(request.root_dir, right_dir);
    assert!(request.show_hidden);
    assert!(request.follow_links);
    assert_eq!(app.status, "jump: fzf loading");
}

#[test]
/// 驗證 `z` 使用的 `fzf` 搜尋會固定包含 hidden 內容，不受 pane 顯示設定影響。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_jump_key_always_searches_hidden_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".secret.txt"), "secret").expect("secret");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.current_pane_mut().expect("pane").show_hidden = false;
    app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
        .expect("open jump");

    let request = app.take_pending_fzf_jump().expect("fzf request");
    assert!(request.show_hidden);
    assert!(request.follow_links);
}

#[test]
/// 驗證套用 `fzf` 選取結果後，游標會跳到對應的檔案。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_apply_fzf_jump_selection_moves_cursor_to_match() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("readme.md"), "b").expect("readme");
    fs::write(dir.path().join("report.txt"), "c").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_fzf_jump();
    let request = app.take_pending_fzf_jump().expect("fzf request");
    app.apply_fzf_jump_selection(request, Some("report.txt"));

    assert_eq!(
        app.panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .name,
        "report.txt"
    );
    assert_eq!(app.status, "jumped: report.txt");
}

#[test]
/// 驗證套用巢狀 `fzf` 結果後，pane 會切到檔案所在目錄並聚焦正確項目。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_apply_fzf_jump_selection_reveals_nested_file() {
    let dir = tempdir().expect("tempdir");
    let nested_dir = dir.path().join("docs");
    fs::create_dir(&nested_dir).expect("docs");
    fs::write(nested_dir.join("guide.md"), "guide").expect("guide");
    fs::write(dir.path().join("root.txt"), "root").expect("root");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_fzf_jump();
    let request = app.take_pending_fzf_jump().expect("fzf request");
    app.apply_fzf_jump_selection(request, Some("docs/guide.md"));

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.cwd, nested_dir);
    assert_eq!(pane.selected_entry().expect("selected").name, "guide.md");
    assert_eq!(app.status, "jumped: docs/guide.md");
}

#[test]
/// 驗證取消 `fzf` 選擇時，不會改動目前游標位置。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_apply_fzf_jump_selection_cancel_keeps_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("readme.md"), "b").expect("readme");
    fs::write(dir.path().join("report.txt"), "c").expect("report");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move to readme");
    let original = app.panes.get(&1).expect("pane").selected;

    app.open_fzf_jump();
    let request = app.take_pending_fzf_jump().expect("fzf request");
    app.apply_fzf_jump_selection(request, None);

    assert_eq!(app.panes.get(&1).expect("pane").selected, original);
    assert_eq!(app.status, "jump cancelled");
}

#[test]
/// 驗證 normal mode 按下 `Ctrl-a` 會把目前 pane 的所有可見項目全部標記起來。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_a_marks_all_visible_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");
    fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
        .expect("mark all");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.marked_count(), 3);
    assert_eq!(app.status, "marked all visible items (+3, total 3)");
}

#[test]
/// 驗證 `:mark-all` 命令也能把目前 pane 的所有可見項目全部標記起來。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_mark_all_command_marks_all_visible_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("mark-all").expect("mark-all command");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.marked_count(), 2);
    assert_eq!(app.status, "marked all visible items (+2, total 2)");
}

#[test]
/// 驗證 normal mode 按下 `c` 會打開文字複製小視窗。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_c_key_opens_copy_picker() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("open copy picker");

    match app.pending_action {
        Some(PendingAction::CopyPicker {
            pane_id, selected, ..
        }) => {
            assert_eq!(pane_id, 1);
            assert_eq!(selected, 0);
        }
        other => panic!("expected copy picker, got {other:?}"),
    }
    assert_eq!(app.status, "copy to clipboard: alpha.txt");
}

#[test]
/// 驗證文字複製小視窗按下 `h` 會關閉並回到一般模式。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_picker_h_closes_panel() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_copy_picker().expect("open copy picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("close copy picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證文字複製小視窗打開後，再按一次 `c` 會直接關閉。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_picker_c_toggles_closed() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_copy_picker().expect("open copy picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("toggle close copy picker");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證文字複製小視窗中，原本的檔案路徑複製已改成 `u`，避免和 opener `c` 衝突。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_copy_picker_u_copies_file_path() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "a").expect("alpha");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.open_copy_picker().expect("open copy picker");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
        .expect("copy file path");

    assert_eq!(app.status, "copied file path: alpha.txt");
}

#[test]
/// 驗證 normal mode 按下 `Ctrl-Shift-A` 會清掉目前 pane 的所有標記。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_shift_a_clears_all_marks_in_focused_pane() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "b").expect("beta");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.execute_command("mark-all").expect("mark-all command");
    app.handle_key(KeyEvent::new(
        KeyCode::Char('A'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ))
    .expect("clear marks");

    let pane = app.panes.get(&1).expect("pane");
    assert_eq!(pane.marked_count(), 0);
    assert_eq!(app.status, "cleared 2 marks");
}

#[test]
/// 驗證 normal mode 的 `Ctrl-d / Ctrl-u` 會依照目前列表 viewport 高度做半頁移動。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_d_and_ctrl_u_move_by_half_page() {
    let dir = tempdir().expect("tempdir");
    for index in 0..10 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_list_viewport_height(6);

    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("page down");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 3);
    assert_eq!(app.status, "half page down: 3");

    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("page up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(app.status, "half page up: 3");
}

#[test]
/// 驗證 normal mode 的 `Ctrl-f / Ctrl-b` 會依照目前列表 viewport 高度做整頁移動。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_ctrl_f_and_ctrl_b_move_by_full_page() {
    let dir = tempdir().expect("tempdir");
    for index in 0..12 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_list_viewport_height(5);

    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
        .expect("full page down");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
    assert_eq!(app.status, "page down: 5");

    app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL))
        .expect("full page up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(app.status, "page up: 5");
}

#[test]
/// 驗證 visual selection 中的 `Ctrl-d / Ctrl-u` 也會用半頁步長移動，並同步更新範圍。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn app_visual_selection_ctrl_d_and_ctrl_u_follow_half_page() {
    let dir = tempdir().expect("tempdir");
    for index in 0..10 {
        fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.panes
        .get_mut(&1)
        .expect("pane")
        .set_list_viewport_height(6);

    app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
        .expect("visual");
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
        .expect("visual page down");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 3);
    assert_eq!(
        app.visual_selection,
        Some(VisualSelectionState {
            pane_id: 1,
            anchor: 0,
            current: 3,
        })
    );

    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
        .expect("visual page up");
    assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
    assert_eq!(
        app.visual_selection,
        Some(VisualSelectionState {
            pane_id: 1,
            anchor: 0,
            current: 0,
        })
    );
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

#[test]
/// 驗證 regex rename 使用的 command UI 可以切到 Normal 模式移動游標，再回到 Insert 修正中間文字。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn command_input_supports_vim_normal_and_insert_modes() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_prefilled_command("rename-regex foo baz");
    app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("enter normal mode");
    assert!(app.command_mode);
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert_eq!(app.rename_cursor_mode(), Some(RenameMode::Normal));

    for _ in 0..2 {
        app.handle_command_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("move left");
    }
    app.handle_command_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("enter insert mode");
    app.handle_command_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("insert correction");

    assert_eq!(app.command_buffer, "rename-regex foo Xbaz");
    assert_eq!(app.text_input_mode, RenameMode::Insert);
}

#[test]
/// 驗證一般 filter 第一次 Esc 只切換模式，Normal 模式第二次 Esc 才鎖定並離開輸入框。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn filter_input_uses_two_stage_escape_and_supports_cursor_editing() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("aXbc.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_filter_input(FilterMode::Normal);
    for character in ['a', 'b', 'c'] {
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
            .expect("type filter");
    }
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("normal mode");
    assert!(app.filter.as_ref().is_some_and(|filter| filter.editing));

    app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("move left");
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("insert mode");
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
        .expect("insert middle");
    assert_eq!(app.filter.as_ref().expect("filter").buffer, "aXbc");

    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("normal mode again");
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("leave editor");
    assert!(!app.filter.as_ref().expect("filter").editing);
}

#[test]
/// 驗證 help、trash、task、bookmark 與 zoxide 共用的面板搜尋器會攔截 Normal 模式按鍵，不會誤關面板。
/// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
fn panel_search_uses_shared_vim_editor_before_panel_actions() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open help filter");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type filter");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("normal mode");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("move cursor instead of closing panel");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            search: PanelSearchState { editing: true, .. },
            ..
        })
    ));
    assert_eq!(app.text_input_mode, RenameMode::Normal);

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close panel search");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            search: PanelSearchState { editing: false, .. },
            ..
        })
    ));
}

#[test]
/// 驗證空白 command UI 在 Insert 模式按第一次 Esc 就會直接關閉。
/// 保護目的：空輸入沒有文字需要進入 Vim Normal 模式修正，不應要求使用者連按兩次。
fn empty_command_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_prefilled_command("");
    app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty command input");

    assert!(!app.command_mode);
    assert!(app.command_buffer.is_empty());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證剛開啟且尚未輸入內容的 filter，第一次 Esc 會完整清除 filter 狀態。
/// 保護目的：避免輸入框雖消失，內部卻殘留一個不可見的空 filter，造成後續 Esc 流程混亂。
fn empty_filter_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_filter_input(FilterMode::Normal);
    app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty filter input");

    assert!(app.filter.is_none());
    assert!(!app.panes.get(&1).expect("pane").has_active_filter());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證空白 Preview Search 在第一次 Esc 就會關閉，並清除 pane 上的搜尋條件。
/// 保護目的：所有共用文字輸入 UI 都必須遵守相同的空輸入快速離開規則。
fn empty_preview_search_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "alpha").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_preview_focus();
    app.open_preview_search_input();
    app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty preview search");

    assert!(app.preview_search.is_none());
    assert_eq!(app.status, "preview mode");
}

#[test]
/// 驗證建立檔案的 inline 輸入框尚未輸入名稱時，第一次 Esc 就會取消建立。
/// 保護目的：rename/create 使用獨立編輯流程，也必須與共用輸入器保持一致。
fn empty_create_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.start_create_entry();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel empty create input");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "create cancelled");
}

#[test]
/// 驗證空白 rename 輸入框在 Insert 模式按第一次 Esc 就會取消改名。
/// 保護目的：rename 使用獨立編輯流程，清空原檔名後也不可殘留在無意義的 Normal 模式。
fn empty_rename_input_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.pending_action = Some(PendingAction::Rename {
        pane_id: 1,
        original_name: String::from("alpha.txt"),
        buffer: String::new(),
        cursor: 0,
        mode: RenameMode::Insert,
    });

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("cancel empty rename input");

    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "rename cancelled: alpha.txt");
}

#[test]
/// 驗證空白 list find 與 global search 都能用第一次 Esc 直接回到一般列表。
/// 保護目的：兩種搜尋使用不同外層狀態機，但都必須遵守共用輸入器的快速離開規則。
fn empty_list_and_global_search_close_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_list_find_input();
    app.handle_list_find_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty list find");
    assert!(app.list_find.is_none());

    app.open_global_search().expect("open global search");
    app.handle_global_search_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty global search");
    assert!(app.global_search.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證列表面板內剛開啟的空白搜尋框，第一次 Esc 就會收起輸入框。
/// 保護目的：Help、Trash、Task、Bookmark、Zoxide 共用此流程，修正一次即可保持一致。
fn empty_panel_search_closes_on_first_escape() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_help_panel();
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open panel search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close empty panel search");

    assert!(matches!(
        app.pending_action,
        Some(PendingAction::HelpPanel {
            search: PanelSearchState {
                editing: false,
                ref buffer,
            },
            ..
        }) if buffer.is_empty()
    ));
}

#[test]
/// 驗證在目錄背景載入尚未完成時按 h 離開，會即時取消舊工作且不把空列表寫入快取。
/// 保護目的：確保使用者在大型目錄快速切出時不卡死，且不會把未完成的空清單當作快取。
fn navigating_away_during_load_cancels_load_and_guards_cache() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    fs::create_dir_all(&child).expect("child dir");
    fs::write(child.join("payload.txt"), "data").expect("payload");

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    // 進入 child 目錄，此時啟動背景載入
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter child");
    assert_eq!(app.panes[&1].cwd, child);
    assert!(app.directory_load_jobs.contains_key(&1));

    // 尚未 poll 完成前立刻按 h 離開回到 parent
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        .expect("return to parent");
    assert_eq!(app.panes[&1].cwd, dir.path());

    // 快取中不能存入 child 的空清單
    assert_ne!(
        app.directory_entry_cache.get(&child),
        Some(&Vec::new()),
        "載入中離開目錄不得將空陣列存入快取"
    );
}

#[test]
/// 驗證建立新檔案與刪除檔案後，快取會同步更新，重新進入目錄不會讀到陳舊快取。
/// 保護目的：防止使用者在目錄操作後離開再進入時，出現已刪除檔案回魂或新檔案消失的現象。
fn file_creation_and_deletion_updates_cache_synchronously() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 建立檔案
    app.create_entry_from_command("new_item.txt")
        .expect("create file");

    let cached = app
        .directory_entry_cache
        .get(dir.path())
        .expect("must have cache");
    assert!(
        cached.iter().any(|entry| entry.name == "new_item.txt"),
        "快取必須包含剛建立的檔案"
    );

    // 刪除檔案
    app.start_delete_confirmation(true);
    app.confirm_delete(1, "new_item.txt", true)
        .expect("delete file");
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let cached_after = app
        .directory_entry_cache
        .get(dir.path())
        .expect("must have cache after delete");
    assert!(
        !cached_after
            .iter()
            .any(|entry| entry.name == "new_item.txt"),
        "快取中不能殘留已刪除的檔案"
    );
}

#[test]
/// 驗證檔案系統 watcher 觸發重新整理時，快取會同步更新為磁碟最新狀態。
/// 保護目的：確保外部編輯器或 Git 操作產生變更後，快取與畫面保持一致。
fn watcher_reload_synchronizes_cache() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    fs::write(dir.path().join("external.txt"), "external content").expect("external write");
    let mut set = BTreeSet::new();
    set.insert(dir.path().to_path_buf());

    app.reload_watched_directories(&set)
        .expect("reload watched");

    let cached = app
        .directory_entry_cache
        .get(dir.path())
        .expect("must have cache");
    assert!(
        cached.iter().any(|entry| entry.name == "external.txt"),
        "watcher 刷新後快取必須包含外部新增的檔案"
    );
}

#[test]
/// 驗證大型目錄以串流載入時，首批清單到達後游標即可立刻以 j/k 移動，不需等待全量掃描結束。
/// 保護目的：確保使用者在幾萬個檔案的目錄中，畫面在毫秒級反應，游標絕不被背景 I/O 凍結。
fn streaming_directory_load_allows_cursor_movement_before_completion() {
    let dir = tempdir().expect("tempdir");
    let child = dir.path().join("child");
    fs::create_dir_all(&child).expect("child");
    for i in 0..300 {
        fs::write(child.join(format!("file_{i:03}.txt")), b"test").expect("write");
    }

    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter child");

    // 等待首批快速 chunk 到達
    for _ in 0..100 {
        app.poll_background_tasks();
        if !app.panes[&1].entries.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    // 首批項目已在畫面上，游標在第 0 筆
    assert!(!app.panes[&1].entries.is_empty());
    assert_eq!(app.panes[&1].selected, 0);

    // 立刻按下 j 移動游標，游標必須成功移動到第 1 筆
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("move down");
    assert_eq!(app.panes[&1].selected, 1, "游標必須在載入中立即響應移動");

    // 等待背景全量載入結束
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.directory_load_jobs.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(app.panes[&1].entries.len(), 300);
    assert_eq!(
        app.panes[&1].selected, 1,
        "全量完成後仍必須保留使用者剛剛移動的游標位置"
    );
}

#[test]
/// 驗證大型目錄的 task badge 查詢只接觸 viewport，不會走訪完整檔案列表。
///
/// 保護目的：舊版 render 會對每個項目執行 `canonicalize()`；實際 `deps` 超過六萬筆
/// 時，每次 j/k 都被同步檔案 I/O 阻塞約半秒。此測試建立 200 筆並固定 viewport
/// 為 18 列，確保未來重構不能再次把完整列表送進 badge 查詢。
fn task_badge_paths_are_limited_to_visible_viewport() {
    let dir = tempdir().expect("tempdir");
    for index in 0..200 {
        fs::write(dir.path().join(format!("file_{index:03}.txt")), b"x").expect("write");
    }
    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.selected = 150;

    let paths = visible_job_badge_paths(&pane, 18);

    assert_eq!(paths.len(), 18);
    assert!(paths.iter().all(|path| path.starts_with(dir.path())));
}

#[test]
/// 驗證當背景有檔案傳輸或寫入進行時，使用者不可進入該正在寫入的目錄。
fn cannot_enter_directory_while_transfer_is_in_progress() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    let child = app.panes[&1].cwd.join("copying_target");
    fs::create_dir(&child).expect("create child");
    app.panes.get_mut(&1).unwrap().reload().expect("reload");

    // 模擬一個正在進行中的背景傳輸工作綁定到 child
    let task_id = app.push_task(
        1,
        "paste",
        "copy files".to_string(),
        "dest".to_string(),
        vec![child.display().to_string()],
        Some(dir.path().display().to_string()),
    );
    app.active_file_job_busy_paths
        .insert(task_id, vec![child.clone()]);
    app.update_task_progress(task_id, 45, 100);

    // 選中該目錄並嘗試進入
    app.panes.get_mut(&1).unwrap().select_path(&child);
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("press l");

    // 工作目錄不可被切換，且狀態列必須提示傳輸進行中
    assert_eq!(app.panes[&1].cwd, child.parent().unwrap());
    assert!(app.status.contains("cannot enter 'copying_target/'"));
    assert!(app.status.contains("transfer in progress"));
    assert!(app.status.contains("45%"));
}

#[test]
/// 驗證永久刪除大型目錄時使用背景 worker 執行，主執行緒不卡死且完成後正確移除。
fn background_delete_removes_directory_and_reports_done() {
    let dir = tempdir().expect("tempdir");
    let to_delete = dir.path().join("large_dir");
    fs::create_dir_all(to_delete.join("nested")).expect("create nested");
    fs::write(to_delete.join("nested/a.bin"), "payload").expect("write payload");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.panes.get_mut(&1).unwrap().select_path(&to_delete);
    app.start_delete_confirmation(true);
    app.confirm_delete(1, "large_dir", true)
        .expect("confirm delete");

    assert!(
        !app.file_job_receivers.is_empty(),
        "刪除工作必須進入背景佇列"
    );
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(!to_delete.exists(), "背景刪除完成後目錄必須已從磁碟移除");
    assert!(app.status.contains("deleted permanently"));
    let task = app.task_log.last().expect("background delete task");
    assert_eq!(task.title, "delete 1 item(s)");
    assert_eq!(task.source_locations, vec![to_delete.display().to_string()]);
    assert_eq!(task.destination_location, None);
}

#[test]
/// 驗證當子目錄或檔案在進行背景傳輸時，其父目錄依然可以正常進入，且進入後該子項目會顯示進度標籤。
fn parent_directory_is_enterable_while_child_is_being_transferred() {
    let dir = tempdir().expect("tempdir");
    let parent = dir.path().join("AB_Demo");
    let child = parent.join("terminal-file-manager");
    fs::create_dir_all(&child).expect("create child dir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 模擬背景傳輸工作鎖定子項目 child
    let task_id = app.push_task(
        1,
        "paste",
        "copy 1 item(s)".to_string(),
        "dest".to_string(),
        vec![child.display().to_string()],
        Some(parent.display().to_string()),
    );
    app.active_file_job_busy_paths
        .insert(task_id, vec![child.clone()]);
    app.update_task_progress(task_id, 99, 100);

    // 父目錄 parent 必須不受影響，不可被視為 busy
    assert!(
        app.active_file_job_for_path(&parent).is_none(),
        "父目錄不可被子項目的傳輸鎖定"
    );
    // 子項目 child 必須被鎖定並能提供 progress badge
    assert!(app.active_file_job_for_path(&child).is_some());
    assert_eq!(
        app.active_job_badge_for_path(&child).as_deref(),
        Some("[copying 99%]")
    );

    // 嘗試進入父目錄 parent
    app.panes.get_mut(&1).unwrap().select_path(&parent);
    app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
        .expect("enter parent dir");

    // 必須成功進入父目錄
    assert_eq!(
        app.panes[&1].cwd.canonicalize().unwrap(),
        parent.canonicalize().unwrap()
    );
}

#[test]
/// 驗證背景刪除任務在執行期間與完成後，Task 紀錄會包含實際刪除的 byte 進度資訊，而非未知的 `-`。
fn background_delete_reports_byte_progress_in_task_record() {
    let dir = tempdir().expect("tempdir");
    let to_delete = dir.path().join("data_folder");
    fs::create_dir_all(&to_delete).expect("create dir");
    fs::write(to_delete.join("file1.dat"), vec![0u8; 1024 * 10]).expect("write file1");
    fs::write(to_delete.join("file2.dat"), vec![0u8; 1024 * 20]).expect("write file2");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.panes.get_mut(&1).unwrap().select_path(&to_delete);
    app.start_delete_confirmation(true);
    app.confirm_delete(1, "data_folder", true)
        .expect("confirm delete");

    // 輪詢等待背景刪除工作完成
    for _ in 0..100 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let task = app.task_log.last().expect("task record");
    assert_eq!(task.state, TaskState::Done);
    assert!(
        task.completed_bytes.unwrap_or(0) >= 1024 * 30,
        "必須記錄實際刪除的 byte 數: {:?}",
        task.completed_bytes
    );
    assert_eq!(task.completed_bytes, task.total_bytes);
    let progress_label = task_progress_label(task);
    assert_ne!(progress_label, "-", "進度標籤不可為未知的 `-`");
    assert!(
        progress_label.contains("30K")
            || progress_label.contains("K")
            || progress_label.contains("M")
    );
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

#[test]
fn diff_command_requires_two_panels() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
    // 只有 1 個 panel 時執行 :diff
    app.execute_command("diff").expect("diff cmd");
    assert!(app.status.contains("diff requires at least 2 open panels"));
    assert!(app.pending_action.is_none());
}

#[test]
fn diff_command_opens_and_navigates_matrix() {
    let dir1 = tempdir().expect("dir1");
    let dir2 = tempdir().expect("dir2");
    let dir3 = tempdir().expect("dir3");

    fs::write(dir1.path().join("a.txt"), b"aaa").expect("write a");
    fs::write(dir2.path().join("a.txt"), b"aaa").expect("write a");
    fs::write(dir3.path().join("a.txt"), b"different").expect("write a");

    fs::write(dir1.path().join("only1.txt"), b"111").expect("write 1");
    fs::write(dir2.path().join("only2.txt"), b"222").expect("write 2");

    let mut app = App::new(dir1.path().to_path_buf(), default_loaded_config()).expect("app");
    // 分割出 panel 2 與 panel 3
    app.split_current(SplitDirection::Vertical)
        .expect("split 1");
    app.change_directory_from_command(&dir2.path().to_string_lossy())
        .expect("goto dir2");

    app.split_current(SplitDirection::Vertical)
        .expect("split 2");
    app.change_directory_from_command(&dir3.path().to_string_lossy())
        .expect("goto dir3");

    // 執行 :diff 比對全部 3 個 Panel
    app.execute_command("diff").expect("execute diff");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::DiffMatrix(_))
    ));

    // 輪詢等待背景 diff 完成
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.poll_background_tasks();
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            if !state.loading {
                break;
            }
        }
    }

    // 測試按鍵導航
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("down");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert_eq!(state.selected_index, 1);
    }

    // 測試篩選切換 (f)
    app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("filter cycle");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert_eq!(
            state.filter_mode,
            crate::file_manager::diff::DiffFilterMode::DiffOnly
        );
    }

    // 測試 gitignore 切換 (i)
    app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
        .expect("toggle gitignore");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert!(!state.git_ignore);
    }
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.poll_background_tasks();
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            if !state.loading {
                break;
            }
        }
    }

    // 測試隱藏檔切換 (.)
    app.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
        .expect("toggle hidden");
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert!(!state.include_hidden);
    }
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.poll_background_tasks();
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            if !state.loading {
                break;
            }
        }
    }

    // 測試搜尋 (/)
    app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
        .expect("search start");
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        .expect("type o");
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("type n");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("confirm search");

    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert!(!state.search_active);
        assert_eq!(state.search_query, "on");
        assert_eq!(state.filtered_indices.len(), 2); // only1.txt, only2.txt
    }

    // 測試退出比對 (q)
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("quit diff");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "diff matrix closed");

    // 測試快速指令別名 :d 1 2
    app.execute_command("d 1 2").expect("execute d 1 2");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::DiffMatrix(_))
    ));
    if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
        assert_eq!(state.panel_ids, vec![1, 2]);
    }

    // 退出
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert!(app.pending_action.is_none());

    // 測試快捷鍵 wd (WindowPicker -> d)
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("w");
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d");
    assert!(matches!(
        app.pending_action,
        Some(PendingAction::DiffMatrix(_))
    ));

    // 退出
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert!(app.pending_action.is_none());

    // 測試快捷鍵 wD (WindowPicker -> D: prefilled :diff )
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
        .expect("w");
    app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("D");
    assert!(app.command_mode);
    assert_eq!(app.command_buffer, "diff ");
}

#[test]
/// 驗證任務面板可以使用 v 進行 visual 選取並使用 d 批次刪除任務。
fn task_panel_supports_visual_selection_and_batch_delete_with_v_and_d() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let t1 = app.push_task(
        1,
        "copy",
        "copy a.txt".into(),
        "running".into(),
        vec!["/a.txt".into()],
        None,
    );
    let t2 = app.push_task(
        1,
        "move",
        "move b.txt".into(),
        "running".into(),
        vec!["/b.txt".into()],
        None,
    );
    let t3 = app.push_task(
        1,
        "delete",
        "delete c.txt".into(),
        "running".into(),
        vec!["/c.txt".into()],
        None,
    );

    // 打開任務面板
    app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
        .expect("open task panel");

    // 目前有 3 筆任務，最新排在最上面 (t3, t2, t1)
    assert_eq!(app.tasks_for_pane(1).len(), 3);

    // 按下 v 開啟 visual 選取模式
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("v start");

    // 向下移動一格 (j)
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("j down");

    // 再次按下 v 提交 visual 選取 (選取了 t3 與 t2)
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("v commit");

    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert_eq!(marked_ids.len(), 2);
            assert!(marked_ids.contains(&t3));
            assert!(marked_ids.contains(&t2));
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 按下 d 批次刪除已選取的任務
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d delete marked");

    assert_eq!(app.tasks_for_pane(1).len(), 1);
    assert_eq!(app.tasks_for_pane(1)[0].id, t1);
    assert_eq!(app.status, "tasks: deleted 2 tasks");
}

#[test]
/// 驗證任務面板可以使用 Space 標記個別任務、使用 a 全選、使用 d/D 刪除與清空。
fn task_panel_supports_space_mark_all_and_clear_all_with_shifted_d() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    let t1 = app.push_task(
        1,
        "copy",
        "copy a.txt".into(),
        "running".into(),
        vec!["/a.txt".into()],
        None,
    );
    let t2 = app.push_task(
        1,
        "move",
        "move b.txt".into(),
        "running".into(),
        vec!["/b.txt".into()],
        None,
    );

    // 打開任務面板
    app.open_task_panel();

    // 按 Space 標記第一筆任務 (t2)
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
        .expect("space mark");

    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert_eq!(marked_ids, &vec![t2]);
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 按 d 刪除被標記的 t2
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d delete");
    assert_eq!(app.tasks_for_pane(1).len(), 1);
    assert_eq!(app.tasks_for_pane(1)[0].id, t1);

    // 按 a 標記全部剩餘任務
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("a mark all");
    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert_eq!(marked_ids, &vec![t1]);
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 按 a 取消全部標記
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("a clear all marks");
    match app.pending_action.as_ref() {
        Some(PendingAction::TaskPanel { marked_ids, .. }) => {
            assert!(marked_ids.is_empty());
        }
        other => panic!("unexpected action: {other:?}"),
    }

    // 在沒有標記狀態下按 d，直接刪除游標所在任務
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
        .expect("d delete single");
    assert_eq!(app.tasks_for_pane(1).len(), 0);

    // 重新新增任務並測試 Shift+D (清空所有任務)
    app.push_task(1, "copy", "task 1".into(), "running".into(), vec![], None);
    app.push_task(1, "copy", "task 2".into(), "running".into(), vec![], None);
    assert_eq!(app.tasks_for_pane(1).len(), 2);

    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
        .expect("D clear all");
    assert_eq!(app.tasks_for_pane(1).len(), 0);
    assert_eq!(app.status, "tasks: cleared 2 tasks");
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

#[test]
/// 驗證按下 ? 鍵在 Normal 模式與 Task 面板能分別開啟對應情境的 Cheatsheet，且 Esc 能無縫返回。
fn cheatsheet_opens_context_specific_help_and_restores_state() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. 在 Normal 模式按 ? 鍵
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
        .expect("press ? in normal mode");
    match &app.pending_action {
        Some(PendingAction::HelpPanel { custom_title, custom_entries, .. }) => {
            assert!(custom_title.as_ref().unwrap().contains("Normal Mode"));
            let entries = custom_entries.as_ref().unwrap();
            assert!(entries.iter().any(|e| e.line.shortcut.contains("j / k")));
            assert!(entries.iter().any(|e| e.line.command == "rename"));
        }
        other => panic!("expected Cheatsheet HelpPanel, got {other:?}"),
    }

    // 按 Esc 退出 Cheatsheet 返回 Normal 模式
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("press Esc to exit cheatsheet");
    assert!(app.pending_action.is_none());

    // 2. 開啟 TaskPanel 後按 ? 鍵
    app.open_task_panel();
    assert!(matches!(app.pending_action, Some(PendingAction::TaskPanel { .. })));

    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
        .expect("press ? in task panel");
    match &app.pending_action {
        Some(PendingAction::HelpPanel { custom_title, custom_entries, .. }) => {
            assert!(custom_title.as_ref().unwrap().contains("Task Panel"));
            let entries = custom_entries.as_ref().unwrap();
            assert!(entries.iter().any(|e| e.line.shortcut.contains("d")));
            assert!(entries.iter().any(|e| e.line.shortcut.contains("x / c")));
            assert!(entries.iter().any(|e| e.line.shortcut.contains("v / V")));
        }
        other => panic!("expected Cheatsheet HelpPanel, got {other:?}"),
    }

    // 按 ? 鍵再次關閉 Cheatsheet，必須精準返回 TaskPanel
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
        .expect("press ? to exit cheatsheet");
    assert!(matches!(app.pending_action, Some(PendingAction::TaskPanel { .. })));

    // 3. 驗證 :cheatsheet 與 :cheat 指令
    app.pending_action = None;
    app.execute_command("cheatsheet").expect("exec :cheatsheet");
    assert!(matches!(app.pending_action, Some(PendingAction::HelpPanel { custom_title: Some(_), .. })));
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).unwrap();

    app.execute_command("cheat").expect("exec :cheat");
    assert!(matches!(app.pending_action, Some(PendingAction::HelpPanel { custom_title: Some(_), .. })));
}

#[test]
/// 驗證 Cheatsheet 支援以 f 鍵開啟搜尋並在情境清單內進行模糊過濾。
fn cheatsheet_search_filters_within_context_entries() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_task_panel();
    app.open_cheatsheet_from_current();

    // 在 Task Cheatsheet 內按 f 搜尋 "cancel"
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
        .expect("open search");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
        .expect("type c");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type a");
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
        .expect("type n");

    match &app.pending_action {
        Some(PendingAction::HelpPanel { search, custom_entries, .. }) => {
            assert_eq!(search.buffer, "can");
            let filtered = filter_custom_help_entries(custom_entries.as_ref().unwrap(), &search.buffer);
            assert!(filtered.len() >= 1);
            assert!(filtered.iter().any(|e| e.line.description.contains("取消")));
        }
        other => panic!("unexpected action: {other:?}"),
    }
}

#[test]
/// 驗證在全域搜尋（s/S）面板中按下 ? 鍵只會顯示搜尋專屬快捷鍵，不會出現 create(a) 或 delete(d)。
fn cheatsheet_in_global_search_shows_only_search_navigation_keys() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 模擬開啟 global search (s)
    app.open_global_search().expect("open global search");
    assert!(app.global_search.is_some());

    // 在 global search 狀態下按 ? 鍵
    app.open_cheatsheet_from_current();

    match &app.pending_action {
        Some(PendingAction::HelpPanel { custom_title, custom_entries, .. }) => {
            let title = custom_title.as_ref().unwrap();
            assert!(title.contains("Global Search"), "title should be Global Search, got: {title}");
            let entries = custom_entries.as_ref().unwrap();
            let commands: Vec<&str> = entries.iter().map(|e| e.line.command.as_str()).collect();

            // 確保包含搜尋導覽與預覽快捷鍵
            assert!(commands.contains(&"move"));
            assert!(commands.contains(&"open"));
            assert!(commands.contains(&"filter"));
            assert!(commands.contains(&"re-edit"));
            assert!(commands.contains(&"exit"));

            // 確保「絕對不包含」無法在搜尋面板執行的 normal 模式指令
            assert!(!commands.contains(&"create"), "cheatsheet should NOT contain create");
            assert!(!commands.contains(&"trash"), "cheatsheet should NOT contain trash");
            assert!(!commands.contains(&"delete!"), "cheatsheet should NOT contain delete!");
            assert!(!commands.contains(&"rename"), "cheatsheet should NOT contain rename");
            assert!(!commands.contains(&"paste"), "cheatsheet should NOT contain paste");
            assert!(!commands.contains(&"undo"), "cheatsheet should NOT contain undo");
        }
        other => panic!("expected Cheatsheet HelpPanel, got {other:?}"),
    }

    // 按 Esc 退出 Cheatsheet，必須精準回復 global_search
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("press Esc to exit cheatsheet");
    assert!(app.global_search.is_some(), "global_search state should be restored");
}

#[test]
/// 驗證所有 ContextHelpKind 都擁有專屬、非空的 Cheatsheet 定義。
fn cheatsheet_covers_all_context_kinds() {
    let all_kinds = [
        ContextHelpKind::Normal,
        ContextHelpKind::GlobalSearch,
        ContextHelpKind::ListFind,
        ContextHelpKind::TaskPanel,
        ContextHelpKind::TrashPanel,
        ContextHelpKind::DiffMatrix,
        ContextHelpKind::VisualSelection,
        ContextHelpKind::BookmarkPicker,
        ContextHelpKind::BookmarkList,
        ContextHelpKind::ZoxideList,
        ContextHelpKind::WindowPicker,
        ContextHelpKind::SortPicker,
        ContextHelpKind::GoPicker,
        ContextHelpKind::LineModePicker,
        ContextHelpKind::ThemePicker,
        ContextHelpKind::CommandMode,
        ContextHelpKind::Filter,
        ContextHelpKind::Preview,
        ContextHelpKind::ToolPanel,
        ContextHelpKind::RegexRename,
        ContextHelpKind::Rename,
        ContextHelpKind::CreateEntry,
        ContextHelpKind::ConfirmAction,
        ContextHelpKind::CopyPicker,
        ContextHelpKind::OpenPicker,
    ];

    for kind in all_kinds {
        let (title, entries) = context_cheatsheet_entries(kind);
        assert!(!title.is_empty(), "title for {kind:?} must not be empty");
        assert!(!entries.is_empty(), "entries for {kind:?} must not be empty");
        for entry in &entries {
            assert!(!entry.line.command.is_empty(), "command in {kind:?} must not be empty");
            assert!(!entry.line.shortcut.is_empty(), "shortcut in {kind:?} must not be empty");
            assert!(!entry.line.description.is_empty(), "description in {kind:?} must not be empty");
        }
    }
}

#[test]
/// 驗證在 Rename 與 CreateEntry 的 Normal 模式下，按 `q` 可以直接取消並離開。
fn q_cancels_rename_and_create_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. Rename: 輸入文字後按 Esc 進入 Normal 模式，再按 q 取消
    app.pending_action = Some(PendingAction::Rename {
        pane_id: 1,
        original_name: String::from("alpha.txt"),
        buffer: String::from("new_alpha.txt"),
        cursor: 4,
        mode: RenameMode::Normal,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels rename in normal mode");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "rename cancelled: alpha.txt");

    // 2. CreateEntry: 輸入文字後按 Esc 進入 Normal 模式，再按 q 取消
    app.pending_action = Some(PendingAction::CreateEntry {
        pane_id: 1,
        buffer: String::from("some_dir/"),
        cursor: 4,
        mode: RenameMode::Normal,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels create in normal mode");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "create cancelled");
}

#[test]
/// 驗證在全域搜尋輸入框中輸入文字後，按 Esc 進入 Normal 模式，再按 q 可以直接退出全域搜尋。
fn q_cancels_global_search_input_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_global_search().expect("open global search");
    assert!(app.global_search.is_some());
    assert!(app.global_search.as_ref().unwrap().editing);

    // 輸入 'k' 與 'j'
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
        .expect("type k");
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
        .expect("type j");
    assert_eq!(app.global_search.as_ref().unwrap().buffer, "kj");
    assert_eq!(app.text_input_mode, RenameMode::Insert);

    // 按 Esc 進入 Normal 模式
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("press esc");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert!(app.global_search.is_some());
    assert!(app.global_search.as_ref().unwrap().editing);

    // 在 Normal 模式下按 q 退出全域搜尋
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("press q");
    assert!(app.global_search.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證在全域搜尋結果瀏覽清單中，按 `q` 可以直接關閉全域搜尋回到一般模式。
fn q_exits_global_search_results_and_preview() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. 搜尋結果瀏覽清單中按 q
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        task_id: None,
        mode: SearchMode::Path,
        buffer: String::from("test"),
        results: vec![GlobalSearchEntry {
            path: dir.path().join("test.txt"),
            relative_path: String::from("test.txt"),
            is_dir: false,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        }],
        selected: 0,
        editing: false,
        searched: true,
        loading: false,
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
    });

    app.handle_global_search_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels global search");
    assert!(app.global_search.is_none());
    assert_eq!(app.status, "normal mode");

    // 2. 在內容搜尋預覽模式下按 q
    app.global_search = Some(GlobalSearchState {
        pane_id: 1,
        root_dir: dir.path().to_path_buf(),
        task_id: None,
        mode: SearchMode::Content,
        buffer: String::from("demo"),
        results: vec![GlobalSearchEntry {
            path: dir.path().join("test.txt"),
            relative_path: String::from("test.txt"),
            is_dir: false,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        }],
        selected: 0,
        editing: false,
        searched: true,
        loading: false,
        filter: PanelSearchState::default(),
        preview_scroll: None,
        preview_current_match: None,
    });
    if let Some(pane) = app.panes.get_mut(&1) {
        pane.set_preview_active(true);
    }

    app.handle_global_search_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q exits preview in content search");
    assert!(app.global_search.is_some());
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
}

#[test]
/// 驗證檔案預覽模式 (Tab preview) 按 `q` 或 `h` 關閉預覽回到檔案列表。
fn q_exits_file_preview_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "hello world").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_preview_focus();
    assert!(app.panes.get(&1).expect("pane").is_preview_active());

    app.handle_preview_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q exits preview");
    assert!(!app.panes.get(&1).expect("pane").is_preview_active());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證視覺選取模式 (Visual Selection) 按 `q` 可以直接取消。
fn q_cancels_visual_selection() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "hello").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
        .expect("start visual selection");
    assert!(app.visual_selection.is_some());

    app.handle_visual_selection_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels visual selection");
    assert!(app.visual_selection.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證確認刪除與確認覆蓋對話框按 `q` 可以直接取消。
fn q_cancels_confirmation_dialogs() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. ConfirmDelete
    app.pending_action = Some(PendingAction::ConfirmDelete {
        pane_id: 1,
        target_name: String::from("important.txt"),
        permanent: true,
        warning_message: None,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels delete");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "delete cancelled: important.txt");

    // 2. ConfirmPasteOverwrite
    app.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
        pane_id: 1,
        target_name: String::from("target.txt"),
        entry_count: 1,
        operation: ClipboardOperation::Copy,
    });
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q cancels overwrite");
    assert!(app.pending_action.is_none());
    assert_eq!(app.status, "paste cancelled: target.txt");
}

#[test]
/// 驗證 CommandMode 在 Normal 模式下按 `q` 可以直接退出。
fn q_cancels_command_mode_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_prefilled_command("rename new_name");
    assert!(app.command_mode);

    // 按 Esc 切換到 Normal 模式
    app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc enters normal mode");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert!(app.command_mode);

    // Normal 模式下按 q 關閉 command mode
    app.handle_command_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes command mode");
    assert!(!app.command_mode);
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證 ListFind 在 Normal 模式下按 `q` 可以直接取消。
fn q_cancels_list_find_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    app.open_list_find_input();
    assert!(app.list_find.is_some());

    // 輸入 'a'
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
        .expect("type a");
    assert_eq!(app.list_find.as_ref().unwrap().buffer, "a");

    // 按 Esc 進入 Normal 模式
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc enters normal mode");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    assert!(app.list_find.is_some());

    // Normal 模式下按 q 關閉 list find
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes list find");
    assert!(app.list_find.is_none());
    assert_eq!(app.status, "normal mode");
}

#[test]
/// 驗證各面板內部搜尋框（TaskPanel, TrashPanel, BookmarkList, ZoxideList）在 Normal 模式下按 `q` 關閉搜尋框。
fn q_cancels_panel_search_in_normal_mode() {
    let dir = tempdir().expect("tempdir");
    let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

    // 1. TaskPanel 內部搜尋
    app.text_input_mode = RenameMode::Insert;
    app.pending_action = Some(PendingAction::TaskPanel {
        pane_id: 1,
        selected: 0,
        search: PanelSearchState {
            buffer: String::from("copy"),
            editing: true,
        },
        marked_ids: Vec::new(),
        visual_anchor: None,
    });
    // Esc -> Normal
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    // q -> 關閉 search.editing
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes task panel search");
    if let Some(PendingAction::TaskPanel { search, .. }) = &app.pending_action {
        assert!(!search.editing);
    } else {
        panic!("expected TaskPanel");
    }

    // 2. TrashPanel 內部搜尋
    app.text_input_mode = RenameMode::Insert;
    app.pending_action = Some(PendingAction::TrashPanel {
        pane_id: 1,
        selected: 0,
        search: PanelSearchState {
            buffer: String::from("del"),
            editing: true,
        },
        marked_ids: Vec::new(),
        visual_anchor: None,
    });
    // Esc -> Normal
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("esc");
    assert_eq!(app.text_input_mode, RenameMode::Normal);
    // q -> 關閉 search.editing
    app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("q closes trash panel search");
    if let Some(PendingAction::TrashPanel { search, .. }) = &app.pending_action {
        assert!(!search.editing);
    } else {
        panic!("expected TrashPanel");
    }
}
