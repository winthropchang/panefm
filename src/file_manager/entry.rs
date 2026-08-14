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
    pub(crate) modified: SystemTime,
    pub(crate) created: SystemTime,
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
