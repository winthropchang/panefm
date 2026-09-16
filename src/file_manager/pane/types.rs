use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use ratatui::text::Line;

use crate::file_manager::entry::FileEntry;

/// 描述一次貼上實際完成後，可供上層建立 Undo 紀錄的檔案系統結果。
///
/// `backup_path` 只會在覆蓋既有目標時存在；該路徑保存覆蓋前的原內容，所有權會交給
/// `OperationHistory`。歷史被復原或淘汰前，上層不可提前刪除這個備份。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PasteOutcome {
    /// 畫面狀態列使用的貼上後名稱。
    pub(crate) display_name: String,
    /// 本次實際建立或移入的完整目的路徑。
    pub(crate) target_path: PathBuf,
    /// 覆蓋前原目標的隱藏備份；一般貼上時為 `None`。
    pub(crate) backup_path: Option<PathBuf>,
}

/// 描述背景傳輸排程器送給 task manager 的進度事件。
///
/// 走訪器發現檔案時先回報 [`TransferProgress::BytesDiscovered`]，file worker 完成實際
/// 寫入時再回報 [`TransferProgress::BytesCopied`]。這讓目錄只需走訪一次，不必為了
/// 百分比另外掃描整棵樹；[`TransferProgress::TargetVisible`] 則通知 UI 第一層目標已建立。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferProgress {
    TargetVisible,
    BytesDiscovered(u64),
    BytesCopied(u64),
}

/// VCS Diff 預覽行快取的執行緒安全容器類型。
pub(crate) type PreviewDiffCache =
    Arc<Mutex<Option<(PathBuf, Option<SystemTime>, Vec<Line<'static>>)>>>;

/// 描述列表過濾目前使用的比對模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FilterMode {
    /// 一般模式：以空白拆分關鍵字，要求各詞皆為連續子字串。
    #[default]
    Normal,
    /// 模糊搜尋模式：使用 Nucleo 進行子序列模糊匹配與相關性評分排序。
    Fuzzy,
}

#[derive(Debug, Clone)]
pub(crate) struct FilterCache {
    pub(crate) query: String,
    pub(crate) is_fuzzy: bool,
    pub(crate) show_hidden: bool,
    pub(crate) entry_revision: u64,
    pub(crate) matched_indices: Vec<usize>,
}

/// 描述 pane 目前使用的排序方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortMode {
    Alphabetical { reverse: bool },
    Natural { reverse: bool },
    Size { reverse: bool },
    Modified { reverse: bool },
    Created { reverse: bool },
    Extension { reverse: bool },
    Random,
}

/// 描述列表右側附加欄位目前採用的 linemode。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineMode {
    Size,
    Permissions,
    Btime,
    Mtime,
    None,
}

impl SortMode {
    /// 回傳適合顯示在狀態列中的名稱。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Alphabetical { reverse: false } => "alphabetical",
            Self::Alphabetical { reverse: true } => "alphabetical (reverse)",
            Self::Natural { reverse: false } => "natural",
            Self::Natural { reverse: true } => "natural (reverse)",
            Self::Size { reverse: false } => "size",
            Self::Size { reverse: true } => "size (reverse)",
            Self::Modified { reverse: false } => "modified",
            Self::Modified { reverse: true } => "modified (reverse)",
            Self::Created { reverse: false } => "birth",
            Self::Created { reverse: true } => "birth (reverse)",
            Self::Extension { reverse: false } => "extension",
            Self::Extension { reverse: true } => "extension (reverse)",
            Self::Random => "random",
        }
    }

    /// 回傳右側欄位目前應該顯示的資訊類型。
    pub(crate) fn detail_kind(self) -> SortDetailKind {
        match self {
            Self::Size { .. } => SortDetailKind::Size,
            Self::Modified { .. } => SortDetailKind::Modified,
            Self::Created { .. } => SortDetailKind::Created,
            Self::Extension { .. } => SortDetailKind::Extension,
            _ => SortDetailKind::None,
        }
    }
}

impl LineMode {
    /// 回傳適合顯示在狀態列與 pane 標題上的 linemode 名稱。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Size => "size",
            Self::Permissions => "permissions",
            Self::Btime => "btime",
            Self::Mtime => "mtime",
            Self::None => "none",
        }
    }

    /// 將 linemode 轉成右側欄位實際應顯示的資料種類。
    pub(crate) fn detail_kind(self) -> SortDetailKind {
        match self {
            Self::Size => SortDetailKind::Size,
            Self::Permissions => SortDetailKind::Permissions,
            Self::Btime => SortDetailKind::Created,
            Self::Mtime => SortDetailKind::Modified,
            Self::None => SortDetailKind::None,
        }
    }
}

/// 描述列表右側欄位目前應該顯示哪一種排序依據。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortDetailKind {
    None,
    Size,
    Modified,
    Created,
    Extension,
    Permissions,
}

/// 描述搜尋列表下方 preview 區塊需要繪製的內容與捲動資訊。
#[derive(Debug, Clone)]
pub(crate) struct SearchPreviewData {
    pub(crate) title: String,
    pub(crate) lines: Vec<Line<'static>>,
}

/// 大型目錄載入回傳的事件類型。
#[derive(Debug)]
pub(crate) enum DirectoryLoadProgress {
    /// 快速發現的增量檔案清單。
    #[allow(dead_code)]
    Batch {
        entries: Vec<FileEntry>,
        is_first_chunk: bool,
    },
    /// 完整掃描結束，傳入最終全量已補齊 metadata 且在背景排序好的清單。
    Complete(Vec<FileEntry>),
}
