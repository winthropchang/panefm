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
pub(crate) fn find_command_in_path(name: &OsStr, path: Option<&OsStr>) -> Option<OsString> {
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

/// 快速檢查系統 PATH 中是否有指定名稱的命令。
pub(crate) fn is_command_in_path(cmd: &str) -> bool {
    let path = std::env::var_os("PATH");
    find_command_in_path(std::ffi::OsStr::new(cmd), path.as_deref()).is_some()
}

/// 判斷 PATH 候選是否為一般檔案；權限細節交由作業系統在啟動時判斷。
fn is_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
#[path = "tests/tools_test.rs"]
mod tests;
