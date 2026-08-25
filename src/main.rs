use anyhow::Result;

/// 啟動 PaneFM 終端檔案管理器。
///
/// 參數：無。
/// 回傳：`Result<()>`，正常離開時回傳 `Ok(())`，初始化或執行失敗時回傳錯誤。
fn main() -> Result<()> {
    panefm::run()
}
