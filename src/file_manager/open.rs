use std::{
    env, io,
    path::{Path, PathBuf},
};

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
            } else if preferred_editor().is_some() {
                Ok(LaunchSpec {
                    program: "/bin/sh".to_string(),
                    args: vec![
                        "-lc".to_string(),
                        "exec \"$EDITOR\" \"$1\"".to_string(),
                        "sh".to_string(),
                        target.path.display().to_string(),
                    ],
                    mode: LaunchMode::TerminalBlocking,
                })
            } else {
                system_open_spec(&target.path)
            }
        }
        OpenAction::Vim => Ok(LaunchSpec {
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                "exec vim \"$1\"".to_string(),
                "sh".to_string(),
                target.path.display().to_string(),
            ],
            mode: LaunchMode::TerminalBlocking,
        }),
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

#[cfg(target_os = "macos")]
fn system_open_spec(path: &Path) -> io::Result<LaunchSpec> {
    Ok(LaunchSpec {
        program: "open".to_string(),
        args: vec![path.display().to_string()],
        mode: LaunchMode::Detached,
    })
}

#[cfg(not(target_os = "macos"))]
fn system_open_spec(path: &Path) -> io::Result<LaunchSpec> {
    Ok(LaunchSpec {
        program: "xdg-open".to_string(),
        args: vec![path.display().to_string()],
        mode: LaunchMode::Detached,
    })
}

#[cfg(target_os = "macos")]
fn reveal_in_system_spec(path: &Path) -> io::Result<LaunchSpec> {
    Ok(LaunchSpec {
        program: "open".to_string(),
        args: vec!["-R".to_string(), path.display().to_string()],
        mode: LaunchMode::Detached,
    })
}

#[cfg(not(target_os = "macos"))]
fn reveal_in_system_spec(path: &Path) -> io::Result<LaunchSpec> {
    let parent = path.parent().unwrap_or(path);
    system_open_spec(parent)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        LaunchMode, OpenAction, OpenTarget, build_launch_spec, is_text_like_path,
        open_picker_options,
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
}
