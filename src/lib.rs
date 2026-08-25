pub mod config;
mod file_manager;
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
