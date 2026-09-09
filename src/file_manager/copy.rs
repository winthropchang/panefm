//! Copy picker 的文字格式與系統剪貼簿內容產生器。
//!
//! 這裡的 copy 是「複製路徑/檔名文字」，不同於 `App` 的檔案複製剪貼簿。URL、父
//! 目錄與副檔名處理集中在此，確保 macOS 和 Windows 傳給其他軟體的文字格式一致。

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

/// 取得適合放入系統剪貼簿的檔名，沒有 final component 時退回完整顯示路徑。
///
/// 參數：`path: &Path`；回傳不含父目錄的 lossy UTF-8 `String`。
fn file_name_text(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 取得不含副檔名的檔名；目錄名稱中的句點不會被誤當成副檔名移除。
///
/// 參數：`path: &Path`；回傳檔案 stem 或目錄原名。
fn file_stem_text(path: &Path) -> String {
    if path.is_dir() {
        return file_name_text(path);
    }

    path.file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name_text(path))
}

/// 把路徑轉成其他編輯器與 shell 可直接使用的原生平台路徑文字。
///
/// 這裡刻意不產生 `file://` URL，也不做 `%20` 編碼，因為使用者通常會把結果貼到
/// 終端、IDE 或檔案選擇器。參數是 `path`，回傳 `Path::display()` 的字串。
fn path_to_clipboard_text(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
#[path = "tests/copy_test.rs"]
mod tests;
