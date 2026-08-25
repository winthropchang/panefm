//! `ripgrep` 外部依賴的定位入口。
//!
//! 內容搜尋固定使用 rg JSON stream；未安裝時明確失敗並交由 dependency panel
//! 說明，避免不同平台走到效能與結果語意不同的備援實作。

use std::ffi::OsString;

use anyhow::{Result, anyhow};

use super::tools::{find_system_command, missing_tool_message};

/// 尋找系統安裝的 `rg`（ripgrep）。
///
/// 參數：無。回傳可交給 `Command::new` 的命令路徑；找不到時回傳缺少依賴錯誤。
pub(crate) fn rg_command() -> Result<OsString> {
    find_system_command("rg").ok_or_else(|| anyhow!("{}", missing_tool_message("rg")))
}
