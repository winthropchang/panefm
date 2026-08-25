//! 外部工具偵測與 dependency status 面板的資料來源。
//!
//! PaneFM 不把 fd、rg、fzf、zoxide 綁進執行檔，而是在 PATH 中尋找系統安裝版本。
//! 新增必要工具時，必須在這裡的集中清單、缺少訊息與測試一起更新。

use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

/// 表示一個外部工具目前是否可由系統 `PATH` 找到。
///
/// 欄位：
/// - `name: &'static str`，顯示在依賴面板中的工具名稱。
/// - `installed: bool`，`true` 代表目前可執行，`false` 代表尚未安裝。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolStatus {
    pub(crate) name: &'static str,
    pub(crate) installed: bool,
}

/// 尋找 `PATH` 中的外部命令，並處理 macOS 與 Windows 的命令副檔名差異。
pub(crate) fn find_system_command(name: &str) -> Option<OsString> {
    find_command_in_path(OsStr::new(name), std::env::var_os("PATH").as_deref())
}

/// 產生 fd、fzf、rg、zoxide 的完整安裝狀態列表。
///
/// 參數：無。
///
/// 回傳：`Vec<ToolStatus>`，依固定顯示順序排列的工具安裝狀態。
pub(crate) fn external_tool_statuses() -> Vec<ToolStatus> {
    ["fd", "fzf", "rg", "zoxide"]
        .into_iter()
        .map(|name| ToolStatus {
            name,
            installed: find_system_command(name).is_some(),
        })
        .collect()
}

/// 將狀態列表轉成適合狀態列顯示的單行訊息。
pub(crate) fn missing_tool_message(tool: &str) -> String {
    format!("missing dependency: {tool}; run :status to check fd, fzf, ripgrep (rg), and zoxide")
}

/// 在指定 PATH 中找出第一個可執行檔，Windows 會依序嘗試常見副檔名。
fn find_command_in_path(name: &OsStr, path: Option<&OsStr>) -> Option<OsString> {
    let path = path?;
    for directory in std::env::split_paths(path) {
        let candidate = directory.join(name);
        if is_file(&candidate) {
            return Some(candidate.into_os_string());
        }
        #[cfg(target_os = "windows")]
        for extension in [".exe", ".cmd", ".bat"] {
            let candidate = directory.join(format!("{}{}", name.to_string_lossy(), extension));
            if is_file(&candidate) {
                return Some(candidate.into_os_string());
            }
        }
    }
    None
}

/// 判斷 PATH 候選是否為一般檔案；權限細節交由作業系統在啟動時判斷。
fn is_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::{external_tool_statuses, find_command_in_path, missing_tool_message};
    use std::ffi::OsStr;
    use tempfile::tempdir;

    #[test]
    /// 驗證 PATH 搜尋會回傳第一個可執行的同名工具。
    /// 保護目的：避免外部依賴清單或 PATH 偵測重構後，向使用者回報錯誤的安裝狀態。
    fn finds_first_command_in_path() {
        let first = tempdir().expect("first tempdir");
        let second = tempdir().expect("second tempdir");
        let first_command = first.path().join("rg");
        std::fs::write(&first_command, b"rg").expect("command");
        std::fs::write(second.path().join("rg"), b"rg").expect("command");
        let path = std::env::join_paths([first.path(), second.path()]).expect("PATH");
        assert_eq!(
            find_command_in_path(OsStr::new("rg"), Some(path.as_os_str())),
            Some(first_command.into_os_string())
        );
    }

    #[test]
    /// 驗證不存在的命令不會被 dependency status 誤判為已安裝。
    /// 保護目的：避免外部依賴清單或 PATH 偵測重構後，向使用者回報錯誤的安裝狀態。
    fn missing_command_is_not_reported_as_installed() {
        let dir = tempdir().expect("tempdir");
        let path = std::env::join_paths([dir.path()]).expect("PATH");
        assert_eq!(
            find_command_in_path(OsStr::new("missing"), Some(path.as_os_str())),
            None
        );
    }

    #[test]
    /// 驗證缺少依賴訊息包含工具名稱與 `status` 指令提示。
    /// 保護目的：避免外部依賴清單或 PATH 偵測重構後，向使用者回報錯誤的安裝狀態。
    fn missing_message_names_installable_tools() {
        let message = missing_tool_message("rg");
        assert!(message.contains("rg"));
        assert!(message.contains("fd"));
        assert!(message.contains("ripgrep"));
        assert!(message.contains("zoxide"));
    }

    #[test]
    /// 驗證 status 面板集中列出 fd、rg、fzf 與 zoxide 等必要工具。
    /// 保護目的：避免外部依賴清單或 PATH 偵測重構後，向使用者回報錯誤的安裝狀態。
    fn dependency_status_lists_all_required_tools() {
        let statuses = external_tool_statuses();
        assert_eq!(
            statuses.iter().map(|tool| tool.name).collect::<Vec<_>>(),
            vec!["fd", "fzf", "rg", "zoxide"]
        );
    }
}
