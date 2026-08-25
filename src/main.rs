//! PaneFM 執行檔入口。
//!
//! 這裡刻意保持輕量，只把控制權交給 library crate。終端初始化、事件迴圈與清理
//! 都集中在 `file_manager`，讓測試可以直接呼叫 library，而不必啟動另一個程序。

use anyhow::Result;

/// 啟動 PaneFM 終端檔案管理器。
///
/// 參數：無。
/// 回傳：`Result<()>`，正常離開時回傳 `Ok(())`，初始化或執行失敗時回傳錯誤。
fn main() -> Result<()> {
    panefm::run()
}
