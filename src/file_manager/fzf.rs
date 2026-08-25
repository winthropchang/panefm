//! `fzf` 外部依賴的定位入口。
//!
//! fzf 會暫時接管整個 terminal，因此本模組只解析可執行檔；終端停用與恢復必須
//! 留在 `file_manager::mod` 的生命週期層處理。

use std::ffi::OsString;

use anyhow::{Result, anyhow};

use super::tools::{find_system_command, missing_tool_message};

/// 尋找系統安裝的 `fzf`。
///
/// 參數：無。回傳可交給 `Command::new` 的命令路徑；找不到時回傳缺少依賴錯誤。
pub(crate) fn fzf_command() -> Result<OsString> {
    find_system_command("fzf").ok_or_else(|| anyhow!("{}", missing_tool_message("fzf")))
}
