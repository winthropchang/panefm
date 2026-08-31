//! PaneFM 執行檔入口。
//!
//! 這裡刻意保持輕量，只把控制權交給 library crate。終端初始化、事件迴圈與清理
//! 都集中在 `file_manager`，讓測試可以直接呼叫 library，而不必啟動另一個程序。

use anyhow::Result;
use std::ffi::OsStr;

/// 判斷第一個參數是否要求顯示程式版本。
fn is_version_flag(argument: Option<&OsStr>) -> bool {
    matches!(argument.and_then(OsStr::to_str), Some("--version" | "-V"))
}

/// 啟動 PaneFM 終端檔案管理器。
///
/// 參數：無。
/// 回傳：`Result<()>`，正常離開時回傳 `Ok(())`，初始化或執行失敗時回傳錯誤。
fn main() -> Result<()> {
    if is_version_flag(std::env::args_os().nth(1).as_deref()) {
        println!("panefm {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    panefm::run()
}

#[cfg(test)]
mod tests {
    use super::is_version_flag;
    use std::ffi::OsStr;

    #[test]
    fn recognizes_version_flags() {
        assert!(is_version_flag(Some(OsStr::new("--version"))));
        assert!(is_version_flag(Some(OsStr::new("-V"))));
        assert!(!is_version_flag(Some(OsStr::new("--help"))));
        assert!(!is_version_flag(None));
    }
}
