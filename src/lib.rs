//! PaneFM 的 library 入口與可公開重用的設定、主題 API。
//!
//! `main.rs` 只呼叫本模組的 [`run`]；實際 TUI 實作保持在私有的 `file_manager`
//! 模組，避免內部狀態在尚未穩定前成為對外相容性承諾。

pub mod config;
pub mod file_manager;
pub mod theme;

use anyhow::Result;

/// 啟動整個 PaneFM terminal file manager。
///
/// 參數：無。
/// 回傳：`Result<()>`。
/// - 成功時代表 TUI 已正常啟動並在結束後完成清理。
/// - 失敗時代表初始化、事件迴圈或終端還原流程中出現錯誤。
pub fn run() -> Result<()> {
    file_manager::run()
}
