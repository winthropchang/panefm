use std::path::Path;

use anyhow::Result;

use super::open::OpenTarget;

/// 描述 `Copy` 小視窗中的單一複製動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyAction {
    FileUrl,
    DirectoryUrl,
    Filename,
    FilenameWithoutExtension,
}

/// 描述 `Copy` 小視窗裡的一列選項。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyPickerOption {
    pub(crate) shortcut: char,
    pub(crate) label: &'static str,
    pub(crate) action: CopyAction,
}

/// 回傳 `Copy` 小視窗應顯示的所有選項。
pub(crate) fn copy_picker_options() -> Vec<CopyPickerOption> {
    vec![
        CopyPickerOption {
            shortcut: 'u',
            label: "Copy file path",
            action: CopyAction::FileUrl,
        },
        CopyPickerOption {
            shortcut: 'd',
            label: "Copy directory path",
            action: CopyAction::DirectoryUrl,
        },
        CopyPickerOption {
            shortcut: 'f',
            label: "Copy filename",
            action: CopyAction::Filename,
        },
        CopyPickerOption {
            shortcut: 'n',
            label: "Copy filename without extension",
            action: CopyAction::FilenameWithoutExtension,
        },
    ]
}

/// 根據目標與複製動作產生要寫進系統剪貼簿的文字。
pub(crate) fn build_copy_text(target: &OpenTarget, action: CopyAction) -> Result<String> {
    match action {
        CopyAction::FileUrl => Ok(path_to_clipboard_text(&target.path)),
        CopyAction::DirectoryUrl => {
            let directory_path = if target.is_dir {
                target.path.as_path()
            } else {
                target.path.parent().unwrap_or(target.path.as_path())
            };
            Ok(path_to_clipboard_text(directory_path))
        }
        CopyAction::Filename => Ok(file_name_text(&target.path)),
        CopyAction::FilenameWithoutExtension => Ok(file_stem_text(&target.path)),
    }
}

/// 依動作回傳操作完成後適合顯示在狀態列的文字。
pub(crate) fn copy_action_status_label(action: CopyAction) -> &'static str {
    match action {
        CopyAction::FileUrl => "copied file path",
        CopyAction::DirectoryUrl => "copied directory path",
        CopyAction::Filename => "copied filename",
        CopyAction::FilenameWithoutExtension => "copied filename without extension",
    }
}

fn file_name_text(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn file_stem_text(path: &Path) -> String {
    if path.is_dir() {
        return file_name_text(path);
    }

    path.file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name_text(path))
}

fn path_to_clipboard_text(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{CopyAction, build_copy_text, copy_action_status_label};
    use crate::file_manager::open::OpenTarget;

    #[test]
    fn copy_file_path_preserves_plain_absolute_path() {
        let target = OpenTarget {
            path: PathBuf::from("/tmp/hello world.txt"),
            display_name: String::from("hello world.txt"),
            is_dir: false,
        };

        let text = build_copy_text(&target, CopyAction::FileUrl).expect("file path");
        assert_eq!(text, "/tmp/hello world.txt");
    }

    #[test]
    fn copy_directory_path_uses_parent_for_files() {
        let target = OpenTarget {
            path: PathBuf::from("/tmp/docs/readme.md"),
            display_name: String::from("readme.md"),
            is_dir: false,
        };

        let text = build_copy_text(&target, CopyAction::DirectoryUrl).expect("dir path");
        assert_eq!(text, "/tmp/docs");
    }

    #[test]
    fn copy_filename_without_extension_uses_stem_for_files() {
        let target = OpenTarget {
            path: PathBuf::from("/tmp/archive.tar.gz"),
            display_name: String::from("archive.tar.gz"),
            is_dir: false,
        };

        let text =
            build_copy_text(&target, CopyAction::FilenameWithoutExtension).expect("filename stem");
        assert_eq!(text, "archive.tar");
    }

    #[test]
    fn copy_status_labels_are_human_readable() {
        assert_eq!(
            copy_action_status_label(CopyAction::FileUrl),
            "copied file path"
        );
        assert_eq!(
            copy_action_status_label(CopyAction::FilenameWithoutExtension),
            "copied filename without extension"
        );
    }
}
