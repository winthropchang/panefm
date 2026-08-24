use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

/// 回傳三個外部工具的安裝狀態，供缺少依賴時顯示給使用者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolStatus {
    pub(crate) name: &'static str,
    pub(crate) installed: bool,
}

/// 尋找 `PATH` 中的外部命令，並處理 macOS 與 Windows 的命令副檔名差異。
pub(crate) fn find_system_command(name: &str) -> Option<OsString> {
    find_command_in_path(OsStr::new(name), std::env::var_os("PATH").as_deref())
}

/// 產生 fzf、rg、zoxide 的完整安裝狀態列表。
pub(crate) fn external_tool_statuses() -> Vec<ToolStatus> {
    ["fzf", "rg", "zoxide"]
        .into_iter()
        .map(|name| ToolStatus {
            name,
            installed: find_system_command(name).is_some(),
        })
        .collect()
}

/// 將狀態列表轉成適合狀態列顯示的單行訊息。
pub(crate) fn missing_tool_message(tool: &str) -> String {
    format!("missing dependency: {tool}; install fzf, ripgrep (rg), and zoxide")
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
    fn missing_command_is_not_reported_as_installed() {
        let dir = tempdir().expect("tempdir");
        let path = std::env::join_paths([dir.path()]).expect("PATH");
        assert_eq!(
            find_command_in_path(OsStr::new("missing"), Some(path.as_os_str())),
            None
        );
    }

    #[test]
    fn missing_message_names_installable_tools() {
        let message = missing_tool_message("rg");
        assert!(message.contains("rg"));
        assert!(message.contains("ripgrep"));
        assert!(message.contains("zoxide"));
    }

    #[test]
    fn dependency_status_lists_all_required_tools() {
        let statuses = external_tool_statuses();
        assert_eq!(
            statuses.iter().map(|tool| tool.name).collect::<Vec<_>>(),
            vec!["fzf", "rg", "zoxide"]
        );
    }
}
