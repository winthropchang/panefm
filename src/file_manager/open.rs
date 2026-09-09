//! 檔案/目錄的開啟選項、外部命令規格與 plugins.toml action 展開。
//!
//! 本模組只產生 `LaunchSpec`，不直接切換 raw mode 或等待 child process；真正啟動
//! 由事件迴圈依 attached/detached 模式處理。自訂模板必須透過平台 quoting，不能把
//! 含空白或 shell 字元的路徑直接串進命令。

use std::{
    env, io,
    path::{Path, PathBuf},
};

use crate::config::{
    ActionLaunchMode, ActionTargetScope, CustomOpenActionConfig, TerminalLauncherConfig,
    TerminalPluginConfig,
};

use super::platform::PlatformKind;

/// 描述拆解後的外部命令，供 `$EDITOR`、`vim` 這類需要在終端內阻塞執行的動作使用。
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandLineSpec {
    program: String,
    args: Vec<String>,
}

/// 描述目前要對選取項目執行的外部開啟動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenAction {
    Editor,
    Vim,
    Open,
    Reveal,
}

/// 描述目前選取項目的最小必要資訊。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenTarget {
    pub(crate) path: PathBuf,
    pub(crate) display_name: String,
    pub(crate) is_dir: bool,
}

/// 描述 `Open with` 面板中的單一選項。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenPickerOption {
    pub(crate) label: String,
    pub(crate) action: OpenPickerAction,
}

/// 描述 `Open with` 面板中某一列實際要執行的動作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpenPickerAction {
    Builtin(OpenAction),
    Custom(CustomOpenActionConfig),
}

/// 描述真正要交給外部系統執行的命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub mode: LaunchMode,
}

/// 描述外部命令應該如何執行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    TerminalBlocking,
    Detached,
    NewTerminal { current_dir: PathBuf },
}

/// 建立「在指定目錄開新終端」的跨平台命令。
///
/// 優先順序：
/// 1. `[terminal]` 全域覆寫命令。
/// 2. `[[terminals]]` 自訂外掛清單（依環境變數與程序特徵匹配）。
/// 3. 系統內建預設終端適配（WezTerm, Alacritty, Windows Terminal 等）。
pub(crate) fn build_terminal_launch_spec(
    path: &Path,
    launcher: Option<&TerminalLauncherConfig>,
    plugins: &[TerminalPluginConfig],
) -> io::Result<LaunchSpec> {
    if let Some(launcher) = launcher {
        let action = CustomOpenActionConfig {
            name: String::from("terminal"),
            scope: ActionTargetScope::Directory,
            mode: ActionLaunchMode::Detached,
            command: launcher.command.clone(),
            mac_command: launcher.mac_command.clone(),
            windows_command: launcher.windows_command.clone(),
        };
        let target = OpenTarget {
            path: path.to_path_buf(),
            display_name: path.display().to_string(),
            is_dir: true,
        };
        return build_custom_launch_spec(&target, &action);
    }

    let ancestor_processes = super::platform::detect_ancestor_process_names();
    for plugin in plugins {
        let env_match = !plugin.match_env.is_empty()
            && plugin
                .match_env
                .iter()
                .any(|var| std::env::var_os(var).is_some());
        let process_match = !plugin.match_process.is_empty()
            && plugin.match_process.iter().any(|proc_name| {
                let proc_lower = proc_name.to_lowercase();
                ancestor_processes.iter().any(|p| p.contains(&proc_lower))
            });

        if env_match || process_match {
            let action = CustomOpenActionConfig {
                name: plugin.name.clone(),
                scope: ActionTargetScope::Directory,
                mode: ActionLaunchMode::Detached,
                command: plugin.command.clone(),
                mac_command: plugin.mac_command.clone(),
                windows_command: plugin.windows_command.clone(),
            };
            let target = OpenTarget {
                path: path.to_path_buf(),
                display_name: path.display().to_string(),
                is_dir: true,
            };
            return build_custom_launch_spec(&target, &action);
        }
    }

    super::platform::new_terminal_spec_for_platform(path, super::platform::current_platform())
}

