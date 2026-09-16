//! 單一 panel 的目錄列表、排序、選取、預覽與檔案操作狀態。
//!
//! `PaneState` 是 PaneFM 的第一級物件：每個 split 都有獨立 cwd、游標、filter、
//! preview 與顯示模式。這一層不決定快捷鍵，也不繪製 popup；它提供可測試的資料
//! 操作給 `App`，並在變更檔案後重新載入列表與盡可能保留游標位置。

pub(crate) mod copy;
pub(crate) mod create;
pub(crate) mod filter;
pub(crate) mod find;
pub(crate) mod fs_read;
pub(crate) mod navigation;
pub(crate) mod ops;
pub(crate) mod preview;
pub(crate) mod preview_render;
pub(crate) mod selection;
pub(crate) mod sort;
pub(crate) mod tree;
pub(crate) mod types;

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use ratatui::widgets::ListState;

use crate::file_manager::{
    bookmark::BookmarkTarget,
    entry::FileEntry,
    preview::{ImageLoader, ImagePreviewCache, PreviewContentCacheMap},
    vcs::VcsRepoInfo,
};

#[allow(unused_imports)]
pub(crate) use copy::{
    commit_staged_copy, copy_file_and_verify, copy_file_and_verify_with,
    copy_file_native_with_progress, copy_file_native_with_progress_using,
    copy_file_streaming_with_progress, copy_file_with_native_fallback, is_sync_unsupported_error,
    remove_transfer_path, sync_target_file, unique_transfer_path,
};
#[allow(unused_imports)]
pub(crate) use create::{
    duplicate_name, split_name_for_duplicate, target_path_file_name, unique_target_path,
    validate_new_entry_name,
};
#[allow(unused_imports)]
pub(crate) use fs_read::{
    path_content_size, read_dir_entries, read_dir_entries_with_cancellation,
    stream_dir_entries_with_cancellation,
};
#[allow(unused_imports)]
pub(crate) use ops::{
    copy_path_direct_with_cleanup, copy_path_transactional, copy_path_transactional_with,
    move_path_with_fallback, remove_undo_backup,
};
#[allow(unused_imports)]
pub(crate) use preview_render::{
    build_search_preview_lines, highlight_cursor_line, highlight_preview_line,
    highlight_preview_matches, preview_match_positions,
};
#[allow(unused_imports)]
pub(crate) use sort::{
    compare_ascii_case_insensitive, compare_entries, compare_with_reverse, file_extension,
    is_hidden_name, natural_cmp, random_key, seed_from_path, sort_file_entries,
};
#[allow(unused_imports)]
pub(crate) use tree::{
    copy_dir_parallel_with_progress, copy_dir_recursive, copy_dir_recursive_with_progress,
};
#[allow(unused_imports)]
pub(crate) use types::{
    DirectoryLoadProgress, FilterCache, FilterMode, LineMode, PasteOutcome, PreviewDiffCache,
    SearchPreviewData, SortDetailKind, SortMode, TransferProgress,
};

