use std::ffi::OsString;

use anyhow::{Result, anyhow};

use super::tools::{find_system_command, missing_tool_message};

/// 尋找系統安裝的 `fd` 檔名搜尋工具。
///
/// 參數：無。
///
/// 回傳：`Result<OsString>`。
/// - 成功時回傳可交給 `Command::new` 的完整命令路徑。
/// - 找不到時回傳缺少 `fd` 依賴的錯誤訊息。
pub(crate) fn fd_command() -> Result<OsString> {
    find_system_command("fd").ok_or_else(|| anyhow!("{}", missing_tool_message("fd")))
}
