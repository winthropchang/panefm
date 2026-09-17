//! 檔案操作模組子系統，拆分為開啟、建立/重新命名、壓縮、刪除/資源回收筒、剪貼簿、貼上/復原、移動/複製等子模組。

pub(crate) mod archive;
pub(crate) mod clipboard;
pub(crate) mod create_rename;
pub(crate) mod delete;
pub(crate) mod open;
pub(crate) mod paste;
pub(crate) mod transfer;