/// 根據目標項目決定 `Open with` 面板應顯示的選項。
pub(crate) fn open_picker_options(target: &OpenTarget) -> Vec<OpenPickerOption> {
    if target.is_dir {
        vec![
            OpenPickerOption {
                label: "$EDITOR".to_string(),
                action: OpenPickerAction::Builtin(OpenAction::Editor),
            },
            OpenPickerOption {
                label: "Vim".to_string(),
                action: OpenPickerAction::Builtin(OpenAction::Vim),
            },
            OpenPickerOption {
                label: "Open".to_string(),
                action: OpenPickerAction::Builtin(OpenAction::Open),
            },
            OpenPickerOption {
                label: "Reveal".to_string(),
                action: OpenPickerAction::Builtin(OpenAction::Reveal),
            },
        ]
    } else {
        vec![
            OpenPickerOption {
                label: "$EDITOR".to_string(),
                action: OpenPickerAction::Builtin(OpenAction::Editor),
            },
            OpenPickerOption {
                label: "Vim".to_string(),
                action: OpenPickerAction::Builtin(OpenAction::Vim),
            },
            OpenPickerOption {
                label: "Reveal".to_string(),
                action: OpenPickerAction::Builtin(OpenAction::Reveal),
            },
        ]
    }
}

/// 建立預設開啟動作。
pub(crate) fn default_open_action() -> OpenAction {
    OpenAction::Editor
}

/// 依照目標與動作組出真正要執行的命令。
pub(crate) fn build_launch_spec(target: &OpenTarget, action: OpenAction) -> io::Result<LaunchSpec> {
    match action {
        OpenAction::Editor => {
            if target.is_dir || !is_text_like_path(&target.path) {
                system_open_spec(&target.path)
            } else if let Some(editor) = preferred_editor_command() {
                Ok(blocking_command_spec(editor, &target.path))
            } else {
                system_open_spec(&target.path)
            }
        }
        OpenAction::Vim => Ok(blocking_command_spec(
            CommandLineSpec {
                program: default_vim_command().to_string(),
                args: Vec::new(),
            },
            &target.path,
        )),
        OpenAction::Open => system_open_spec(&target.path),
        OpenAction::Reveal => reveal_in_system_spec(&target.path),
    }
}

/// 依照自訂動作建立真正要執行的命令。
pub(crate) fn build_custom_launch_spec(
    target: &OpenTarget,
    action: &CustomOpenActionConfig,
) -> io::Result<LaunchSpec> {
    build_custom_launch_spec_for_platform(target, action, super::platform::current_platform())
}

/// 依照指定平台建立自訂動作命令，供跨平台測試使用。
pub(crate) fn build_custom_launch_spec_for_platform(
    target: &OpenTarget,
    action: &CustomOpenActionConfig,
    platform: PlatformKind,
) -> io::Result<LaunchSpec> {
    let template = match platform {
        PlatformKind::Windows => action
            .windows_command
            .as_deref()
            .or(action.command.as_deref()),
        PlatformKind::MacOs => action.mac_command.as_deref().or(action.command.as_deref()),
        PlatformKind::LinuxLike => action.command.as_deref(),
    }
    .ok_or_else(|| {
        io::Error::other(format!(
            "action {} has no command for this platform",
            action.name
        ))
    })?;

    let command = expand_custom_action_template(template, target, platform);
    let mode = match action.mode {
        ActionLaunchMode::Detached => LaunchMode::Detached,
        ActionLaunchMode::TerminalBlocking => LaunchMode::TerminalBlocking,
    };

    let (program, args) = match platform {
        PlatformKind::Windows => ("cmd.exe".to_string(), vec!["/C".to_string(), command]),
        PlatformKind::MacOs | PlatformKind::LinuxLike => {
            ("sh".to_string(), vec!["-lc".to_string(), command])
        }
    };

    Ok(LaunchSpec {
        program,
        args,
        mode,
    })
}

/// 取得目前環境中可用的 `$EDITOR` 設定；若未設定則回傳 `None`。
fn preferred_editor() -> Option<String> {
    env::var("EDITOR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 將 `$EDITOR` 的內容拆成實際命令與參數，避免再依賴 shell 包裝。
fn preferred_editor_command() -> Option<CommandLineSpec> {
    let editor = preferred_editor()?;
    parse_command_line(&editor)
}

/// 根據拆解後的命令與目標路徑，建立阻塞式終端命令。
fn blocking_command_spec(command: CommandLineSpec, path: &Path) -> LaunchSpec {
    let mut args = command.args;
    args.push(path.display().to_string());
    LaunchSpec {
        program: command.program,
        args,
        mode: LaunchMode::TerminalBlocking,
    }
}

/// 回傳內建 `Vim` 動作預設要執行的命令名稱。
fn default_vim_command() -> &'static str {
    "vim"
}

/// 判斷某個路徑是否應該被視為文字類型檔案。
pub(crate) fn is_text_like_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.contains('.') {
        return true;
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());

    matches!(
        extension.as_deref(),
        Some(
            "txt"
                | "md"
                | "markdown"
                | "rs"
                | "toml"
                | "json"
                | "yaml"
                | "yml"
                | "js"
                | "ts"
                | "jsx"
                | "tsx"
                | "html"
                | "css"
                | "scss"
                | "py"
                | "sh"
                | "zsh"
                | "bash"
                | "c"
                | "cc"
                | "cpp"
                | "h"
                | "hpp"
                | "java"
                | "go"
                | "rb"
                | "php"
                | "lua"
                | "sql"
                | "csv"
                | "tsv"
                | "xml"
                | "env"
                | "ini"
                | "conf"
                | "log"
        )
    )
}

