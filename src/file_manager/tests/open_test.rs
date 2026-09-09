use std::path::PathBuf;

use crate::config::{
    ActionLaunchMode, ActionTargetScope, CustomOpenActionConfig, TerminalPluginConfig,
};

use super::{
    LaunchMode, OpenAction, OpenPickerAction, OpenTarget, build_custom_launch_spec_for_platform,
    build_launch_spec, build_terminal_launch_spec, is_text_like_path, open_picker_options,
    parse_command_line,
};
use crate::file_manager::platform::PlatformKind;

#[test]
/// 驗證常見文字、程式碼與設定檔會走 Editor，而二進位格式不會。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn text_like_detection_matches_common_extensions() {
    assert!(is_text_like_path(&PathBuf::from("notes.txt")));
    assert!(is_text_like_path(&PathBuf::from("Cargo.toml")));
    assert!(is_text_like_path(&PathBuf::from("README")));
    assert!(!is_text_like_path(&PathBuf::from("photo.jpg")));
    assert!(!is_text_like_path(&PathBuf::from("report.pdf")));
}

#[test]
/// 驗證檔案 Open picker 不顯示只適用於目錄的系統 Open 選項。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn file_picker_omits_system_open_entry() {
    let options = open_picker_options(&OpenTarget {
        path: PathBuf::from("notes.txt"),
        display_name: "notes.txt".to_string(),
        is_dir: false,
    });
    assert_eq!(options.len(), 3);
    assert_eq!(
        options[0].action,
        OpenPickerAction::Builtin(OpenAction::Editor)
    );
    assert_eq!(
        options[1].action,
        OpenPickerAction::Builtin(OpenAction::Vim)
    );
    assert_eq!(
        options[2].action,
        OpenPickerAction::Builtin(OpenAction::Reveal)
    );
}

#[test]
/// 驗證文字檔優先採用 `$EDITOR`，未設定時才退回平台預設程式。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn editor_for_text_file_uses_editor_when_available_or_system_open_otherwise() {
    let spec = build_launch_spec(
        &OpenTarget {
            path: PathBuf::from("notes.txt"),
            display_name: "notes.txt".to_string(),
            is_dir: false,
        },
        OpenAction::Editor,
    )
    .expect("spec");

    let expected = if std::env::var("EDITOR")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        LaunchMode::TerminalBlocking
    } else {
        LaunchMode::Detached
    };
    assert_eq!(spec.mode, expected);
}

#[test]
/// 驗證命令列 parser 能保留引號內空白與反斜線跳脫內容。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn parse_command_line_supports_quoted_program_and_args() {
    let command = parse_command_line("\"C:\\Program Files\\Neovim\\bin\\nvim.exe\" -u NONE")
        .expect("command");

    assert_eq!(command.program, "C:\\Program Files\\Neovim\\bin\\nvim.exe");
    assert_eq!(command.args, vec!["-u", "NONE"]);
}

#[test]
/// 驗證 plugin action 會選擇平台命令、展開 placeholder 並套用啟動模式。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn custom_open_action_uses_platform_specific_command_and_placeholders() {
    let action = CustomOpenActionConfig {
        name: "Xcode".to_string(),
        scope: ActionTargetScope::Directory,
        mode: ActionLaunchMode::Detached,
        command: None,
        mac_command: Some("open -a Xcode {path}".to_string()),
        windows_command: Some("code {path}".to_string()),
    };

    let spec = build_custom_launch_spec_for_platform(
        &OpenTarget {
            path: PathBuf::from("/tmp/My Project"),
            display_name: "My Project/".to_string(),
            is_dir: true,
        },
        &action,
        PlatformKind::MacOs,
    )
    .expect("spec");

    assert_eq!(spec.program, "sh");
    assert_eq!(spec.args[0], "-lc");
    assert!(spec.args[1].contains("open -a Xcode '/tmp/My Project'"));
    assert_eq!(spec.mode, LaunchMode::Detached);
}

#[test]
/// 驗證 build_terminal_launch_spec 能優先匹配自訂的 terminal plugin。
fn build_terminal_launch_spec_matches_terminal_plugin() {
    let plugin = TerminalPluginConfig {
        name: "custom-term".to_string(),
        match_env: vec!["CUSTOM_TERM_ENV_FOR_TEST".to_string()],
        match_process: Vec::new(),
        command: Some("custom-term {path}".to_string()),
        mac_command: None,
        windows_command: None,
    };

    // 未設置環境變數時不匹配
    let spec_without_env = build_terminal_launch_spec(
        &PathBuf::from("/tmp/foo"),
        None,
        std::slice::from_ref(&plugin),
    )
    .expect("spec");

    // 設置環境變數後匹配
    unsafe {
        std::env::set_var("CUSTOM_TERM_ENV_FOR_TEST", "1");
    }
    let spec_with_env =
        build_terminal_launch_spec(&PathBuf::from("/tmp/foo"), None, &[plugin]).expect("spec");
    unsafe {
        std::env::remove_var("CUSTOM_TERM_ENV_FOR_TEST");
    }

    assert_ne!(spec_without_env.program, spec_with_env.program);
}
