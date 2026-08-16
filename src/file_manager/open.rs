use std::{
    env, io,
    path::{Path, PathBuf},
};

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
    pub(crate) label: &'static str,
    pub(crate) action: OpenAction,
}

/// 描述真正要交給外部系統執行的命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchSpec {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) mode: LaunchMode,
}

/// 描述外部命令應該如何執行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    TerminalBlocking,
    Detached,
}

/// 根據目標項目決定 `Open with` 面板應顯示的選項。
pub(crate) fn open_picker_options(target: &OpenTarget) -> Vec<OpenPickerOption> {
    if target.is_dir {
        vec![
            OpenPickerOption {
                label: "$EDITOR",
                action: OpenAction::Editor,
            },
            OpenPickerOption {
                label: "Vim",
                action: OpenAction::Vim,
            },
            OpenPickerOption {
                label: "Open",
                action: OpenAction::Open,
            },
            OpenPickerOption {
                label: "Reveal",
                action: OpenAction::Reveal,
            },
        ]
    } else {
        vec![
            OpenPickerOption {
                label: "$EDITOR",
                action: OpenAction::Editor,
            },
            OpenPickerOption {
                label: "Vim",
                action: OpenAction::Vim,
            },
            OpenPickerOption {
                label: "Reveal",
                action: OpenAction::Reveal,
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        LaunchMode, OpenAction, OpenTarget, build_launch_spec, is_text_like_path,
        open_picker_options, parse_command_line,
    };

    #[test]
    fn text_like_detection_matches_common_extensions() {
        assert!(is_text_like_path(&PathBuf::from("notes.txt")));
        assert!(is_text_like_path(&PathBuf::from("Cargo.toml")));
        assert!(is_text_like_path(&PathBuf::from("README")));
        assert!(!is_text_like_path(&PathBuf::from("photo.jpg")));
        assert!(!is_text_like_path(&PathBuf::from("report.pdf")));
    }

    #[test]
    fn file_picker_omits_system_open_entry() {
        let options = open_picker_options(&OpenTarget {
            path: PathBuf::from("notes.txt"),
            display_name: "notes.txt".to_string(),
            is_dir: false,
        });
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].action, OpenAction::Editor);
        assert_eq!(options[1].action, OpenAction::Vim);
        assert_eq!(options[2].action, OpenAction::Reveal);
    }

    #[test]
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
    fn parse_command_line_supports_quoted_program_and_args() {
        let command = parse_command_line("\"C:\\Program Files\\Neovim\\bin\\nvim.exe\" -u NONE")
            .expect("command");

        assert_eq!(command.program, "C:\\Program Files\\Neovim\\bin\\nvim.exe");
        assert_eq!(command.args, vec!["-u", "NONE"]);
    }
}
