//! PaneFM 的 library 入口與可公開重用的設定、主題 API。
//!
//! `main.rs` 只呼叫本模組的 [`run`]；實際 TUI 實作保持在私有的 `file_manager`
//! 模組，避免內部狀態在尚未穩定前成為對外相容性承諾。

pub mod config;
pub mod file_manager;
pub mod theme;
pub mod updater;

use anyhow::Result;

/// 啟動整個 PaneFM terminal file manager（預設參數）。
pub fn run() -> Result<()> {
    file_manager::run()
}

/// 啟動整個 PaneFM terminal file manager，並套用自訂命令列啟動參數。
pub fn run_with_options(args: updater::LaunchArgs) -> Result<()> {
    file_manager::run_with_options(args)
}
