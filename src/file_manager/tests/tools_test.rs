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
