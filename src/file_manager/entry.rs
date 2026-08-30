//! 目錄列表中單一檔案系統項目的跨平台資料模型。
//!
//! 掃描目錄時會把 metadata 正規化成 `FileEntry`，讓排序與 UI 不必直接依賴平台
//! API。Windows 不存在的 Unix 權限等資訊以 `Option` 表示，而不是在 UI 端猜測。

use std::{path::PathBuf, time::SystemTime};

/// 表示目錄列表中的單一檔案或資料夾項目。
///
/// 這個結構是檔案瀏覽清單與預覽系統共用的最小單位資料。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) is_dir: bool,
    pub(crate) size: u64,
    /// 目錄目前已遞迴統計到的內容大小；一般檔案不使用這個欄位。
    pub(crate) directory_size: Option<u64>,
    /// `true` 表示 `directory_size` 已包含整棵目錄樹，而不是背景掃描中的暫存值。
    pub(crate) directory_size_complete: bool,
    pub(crate) modified: SystemTime,
    pub(crate) created: SystemTime,
    pub(crate) readonly: bool,
    pub(crate) unix_mode: Option<u32>,
}

impl FileEntry {
    /// 產生適合顯示在列表中的名稱。
    ///
    /// 參數：
    /// - `self: &FileEntry`，目前的檔案項目。
    ///
    /// 回傳：`String`。
    /// - 若是資料夾，名稱尾端會補上 `/`。
    /// - 若是一般檔案，直接回傳原名稱。
    pub(crate) fn display_name(&self) -> String {
        if self.is_dir {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}