/// 依目前編譯平台建立「交給系統預設程式開啟」的啟動規格。
///
/// 參數：`path: &Path`，要開啟的檔案或目錄。
/// 回傳：`io::Result<LaunchSpec>`，只描述命令，不會在此函數啟動 child process。
fn system_open_spec(path: &Path) -> io::Result<LaunchSpec> {
    super::platform::system_open_spec_for_platform(path, super::platform::current_platform())
}

/// 依照目前平台建立「在系統檔案管理器中顯示目標」的命令。
fn reveal_in_system_spec(path: &Path) -> io::Result<LaunchSpec> {
    super::platform::reveal_in_system_spec_for_platform(path, super::platform::current_platform())
}

/// 將像 `nvim -p` 或 `"C:\\Program Files\\Vim\\vim.exe" -u NONE` 這種字串拆成命令與參數。
fn parse_command_line(input: &str) -> Option<CommandLineSpec> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = input.trim().chars().peekable();

    while let Some(ch) = chars.next() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            None if ch == '\\' => {
                if let Some(next) = chars.peek().copied() {
                    if next == '"' || next == '\'' || next.is_whitespace() || next == '\\' {
                        current.push(next);
                        chars.next();
                    } else {
                        current.push(ch);
                    }
                } else {
                    current.push(ch);
                }
            }
            None => current.push(ch),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    let mut parts = parts.into_iter();
    let program = parts.next()?;
    Some(CommandLineSpec {
        program,
        args: parts.collect(),
    })
}

/// 將 plugins.toml action 中的 placeholder 展開成已依平台 quoting 的值。
///
/// 參數：
/// - `template: &str`，可包含 `{path}`、`{parent}`、`{name}`、`{stem}` 的命令模板。
/// - `target: &OpenTarget`，目前選取項目的完整路徑與類型。
/// - `platform: PlatformKind`，決定 shell escaping 採用 Windows 或 POSIX 規則。
///
/// 回傳：`String`，可再交給 command-line parser 的展開結果。placeholder 在取代前
/// 已 quote，避免空白路徑被拆成多個參數；自訂模板本身仍視為使用者信任的設定。
fn expand_custom_action_template(
    template: &str,
    target: &OpenTarget,
    platform: PlatformKind,
) -> String {
    let path = shell_quote_for_platform(&target.path.display().to_string(), platform);
    let parent = shell_quote_for_platform(
        &target
            .path
            .parent()
            .unwrap_or(&target.path)
            .display()
            .to_string(),
        platform,
    );
    let name = shell_quote_for_platform(&target.display_name, platform);
    let stem = shell_quote_for_platform(
        target
            .path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default(),
        platform,
    );

    template
        .replace("{path}", &path)
        .replace("{parent}", &parent)
        .replace("{name}", &name)
        .replace("{stem}", &stem)
}

/// 把單一 placeholder 值包成目前平台 shell 可安全視為一個參數的文字。
///
/// 參數：`value: &str` 是未跳脫文字，`platform` 是目標 shell 類型。
/// 回傳：`String`，Windows 使用雙引號，macOS/Linux 使用 POSIX 單引號規則。
fn shell_quote_for_platform(value: &str, platform: PlatformKind) -> String {
    match platform {
        PlatformKind::Windows => format!("\"{}\"", value.replace('"', "\"\"")),
        PlatformKind::MacOs | PlatformKind::LinuxLike => {
            format!("'{}'", value.replace('\'', "'\"'\"'"))
        }
    }
}

/// 判斷 plugins.toml 的自訂動作是否應出現在目前檔案或目錄的 Open picker。
///
/// 參數：`action` 是已解析設定，`target` 是目前選取項目。
/// 回傳：`bool`，符合 file/dir/both scope 時為 `true`。
pub(crate) fn custom_action_applies_to_target(
    action: &CustomOpenActionConfig,
    target: &OpenTarget,
) -> bool {
    match action.scope {
        ActionTargetScope::File => !target.is_dir,
        ActionTargetScope::Directory => target.is_dir,
        ActionTargetScope::Both => true,
    }
}

#[cfg(test)]
#[path = "tests/open_test.rs"]
mod tests;