/// 表示單一 pane 的完整瀏覽狀態。
///
/// 每個 pane 都獨立維護自己的目錄、游標與列表狀態，
/// 這樣分割視窗後每個區塊才可以各自操作。
#[derive(Debug, Clone)]
pub(crate) struct PaneState {
    /// 目前 pane 正在瀏覽的目錄。
    pub(crate) cwd: PathBuf,
    /// 目前這個 pane 對外應該被視為哪一個可書籤化目標。
    pub(crate) bookmark_target: BookmarkTarget,
    /// 目前目錄下的檔案與資料夾清單。
    pub(crate) entries: Vec<FileEntry>,
    /// 目前選取項目的索引位置。
    pub(crate) selected: usize,
    /// `ratatui` 的列表狀態，供畫面渲染使用。
    pub(crate) list_state: ListState,
    /// 目前列表區實際可顯示的列數，供半頁移動等行為計算步長。
    pub(crate) list_viewport_height: usize,
    /// 目前啟用中的過濾字串，`None` 代表沒有啟用 filter。
    pub(crate) filter_query: Option<String>,
    /// 目前使用的過濾模式（一般子字串過濾或模糊匹配）。
    pub(crate) filter_mode: FilterMode,
    /// 目前實際顯示在列表中的項目索引。
    pub(crate) visible_indices: Vec<usize>,
    /// 是否顯示以 `.` 開頭的隱藏檔案與資料夾。
    pub(crate) show_hidden: bool,
    /// 目前使用中的排序模式。
    pub(crate) sort_mode: SortMode,
    /// 目前是否用 linemode 覆蓋右側欄位顯示內容。
    pub(crate) line_mode: Option<LineMode>,
    /// 隨機排序時使用的種子，讓每次重新套用時都能洗牌。
    pub(crate) random_seed: u64,
    /// 目前這個 panel 是否以放大的 preview 取代檔案列表。
    ///
    /// 這個開關必須跟著 `PaneState` 保存，不能放在 `App` 的全域欄位；否則在第二個
    /// panel 打開 preview 時，第一個 panel 的 preview 會被同一個全域值覆蓋。
    pub(crate) preview_active: bool,
    /// 雙欄預覽是否已開啟（若開啟且寬度足夠，畫面呈現左清單、右預覽）。
    pub(crate) preview_open: bool,
    /// 雙欄預覽開啟時，焦點是否正處於右側預覽（true 為預覽操作，false 為清單操作）。
    pub(crate) preview_focused: bool,
    /// 預覽是否處於 VCS Diff 差異模式（true 為顯示 Git/SVN 變更差異，false 為檔案全文）。
    pub(crate) preview_diff_mode: bool,
    /// VCS Diff 預覽行快取，避免滾動時反覆調用外部 git/svn 指令。
    pub(crate) preview_diff_cache: PreviewDiffCache,
    /// 目前 preview 在內容中的捲動偏移量。
    pub(crate) preview_scroll: usize,
    /// 目前 preview 游標所在行號（0-indexed）。
    pub(crate) preview_cursor: usize,
    /// 目前 preview 區實際可顯示的欄位寬度與列數，供縮圖縮放與捲動邏輯計算上下界。
    pub(crate) preview_viewport_width: usize,
    pub(crate) preview_viewport_height: usize,
    /// 圖片預覽快取，避免每幀重新讀檔與解碼。
    pub(crate) preview_image_cache: Arc<Mutex<Option<ImagePreviewCache>>>,
    /// 程式碼與檔案內容預覽快取，支援多項目 LRU 與鄰近預熱。
    pub(crate) preview_content_cache: Arc<Mutex<PreviewContentCacheMap>>,
    /// 背景非同步圖片解碼任務。
    pub(crate) preview_image_loader: Arc<Mutex<Option<ImageLoader>>>,
    /// 目前 preview 內搜尋使用的查詢字串。
    pub(crate) preview_search_query: Option<String>,
    /// 目前 preview 搜尋命中的定位列，用來標示 n/p 目前停在哪一個結果。
    pub(crate) preview_current_match: Option<usize>,
    /// 目前列表內 find-next 使用的查詢字串。
    pub(crate) list_find_query: Option<String>,
    /// 目前在這個 pane 中已被標記的項目路徑。
    pub(crate) marked_paths: BTreeSet<PathBuf>,
    /// 記錄上一次 filter 的命中集合，讓連續輸入時可只在較小候選集內重新比對。
    pub(crate) filter_cache: Option<FilterCache>,
    /// 每次 entries 順序或內容變動都遞增，避免沿用失效的 filter cache。
    pub(crate) entry_revision: u64,
    /// 目前 pane 所在目錄的版本控制（Git / SVN）資訊與檔案狀態。
    pub(crate) vcs_info: Option<Arc<VcsRepoInfo>>,
}

#[cfg(test)]
#[path = "../tests/pane_test.rs"]
mod tests;
