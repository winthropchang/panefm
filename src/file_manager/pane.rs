//! 單一 panel 的目錄列表、排序、選取、預覽與檔案操作狀態。
//!
//! `PaneState` 是 PaneFM 的第一級物件：每個 split 都有獨立 cwd、游標、filter、
//! preview 與顯示模式。這一層不決定快捷鍵，也不繪製 popup；它提供可測試的資料
//! 操作給 `App`，並在變更檔案後重新載入列表與盡可能保留游標位置。

use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListState,
};

use crate::theme::Theme;

use super::{
    bookmark::BookmarkTarget, entry::FileEntry, fuzzy::fuzzy_matched_indices,
    search::GlobalSearchEntry, trash::TrashStore,
};

/// 產生同一程序內不重複的暫存檔序號，避免同時複製多個項目時互相覆蓋。
static TRANSFER_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 目錄傳輸使用的固定檔案 worker 數量。
///
/// 檔案複製主要受儲存裝置與網路 share 限制，worker 太多反而會讓 macOS APFS、
/// Windows redirector 或 SMB server 產生額外 seek、鎖定與 metadata 競爭。固定三條 worker
/// 可讓一條處理較慢檔案時，另外兩條繼續消化小檔案，同時避免對網路磁碟過度併發。
const COPY_FILE_WORKERS: usize = 3;

/// 單檔達到這個大小後，才值得建立額外 thread 輪詢目的檔大小。
///
/// 小檔案若每一筆都建立 thread，像 Rust `target` 這類含數萬個小檔案的目錄會把
/// 大部分時間花在線程建立與排程，而不是實際 I/O。小於門檻的檔案直接使用原生 copy，
/// 完成後一次回報大小；大檔案才每 200ms 更新進度。
const PROGRESSIVE_NATIVE_COPY_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

/// SMB 或其他檔案系統不支援平台原生 copy 時，分塊傳輸使用的 buffer 大小。
///
/// 1 MiB 可降低網路 share 上大量小 read/write system call 的成本，同時不會讓每一條
/// file worker 占用過多記憶體。
const STREAM_COPY_BUFFER_BYTES: usize = 1024 * 1024;

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
    /// 目前 preview 在內容中的捲動偏移量。
    pub(crate) preview_scroll: usize,
    /// 目前 preview 區實際可顯示的列數，供捲動邏輯計算上下界。
    pub(crate) preview_viewport_height: usize,
    /// 目前 preview 內搜尋使用的查詢字串。
    pub(crate) preview_search_query: Option<String>,
    /// 目前 preview 搜尋命中的定位列，用來標示 n/p 目前停在哪一個結果。
    pub(crate) preview_current_match: Option<usize>,
    /// 目前列表內 find-next 使用的查詢字串。
    pub(crate) list_find_query: Option<String>,
    /// 目前在這個 pane 中已被標記的項目路徑。
    pub(crate) marked_paths: BTreeSet<PathBuf>,
    /// 記錄上一次 filter 的命中集合，讓連續輸入時可只在較小候選集內重新比對。
    filter_cache: Option<FilterCache>,
    /// 每次 entries 順序或內容變動都遞增，避免沿用失效的 filter cache。
    entry_revision: u64,
}

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
struct FilterCache {
    query: String,
    is_fuzzy: bool,
    show_hidden: bool,
    entry_revision: u64,
    matched_indices: Vec<usize>,
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

impl PaneState {
    /// 建立一個新的 pane 狀態，並立即載入指定目錄內容。
    ///
    /// 參數：
    /// - `cwd: PathBuf`，這個 pane 啟動時要顯示的目錄。
    ///
    /// 回傳：`io::Result<PaneState>`。
    /// - 成功時回傳已載入目錄內容的 pane。
    /// - 失敗時回傳讀取目錄時發生的 I/O 錯誤。
    pub(crate) fn new(cwd: PathBuf) -> io::Result<Self> {
        let random_seed = seed_from_path(&cwd);
        let mut pane = Self {
            bookmark_target: BookmarkTarget::LocalPath(cwd.clone()),
            cwd,
            entries: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            list_viewport_height: 1,
            filter_query: None,
            filter_mode: FilterMode::Normal,
            visible_indices: Vec::new(),
            show_hidden: false,
            sort_mode: SortMode::Natural { reverse: false },
            line_mode: None,
            random_seed,
            preview_active: false,
            preview_scroll: 0,
            preview_viewport_height: 4,
            preview_search_query: None,
            preview_current_match: None,
            list_find_query: None,
            marked_paths: BTreeSet::new(),
            filter_cache: None,
            entry_revision: 0,
        };
        pane.reload()?;
        Ok(pane)
    }

    /// 判斷目前 panel 是否正在顯示放大的 preview。
    ///
    /// 參數：無。
    ///
    /// 回傳：`bool`；`true` 代表 preview 已取代這個 panel 的檔案列表，`false`
    /// 代表顯示一般列表。每個 `PaneState` 都有自己的值，因此切換 panel 不會互相影響。
    pub(crate) fn is_preview_active(&self) -> bool {
        self.preview_active
    }

    /// 明確設定目前 panel 的 preview 顯示狀態。
    ///
    /// 參數：
    /// - `active: bool`，`true` 表示打開 preview，`false` 表示回到一般列表。
    ///
    /// 回傳：`()`；只更新目前 `PaneState`，不會修改其他 panel。
    pub(crate) fn set_preview_active(&mut self, active: bool) {
        self.preview_active = active;
    }

    /// 切換目前 panel 的 preview 顯示狀態，並回傳切換後的結果。
    ///
    /// 參數：無。
    ///
    /// 回傳：`bool`；`true` 代表切換後已打開 preview，`false` 代表切換後已關閉。
    pub(crate) fn toggle_preview_active(&mut self) -> bool {
        self.preview_active = !self.preview_active;
        self.preview_active
    }

    /// 重新掃描目前目錄，並同步更新列表與游標位置。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane 狀態。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表目錄內容已重新載入。
    /// - 失敗時代表讀目錄過程發生 I/O 錯誤。
    pub(crate) fn reload(&mut self) -> io::Result<()> {
        // 外部程式可能在游標前方新增或刪除項目。先記住實際路徑，重新排序後再找回
        // 同一項，避免 watcher 更新列表時游標只依舊索引而跳到另一個檔案。
        let selected_path = self.selected_entry().map(|entry| entry.path.clone());
        let cached_directory_sizes = self
            .entries
            .iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| {
                (
                    entry.path.clone(),
                    (entry.directory_size, entry.directory_size_complete),
                )
            })
            .collect::<BTreeMap<_, _>>();
        self.entries = read_dir_entries(&self.cwd)?;
        self.bump_entry_revision();
        for entry in &mut self.entries {
            if let Some((size, complete)) = cached_directory_sizes.get(&entry.path) {
                entry.directory_size = *size;
                entry.directory_size_complete = *complete;
            }
        }
        self.marked_paths
            .retain(|path| self.entries.iter().any(|entry| &entry.path == path));
        self.sort_entries();
        self.refresh_visible_entries();
        if let Some(path) = selected_path {
            self.select_path(&path);
        }
        Ok(())
    }

    /// 將列表選取游標向上移動指定格數。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被移動游標的 pane。
    /// - `count: usize`，要往上移動的格數。
    ///
    /// 回傳：`()`
    pub(crate) fn move_up_by(&mut self, count: usize) {
        if self.visible_indices.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(count.max(1));
        self.list_state.select(Some(self.selected));
        self.preview_scroll = 0;
    }

    /// 將列表選取游標向下移動指定格數。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被移動游標的 pane。
    /// - `count: usize`，要往下移動的格數。
    ///
    /// 回傳：`()`
    pub(crate) fn move_down_by(&mut self, count: usize) {
        if self.visible_indices.is_empty() {
            return;
        }
        self.selected =
            (self.selected + count.max(1)).min(self.visible_indices.len().saturating_sub(1));
        self.list_state.select(Some(self.selected));
        self.preview_scroll = 0;
    }

    /// 將列表選取游標跳到最上方。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn move_top(&mut self) {
        if self.visible_indices.is_empty() {
            return;
        }
        self.selected = 0;
        self.list_state.select(Some(self.selected));
        self.preview_scroll = 0;
    }

    /// 將列表選取游標跳到最下方。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn move_bottom(&mut self) {
        if self.visible_indices.is_empty() {
            return;
        }
        self.selected = self.visible_indices.len() - 1;
        self.list_state.select(Some(self.selected));
        self.preview_scroll = 0;
    }

    /// 更新列表區目前實際可顯示的列數，供半頁移動等行為使用。
    ///
    /// 參數：
    /// - `height: usize`，扣掉邊框後目前列表區可視的列數。
    ///
    /// 回傳：`()`
    pub(crate) fn set_list_viewport_height(&mut self, height: usize) {
        self.list_viewport_height = height.max(1);
    }

    /// 依照目前列表 viewport 高度向下移動半頁。
    ///
    /// 回傳：實際採用的步長。
    pub(crate) fn page_down(&mut self) -> usize {
        let step = (self.list_viewport_height / 2).max(1);
        self.move_down_by(step);
        step
    }

    /// 依照目前列表 viewport 高度向上移動半頁。
    ///
    /// 回傳：實際採用的步長。
    pub(crate) fn page_up(&mut self) -> usize {
        let step = (self.list_viewport_height / 2).max(1);
        self.move_up_by(step);
        step
    }

    /// 依照目前列表 viewport 高度向下移動一整頁。
    ///
    /// 回傳：實際採用的步長。
    pub(crate) fn full_page_down(&mut self) -> usize {
        let step = self.list_viewport_height.max(1);
        self.move_down_by(step);
        step
    }

    /// 依照目前列表 viewport 高度向上移動一整頁。
    ///
    /// 回傳：實際採用的步長。
    pub(crate) fn full_page_up(&mut self) -> usize {
        let step = self.list_viewport_height.max(1);
        self.move_up_by(step);
        step
    }

    /// 將列表選取游標跳到指定的可見索引位置。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane。
    /// - `index: usize`，目標可見索引，超出範圍時會自動夾住。
    ///
    /// 回傳：`()`
    pub(crate) fn move_to_visible_index(&mut self, index: usize) {
        if self.visible_indices.is_empty() {
            return;
        }
        self.selected = index.min(self.visible_indices.len().saturating_sub(1));
        self.list_state.select(Some(self.selected));
        self.preview_scroll = 0;
    }

    /// 取得目前游標指向的檔案項目。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Option<&FileEntry>`。
    /// - `Some(...)` 代表有選取項目。
    /// - `None` 代表目前目錄為空。
    pub(crate) fn selected_entry(&self) -> Option<&FileEntry> {
        self.visible_indices
            .get(self.selected)
            .and_then(|index| self.entries.get(*index))
    }

    /// 判斷指定項目是否已被標記。
    pub(crate) fn is_marked(&self, entry: &FileEntry) -> bool {
        self.marked_paths.contains(&entry.path)
    }

    /// 回傳目前 pane 裡被標記的項目數量。
    pub(crate) fn marked_count(&self) -> usize {
        self.marked_paths.len()
    }

    /// 清除目前 pane 中所有已標記項目。
    pub(crate) fn clear_marks(&mut self) {
        self.marked_paths.clear();
    }

    /// 切換目前游標指向項目的標記狀態。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，目前要切換標記的 pane。
    ///
    /// 回傳：`Option<bool>`。
    /// - `Some(true)` 代表原本未標記，這次已加入標記。
    /// - `Some(false)` 代表原本已標記，這次已取消標記。
    /// - `None` 代表目前沒有任何可切換的選取項目。
    pub(crate) fn toggle_mark_selected(&mut self) -> Option<bool> {
        let entry = self.selected_entry()?.clone();
        if self.marked_paths.remove(&entry.path) {
            Some(false)
        } else {
            self.marked_paths.insert(entry.path);
            Some(true)
        }
    }

    /// 將索引範圍內的項目加入標記集合，並回傳實際新增了多少項目。
    pub(crate) fn mark_range(&mut self, start: usize, end: usize) -> usize {
        if self.visible_indices.is_empty() {
            return 0;
        }

        let range_start = start.min(end);
        let range_end = start
            .max(end)
            .min(self.visible_indices.len().saturating_sub(1));
        let mut added = 0usize;

        for visible_index in range_start..=range_end {
            let Some(entry_index) = self.visible_indices.get(visible_index) else {
                continue;
            };
            let Some(entry) = self.entries.get(*entry_index) else {
                continue;
            };
            if self.marked_paths.insert(entry.path.clone()) {
                added += 1;
            }
        }

        added
    }

    /// 將目前列表中所有可見項目全部加入標記集合，並回傳實際新增數量。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，目前要套用全選的 pane。
    ///
    /// 回傳：`usize`。
    /// - 代表這次全選新加入了多少個標記項目。
    pub(crate) fn mark_all_visible(&mut self) -> usize {
        if self.visible_indices.is_empty() {
            return 0;
        }

        let mut added = 0usize;
        for visible_index in &self.visible_indices {
            let Some(entry) = self.entries.get(*visible_index) else {
                continue;
            };
            if self.marked_paths.insert(entry.path.clone()) {
                added += 1;
            }
        }
        added
    }

    /// 反轉目前所有可見項目的標記狀態。
    ///
    /// 規則：
    /// - 原本已標記的可見項目會被取消。
    /// - 原本未標記的可見項目會被加入標記。
    /// - 不可見項目的標記狀態保持不變。
    ///
    /// 回傳：`(usize, usize)`。
    /// - 第一個值是這次新增的標記數量。
    /// - 第二個值是這次取消的標記數量。
    pub(crate) fn invert_visible_marks(&mut self) -> (usize, usize) {
        let mut added = 0usize;
        let mut removed = 0usize;

        for visible_index in &self.visible_indices {
            let Some(entry) = self.entries.get(*visible_index) else {
                continue;
            };
            if self.marked_paths.remove(&entry.path) {
                removed += 1;
            } else {
                self.marked_paths.insert(entry.path.clone());
                added += 1;
            }
        }

        (added, removed)
    }

    /// 回傳目前應該參與批次操作的項目清單。
    ///
    /// 規則：
    /// - 若已有標記項目，優先回傳所有標記項目。
    /// - 若沒有標記項目，則回傳目前選取項目。
    pub(crate) fn selected_or_marked_entries(&self) -> Vec<FileEntry> {
        if !self.marked_paths.is_empty() {
            self.entries
                .iter()
                .filter(|entry| self.marked_paths.contains(&entry.path))
                .cloned()
                .collect()
        } else {
            self.selected_entry().cloned().into_iter().collect()
        }
    }

    /// 回傳目前列表實際可見的項目，供畫面渲染使用。
    pub(crate) fn visible_entries(&self) -> Vec<&FileEntry> {
        self.visible_indices
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .collect()
    }

    /// 若目前選到的是資料夾，則進入該資料夾。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要切換目錄的 pane。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已完成目錄切換或目前選項不是資料夾。
    /// - 失敗時代表進入目錄或重新載入內容時發生錯誤。
    #[cfg(test)]
    pub(crate) fn enter_selected(&mut self) -> io::Result<()> {
        if let Some(entry) = self.selected_entry().cloned()
            && entry.is_dir
        {
            self.cwd = entry.path.clone();
            self.bookmark_target = match &self.bookmark_target {
                BookmarkTarget::SmbLocation(current_url) => {
                    BookmarkTarget::SmbLocation(append_smb_url_segment(current_url, &entry.name))
                }
                BookmarkTarget::LocalPath(_) => BookmarkTarget::LocalPath(self.cwd.clone()),
            };
            self.selected = 0;
            self.filter_query = None;
            self.reload()?;
        }
        Ok(())
    }

    /// 立即切換到目前選取的子目錄，但不在呼叫端同步讀取新目錄內容。
    ///
    /// 參數：無。
    /// 回傳：`Option<PathBuf>`；成功切換時回傳新 cwd，選到檔案時回傳 `None`。
    /// `App` 會先繪製空列表或快取，再由背景 worker 呼叫 `read_dir_entries`，避免大型
    /// 目錄的數萬筆 metadata 與排序凍結 TUI。
    pub(crate) fn begin_enter_selected(&mut self) -> Option<PathBuf> {
        let entry = self.selected_entry()?.clone();
        if !entry.is_dir {
            return None;
        }
        self.cwd = entry.path;
        self.bookmark_target = match &self.bookmark_target {
            BookmarkTarget::SmbLocation(current_url) => {
                BookmarkTarget::SmbLocation(append_smb_url_segment(current_url, &entry.name))
            }
            BookmarkTarget::LocalPath(_) => BookmarkTarget::LocalPath(self.cwd.clone()),
        };
        self.selected = 0;
        self.filter_query = None;
        self.replace_entries(Vec::new(), None);
        Some(self.cwd.clone())
    }

    /// 立即切換到父目錄，但把目錄讀取交給背景 worker。
    ///
    /// 參數：無。
    /// 回傳：`Option<(PathBuf, PathBuf)>`；依序為父目錄 cwd 與載入完成後應重新選取的
    /// 原目錄。已位於檔案系統根目錄時回傳 `None`。
    pub(crate) fn begin_go_parent(&mut self) -> Option<(PathBuf, PathBuf)> {
        let previous_cwd = self.cwd.clone();
        let parent = self.cwd.parent()?.to_path_buf();
        self.cwd = parent;
        self.bookmark_target = match &self.bookmark_target {
            BookmarkTarget::SmbLocation(current_url) => smb_parent_url(current_url)
                .map(BookmarkTarget::SmbLocation)
                .unwrap_or_else(|| BookmarkTarget::LocalPath(self.cwd.clone())),
            BookmarkTarget::LocalPath(_) => BookmarkTarget::LocalPath(self.cwd.clone()),
        };
        self.filter_query = None;
        self.replace_entries(Vec::new(), None);
        Some((self.cwd.clone(), previous_cwd))
    }

    /// 套用背景讀取或目錄快取提供的完整清單，並依目前 pane 設定重新排序與選取。
    ///
    /// 參數：`entries` 是新 cwd 的項目；`selected_path` 是回到父目錄時要找回的子目錄。
    /// 回傳：`()`；此函數只修改目前 panel，不會觸碰其他 panel 或全域 UI。
    pub(crate) fn replace_entries(
        &mut self,
        entries: Vec<FileEntry>,
        selected_path: Option<&Path>,
    ) {
        self.entries = entries;
        self.bump_entry_revision();
        self.sort_entries();
        self.refresh_visible_entries();
        if let Some(path) = selected_path {
            self.select_path(path);
        }
    }

    /// 套用已在背景完成排序的清單，跳過主 UI 執行緒的排序運算。
    pub(crate) fn replace_entries_presorted(
        &mut self,
        entries: Vec<FileEntry>,
        selected_path: Option<&Path>,
    ) {
        self.entries = entries;
        self.bump_entry_revision();
        self.refresh_visible_entries();
        if let Some(path) = selected_path {
            self.select_path(path);
        }
    }

    /// 增量追加載入中的目錄項目，並在保留目前可見游標索引的前提下即時更新畫面。
    pub(crate) fn extend_entries(&mut self, new_entries: Vec<FileEntry>) {
        self.entries.extend(new_entries);
        self.bump_entry_revision();
        self.refresh_visible_entries();
    }

    /// 回到目前目錄的上一層。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要切換到父目錄的 pane。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已回到父目錄或目前已無父目錄。
    /// - 失敗時代表重新載入父目錄內容時發生錯誤。
    #[cfg(test)]
    pub(crate) fn go_parent(&mut self) -> io::Result<()> {
        let current_dir = self.cwd.clone();
        if let Some(parent) = self.cwd.parent().map(Path::to_path_buf) {
            self.cwd = parent;
            self.bookmark_target = match &self.bookmark_target {
                BookmarkTarget::SmbLocation(current_url) => smb_parent_url(current_url)
                    .map(BookmarkTarget::SmbLocation)
                    .unwrap_or_else(|| BookmarkTarget::LocalPath(self.cwd.clone())),
                BookmarkTarget::LocalPath(_) => BookmarkTarget::LocalPath(self.cwd.clone()),
            };
            self.filter_query = None;
            self.reload()?;
            self.select_path(&current_dir);
        }
        Ok(())
    }

    /// 依照目前選取項目產生預覽區要顯示的文字行。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    /// - `max_lines: usize`，預覽區最多顯示的行數。
    ///
    /// 回傳：`Vec<Line<'static>>`，可直接交給 `ratatui` 的 Paragraph 渲染。
    pub(crate) fn preview_lines(&self, max_lines: usize, theme: Theme) -> Vec<Line<'static>> {
        let max_lines = max_lines.max(1);
        if self.preview_search_query.is_some() {
            return self
                .preview_content_lines(theme)
                .into_iter()
                .skip(self.preview_scroll)
                .take(max_lines)
                .collect();
        }

        let needed_lines = self.preview_scroll + max_lines;
        self.raw_preview_content_lines_limited(needed_lines)
            .into_iter()
            .skip(self.preview_scroll)
            .take(max_lines)
            .collect()
    }

    /// 更新 preview 區目前實際可顯示的列數，供捲動行為使用。
    pub(crate) fn set_preview_viewport_height(&mut self, height: usize) {
        self.preview_viewport_height = height.max(1);
        if self.preview_scroll > 0 || self.preview_search_query.is_some() {
            self.clamp_preview_scroll();
        }
    }

    /// 將 preview 向下捲動指定列數。
    pub(crate) fn scroll_preview_down(&mut self, lines: usize) {
        let max_scroll = self.max_preview_scroll();
        self.preview_scroll = (self.preview_scroll + lines).min(max_scroll);
    }

    /// 將 preview 向上捲動指定列數。
    pub(crate) fn scroll_preview_up(&mut self, lines: usize) {
        self.preview_scroll = self.preview_scroll.saturating_sub(lines);
    }

    /// 將 preview 捲到最上方。
    pub(crate) fn scroll_preview_top(&mut self) {
        self.preview_scroll = 0;
    }

    /// 將 preview 捲到最下方。
    pub(crate) fn scroll_preview_bottom(&mut self) {
        self.preview_scroll = self.max_preview_scroll();
    }

    /// 依照目前 viewport 高度向下翻半頁。
    pub(crate) fn page_preview_down(&mut self) {
        let step = (self.preview_viewport_height / 2).max(1);
        self.scroll_preview_down(step);
    }

    /// 依照目前 viewport 高度向上翻半頁。
    pub(crate) fn page_preview_up(&mut self) {
        let step = (self.preview_viewport_height / 2).max(1);
        self.scroll_preview_up(step);
    }

    /// 依照目前 preview viewport 高度向下翻一整頁。
    pub(crate) fn full_page_preview_down(&mut self) {
        let step = self.preview_viewport_height.max(1);
        self.scroll_preview_down(step);
    }

    /// 依照目前 preview viewport 高度向上翻一整頁。
    pub(crate) fn full_page_preview_up(&mut self) {
        let step = self.preview_viewport_height.max(1);
        self.scroll_preview_up(step);
    }

    /// 判斷目前 preview 是否已經有捲動位置。
    pub(crate) fn has_preview_scroll(&self) -> bool {
        self.preview_scroll > 0
    }

    /// 判斷目前 preview 是否正在套用搜尋條件。
    pub(crate) fn has_preview_search(&self) -> bool {
        self.preview_search_query.is_some()
    }

    /// 取得目前 preview 搜尋字串，供 UI 顯示狀態使用。
    pub(crate) fn preview_search_query(&self) -> Option<&str> {
        self.preview_search_query.as_deref()
    }

    /// 判斷目前 preview 是否還有更多內容可以往下捲動。
    pub(crate) fn preview_has_more_below(&self) -> bool {
        if self.preview_search_query.is_some() {
            return self.preview_scroll < self.max_preview_scroll();
        }

        let viewport_height = self.preview_viewport_height.max(1);
        let needed_lines = self.preview_scroll + viewport_height + 1;
        let total_loaded_lines = self.raw_preview_content_lines_limited(needed_lines).len();
        total_loaded_lines > self.preview_scroll + viewport_height
    }

    /// 回傳完整 preview 內容最多可以向下捲到哪一列。
    fn max_preview_scroll(&self) -> usize {
        let total_lines = self.raw_preview_content_lines().len();
        total_lines.saturating_sub(self.preview_viewport_height.max(1))
    }

    /// 產生目前選取項目的完整 preview 內容，供捲動切片與上下界計算使用。
    fn preview_content_lines(&self, theme: Theme) -> Vec<Line<'static>> {
        let lines = self.raw_preview_content_lines();
        if let Some(query) = self.preview_search_query.as_deref() {
            highlight_preview_matches(lines, query, theme, self.preview_current_match)
        } else {
            lines
        }
    }

    /// 產生目前選取項目的完整 preview 原始內容，不套用任何搜尋高亮。
    fn raw_preview_content_lines(&self) -> Vec<Line<'static>> {
        self.raw_preview_content_lines_limited(usize::MAX)
    }

    /// 針對搜尋列表中的某個結果建立 preview，並自動跳到第一個命中位置。
    pub(crate) fn search_preview_for_entry(
        entry: &GlobalSearchEntry,
        viewport_height: usize,
        query: &str,
        requested_scroll: Option<usize>,
        current_match_line: Option<usize>,
        preview_focused: bool,
        theme: Theme,
    ) -> SearchPreviewData {
        let viewport_height = viewport_height.max(1);
        let match_positions = Self::search_preview_match_positions(&entry.path, query);
        let current_match_line = current_match_line
            .or(entry.match_line_number)
            .unwrap_or_else(|| match_positions.first().copied().unwrap_or(1));
        let effective_scroll = requested_scroll.unwrap_or(current_match_line);
        let visible_lines = build_search_preview_lines(
            &entry.path,
            viewport_height,
            query,
            current_match_line,
            theme,
        );
        let mut title = Self::preview_title_for_path(&entry.path, preview_focused, Some(query));
        if effective_scroll > 1 {
            title.push_str("  ^");
        }
        if match_positions
            .iter()
            .any(|line| *line > current_match_line)
        {
            title.push_str("  v");
        }
        SearchPreviewData {
            title,
            lines: visible_lines,
        }
    }

    /// 回傳指定檔案 preview 中所有命中的列位置，供搜尋 preview 導航使用。
    pub(crate) fn search_preview_match_positions(path: &Path, query: &str) -> Vec<usize> {
        let trimmed = query.trim().to_lowercase();
        if trimmed.is_empty() {
            return Vec::new();
        }

        let Ok(file) = File::open(path) else {
            return Vec::new();
        };
        let reader = BufReader::new(file);
        reader
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let Ok(line) = line else {
                    return None;
                };
                line.to_lowercase().contains(&trimmed).then_some(index + 1)
            })
            .collect()
    }

    /// 依照指定路徑建立 preview 區塊標題。
    pub(crate) fn preview_title_for_path(
        path: &Path,
        preview_focused: bool,
        query: Option<&str>,
    ) -> String {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let mut title = format!("Preview: {name}");
        if preview_focused {
            title.push_str("  [preview]");
        }
        if let Some(query) = query.filter(|query| !query.trim().is_empty()) {
            title.push_str(&format!("  [/{query}]"));
        }
        title
    }

    /// 產生目前選取項目的 preview 原始內容，並限制最多只建立指定行數。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    /// - `max_lines: usize`，最多建立的 preview 行數。
    ///
    /// 回傳：`Vec<Line<'static>>`，未套用搜尋高亮的 preview 原始內容。
    fn raw_preview_content_lines_limited(&self, max_lines: usize) -> Vec<Line<'static>> {
        match self.selected_entry() {
            Some(entry) if entry.is_dir => preview_directory(entry, max_lines),
            Some(entry) => preview_file(&entry.path, max_lines),
            None => vec![Line::from("empty directory")],
        }
    }

    /// 當列表或 viewport 發生變化時，把 preview 捲動位置壓回合法範圍。
    fn clamp_preview_scroll(&mut self) {
        self.preview_scroll = self.preview_scroll.min(self.max_preview_scroll());
    }

    /// 對 preview 內容套用搜尋條件，並跳到第一個命中的結果。
    pub(crate) fn set_preview_search_query(&mut self, query: &str) {
        let trimmed = query.trim();
        self.preview_search_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        if self.preview_search_query.is_some() {
            self.preview_scroll = 0;
            self.preview_current_match = None;
            self.jump_to_preview_match(true);
        } else {
            self.preview_scroll = 0;
            self.preview_current_match = None;
        }
    }

    /// 清除 preview 搜尋條件與其高亮狀態。
    pub(crate) fn clear_preview_search(&mut self) {
        self.preview_search_query = None;
        self.preview_current_match = None;
    }

    /// 判斷目前列表是否有啟用 find-next 搜尋。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`bool`。
    /// - `true` 代表目前列表仍保留 find-next 查詢結果。
    /// - `false` 代表目前沒有啟用列表內搜尋。
    pub(crate) fn has_list_find(&self) -> bool {
        self.list_find_query.is_some()
    }

    /// 取得目前列表內 find-next 的查詢文字。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Option<&str>`。
    /// - `Some(query)` 代表目前有啟用中的查詢。
    /// - `None` 代表目前沒有查詢。
    pub(crate) fn list_find_query(&self) -> Option<&str> {
        self.list_find_query.as_deref()
    }

    /// 設定列表內 find-next 的查詢文字。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要更新搜尋條件的 pane。
    /// - `query: &str`，新的查詢字串；空白字串會視為清除。
    ///
    /// 回傳：`()`
    pub(crate) fn set_list_find_query(&mut self, query: &str) {
        let trimmed = query.trim();
        self.list_find_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_lowercase())
        };
    }

    /// 清除目前列表中的 find-next 結果。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被清除搜尋狀態的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn clear_list_find(&mut self) {
        self.list_find_query = None;
    }

    /// 回傳目前可見列表中所有符合 find-next 的索引位置。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Vec<usize>`。
    /// - 每個值都是目前可見列表中的索引位置，而不是 `entries` 的原始索引。
    pub(crate) fn list_find_match_indices(&self) -> Vec<usize> {
        let Some(query) = self.list_find_query.as_deref() else {
            return Vec::new();
        };

        self.visible_entries()
            .into_iter()
            .enumerate()
            .filter_map(|(visible_index, entry)| {
                entry
                    .name
                    .to_lowercase()
                    .contains(query)
                    .then_some(visible_index)
            })
            .collect()
    }

    /// 回傳目前選取項目在 find-next 命中結果中的順位與總數。
    ///
    /// 參數：
    /// - `self: &PaneState`，目前的 pane 狀態。
    ///
    /// 回傳：`Option<(usize, usize)>`。
    /// - `Some((current, total))` 代表目前選取列就是命中結果之一，且 `current` 為 1-based。
    /// - `None` 代表目前沒有命中結果，或目前游標不在命中項目上。
    pub(crate) fn list_find_match_position(&self) -> Option<(usize, usize)> {
        let matches = self.list_find_match_indices();
        let total = matches.len();
        if total == 0 {
            return None;
        }

        matches
            .iter()
            .position(|index| *index == self.selected)
            .map(|position| (position + 1, total))
    }

    /// 把列表游標移到下一個 find-next 命中項目，找不到時會循環回第一個。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要移動游標的 pane。
    ///
    /// 回傳：`bool`。
    /// - `true` 代表成功跳到某個命中項目。
    /// - `false` 代表目前沒有任何命中。
    pub(crate) fn jump_to_next_list_find_match(&mut self) -> bool {
        let matches = self.list_find_match_indices();
        let Some(target) = matches
            .iter()
            .copied()
            .find(|index| *index > self.selected)
            .or_else(|| matches.first().copied())
        else {
            return false;
        };

        self.selected = target;
        self.list_state.select(Some(target));
        self.preview_scroll = 0;
        true
    }

    /// 把列表游標移到上一個 find-next 命中項目，找不到時會循環回最後一個。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要移動游標的 pane。
    ///
    /// 回傳：`bool`。
    /// - `true` 代表成功跳到某個命中項目。
    /// - `false` 代表目前沒有任何命中。
    pub(crate) fn jump_to_previous_list_find_match(&mut self) -> bool {
        let matches = self.list_find_match_indices();
        let Some(target) = matches
            .iter()
            .rev()
            .copied()
            .find(|index| *index < self.selected)
            .or_else(|| matches.last().copied())
        else {
            return false;
        };

        self.selected = target;
        self.list_state.select(Some(target));
        self.preview_scroll = 0;
        true
    }

    /// 跳到下一個 preview 搜尋命中結果，若已到底則循環回第一個。
    pub(crate) fn jump_to_next_preview_match(&mut self) -> bool {
        self.jump_to_preview_match(true)
    }

    /// 跳到上一個 preview 搜尋命中結果，若已到頂則循環回最後一個。
    pub(crate) fn jump_to_previous_preview_match(&mut self) -> bool {
        self.jump_to_preview_match(false)
    }

    /// 回傳目前 preview 搜尋命中的總數，供狀態列顯示。
    pub(crate) fn preview_match_count(&self) -> usize {
        let Some(query) = self.preview_search_query.as_deref() else {
            return 0;
        };
        let query = query.to_lowercase();
        preview_match_positions(&self.raw_preview_content_lines(), &query).len()
    }

    /// 依照方向跳到 preview 搜尋的下一個或上一個命中位置。
    fn jump_to_preview_match(&mut self, forward: bool) -> bool {
        let Some(query) = self.preview_search_query.as_deref() else {
            return false;
        };

        let query = query.to_lowercase();
        let matches = preview_match_positions(&self.raw_preview_content_lines(), &query);

        let target = if forward {
            self.preview_current_match
                .and_then(|current| current.checked_add(1))
                .filter(|next| *next < matches.len())
                .or(Some(0))
        } else {
            self.preview_current_match
                .and_then(|current| current.checked_sub(1))
                .or_else(|| matches.len().checked_sub(1))
        };
        let Some(target) = target else {
            return false;
        };
        let Some((line_index, _)) = matches.get(target).copied() else {
            return false;
        };

        self.preview_current_match = Some(target);
        self.preview_scroll = line_index.min(self.max_preview_scroll());
        true
    }

    /// 刪除目前選取的檔案或資料夾。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，執行刪除的 pane。
    ///
    /// 回傳：`io::Result<Option<String>>`。
    /// - `Ok(Some(name))` 代表成功刪除，並回傳顯示名稱。
    /// - `Ok(None)` 代表目前沒有可刪除的選取項目。
    /// - `Err(...)` 代表檔案系統操作失敗。
    #[allow(dead_code)]
    pub(crate) fn delete_selected(&mut self) -> io::Result<Option<String>> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(None);
        };

        if entry.is_dir {
            fs::remove_dir_all(&entry.path)?;
        } else {
            fs::remove_file(&entry.path)?;
        }

        let removed_name = entry.display_name();
        self.reload()?;
        Ok(Some(removed_name))
    }

    /// 刪除目前選取項目，或是所有已標記項目。
    ///
    /// 回傳：
    /// - `Ok(vec![...])` 代表成功刪除的顯示名稱清單。
    /// - `Ok(vec![])` 代表目前沒有可刪除項目。
    /// - `Err(...)` 代表檔案系統操作失敗。
    #[allow(dead_code)]
    pub(crate) fn delete_selected_or_marked(&mut self) -> io::Result<Vec<String>> {
        let entries = self.selected_or_marked_entries();
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let mut removed_names = Vec::new();
        for entry in entries {
            if entry.is_dir {
                fs::remove_dir_all(&entry.path)?;
            } else {
                fs::remove_file(&entry.path)?;
            }
            removed_names.push(entry.display_name());
        }

        self.marked_paths.clear();
        self.reload()?;
        Ok(removed_names)
    }

    /// 將目前選取項目，或是所有已標記項目移到 trash。
    ///
    /// 回傳：
    /// - `Ok(vec![...])` 代表成功移入 trash 的顯示名稱清單。
    /// - `Ok(vec![])` 代表目前沒有可處理項目。
    /// - `Err(...)` 代表檔案系統或 trash 寫入操作失敗。
    pub(crate) fn trash_selected_or_marked(
        &mut self,
        trash_store: &TrashStore,
    ) -> io::Result<Vec<String>> {
        let entries = self.selected_or_marked_entries();
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let mut trashed_names = Vec::new();
        for entry in entries {
            let display_name = entry.display_name();
            trash_store.trash_path(&entry.path, &display_name)?;
            trashed_names.push(display_name);
        }

        self.marked_paths.clear();
        self.reload()?;
        Ok(trashed_names)
    }

    /// 重新命名目前選取的檔案或資料夾。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，執行重新命名的 pane。
    /// - `new_name: &str`，新的檔案或資料夾名稱。
    ///
    /// 回傳：`io::Result<Option<String>>`。
    /// - `Ok(Some(name))` 代表成功重新命名，並回傳新的顯示名稱。
    /// - `Ok(None)` 代表目前沒有可重新命名的選取項目。
    /// - `Err(...)` 代表重新命名過程中的檔案系統操作失敗。
    pub(crate) fn rename_selected(&mut self, new_name: &str) -> io::Result<Option<String>> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Ok(None);
        };

        let trimmed_name = new_name.trim();
        if trimmed_name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "new name cannot be empty",
            ));
        }

        let new_path = entry.path.parent().unwrap_or(&self.cwd).join(trimmed_name);
        fs::rename(&entry.path, &new_path)?;
        self.reload()?;

        if let Some(index) = self.visible_indices.iter().position(|visible_index| {
            self.entries
                .get(*visible_index)
                .map(|candidate| candidate.path == new_path)
                .unwrap_or(false)
        }) {
            self.selected = index;
            self.list_state.select(Some(index));
        }

        let renamed_name = if entry.is_dir {
            format!("{trimmed_name}/")
        } else {
            trimmed_name.to_string()
        };
        Ok(Some(renamed_name))
    }

    /// 將外部來源的檔案或資料夾複製到目前 pane 的目錄中。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要接收新項目的目標 pane。
    /// - `source_path: &Path`，原始檔案或資料夾路徑。
    ///
    /// 回傳：`io::Result<String>`。
    /// - 成功時回傳貼上後的顯示名稱。
    /// - 失敗時回傳檔案系統操作錯誤，例如目標已存在或無法讀寫。
    #[cfg(test)]
    pub(crate) fn copy_entry_into_current_dir(&mut self, source_path: &Path) -> io::Result<String> {
        let outcome = copy_path_into_dir(source_path, &self.cwd, false, false)?;
        self.reload()?;

        let pasted_path = outcome.target_path.clone();
        if let Some(index) = self.visible_indices.iter().position(|visible_index| {
            self.entries
                .get(*visible_index)
                .map(|candidate| candidate.path == pasted_path)
                .unwrap_or(false)
        }) {
            self.selected = index;
            self.list_state.select(Some(index));
        }

        Ok(outcome.display_name)
    }

    /// 複製項目並保留建立 Undo 所需的完整結果。
    ///
    /// 參數：`source_path: &Path`，要複製的來源檔案或資料夾。
    /// 回傳：`io::Result<PasteOutcome>`；成功時包含實際目標與可能的覆蓋備份。
    pub(crate) fn copy_entry_with_history(
        &mut self,
        source_path: &Path,
        overwrite: bool,
    ) -> io::Result<PasteOutcome> {
        let outcome = copy_path_into_dir(source_path, &self.cwd, overwrite, true)?;
        self.reload()?;
        self.select_path(&outcome.target_path);
        Ok(outcome)
    }

    /// 計算貼上操作實際會使用的完整目標路徑，但不建立或修改任何檔案。
    ///
    /// 這個方法讓上層在貼上失敗時能顯示真正的 destination，而不是只顯示目標目錄。
    /// 一般貼上若遇到同名項目，結果會包含 `copy` / `copy 2` 等實際名稱；覆蓋貼上則
    /// 回傳原始檔名。計算規則與真正執行 copy/move 的底層共用，避免錯誤訊息誤導使用者。
    ///
    /// 參數：
    /// - `self: &PaneState`，提供目前 pane 的目標目錄。
    /// - `source_path: &Path`，準備貼上的來源檔案或資料夾。
    /// - `overwrite: bool`，是否使用無條件覆蓋規則。
    ///
    /// 回傳：`io::Result<PathBuf>`。
    /// - 成功時回傳本次操作預計使用的完整目標路徑。
    /// - 失敗時代表來源路徑沒有可用檔名，無法建立目標路徑。
    pub(crate) fn planned_paste_target(
        &self,
        source_path: &Path,
        overwrite: bool,
    ) -> io::Result<PathBuf> {
        let file_name = source_path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
        })?;
        target_path_for_paste(source_path, &self.cwd, file_name, overwrite)
    }

    /// 在沒有借用 panel 狀態時，計算來源貼到指定目錄後的預定完整路徑。
    ///
    /// 參數：`source_path: &Path` 為來源；`target_dir: &Path` 為目的目錄；
    /// `overwrite: bool` 表示是否沿用原檔名覆蓋。
    /// 回傳：`io::Result<PathBuf>`，可供背景 paste 工作建立錯誤訊息與實際傳輸。
    pub(crate) fn planned_paste_target_in_dir(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
    ) -> io::Result<PathBuf> {
        let file_name = source_path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
        })?;
        target_path_for_paste(source_path, target_dir, file_name, overwrite)
    }

    /// 在背景工作中複製路徑，並於每次寫入資料後回報新增完成的 byte 數。
    ///
    /// 參數：`source_path: &Path` 為來源；`target_dir: &Path` 為目的目錄；
    /// `overwrite: bool` 表示是否覆蓋；`progress: &mut F` 接收目標可見、發現大小與完成量。
    /// 回傳：`io::Result<PasteOutcome>`；不重新載入 panel，適合 worker thread 使用。
    pub(crate) fn copy_path_to_dir_with_history_progress<F>(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
        progress: &mut F,
    ) -> io::Result<PasteOutcome>
    where
        F: FnMut(TransferProgress),
    {
        copy_path_into_dir_with_progress(source_path, target_dir, overwrite, true, progress)
    }

    /// 在背景工作中移動路徑；跨磁碟或 SMB 無法 rename 時改採 copy 後刪除來源。
    ///
    /// 參數：`source_path: &Path` 為來源；`target_dir: &Path` 為目的目錄；
    /// `overwrite: bool` 表示是否覆蓋；`progress: &mut F` 接收傳輸排程與 byte 進度。
    /// 回傳：`io::Result<PasteOutcome>`；同磁碟 rename 會立即完成，跨裝置則可持續回報。
    pub(crate) fn move_path_to_dir_with_history_progress<F>(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
        progress: &mut F,
    ) -> io::Result<PasteOutcome>
    where
        F: FnMut(TransferProgress),
    {
        Self::move_path_to_dir_with_history_progress_using_rename(
            source_path,
            target_dir,
            overwrite,
            progress,
            |source, target| fs::rename(source, target),
        )
    }

    /// 以可替換的原生 rename 執行背景 move，讓 fallback 規則可被單元測試完整覆蓋。
    ///
    /// 參數：`source_path`、`target_dir`、`overwrite` 與 `progress` 和公開入口相同；
    /// `rename_source` 型別為 `FnOnce(&Path, &Path) -> io::Result<()>`，只負責把來源移到
    /// 最終目標。回傳：`io::Result<PasteOutcome>`；rename 失敗時會改用
    /// 原生 copy，驗證成功後才刪除來源。
    fn move_path_to_dir_with_history_progress_using_rename<F, R>(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
        progress: &mut F,
        rename_source: R,
    ) -> io::Result<PasteOutcome>
    where
        F: FnMut(TransferProgress),
        R: FnOnce(&Path, &Path) -> io::Result<()>,
    {
        // 一般檔案可用單次 metadata 提供完成量；目錄不可在 rename 前遞迴掃描，否則
        // 原本應瞬間完成的同磁碟 move 會因數萬個子項目而先卡住數秒。
        let source_file_size = fs::metadata(source_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len());
        match move_path_into_dir_with_source_rename(
            source_path,
            target_dir,
            overwrite,
            true,
            rename_source,
        ) {
            Ok(outcome) => {
                if let Some(source_file_size) = source_file_size {
                    progress(TransferProgress::BytesDiscovered(source_file_size));
                    progress(TransferProgress::BytesCopied(source_file_size));
                }
                Ok(outcome)
            }
            Err(_rename_error) => {
                let outcome = copy_path_into_dir_with_progress(
                    source_path,
                    target_dir,
                    overwrite,
                    true,
                    progress,
                )?;
                remove_existing_target(source_path)?;
                Ok(outcome)
            }
        }
    }

    /// 移動項目並保留建立 Undo 所需的完整結果。
    ///
    /// 參數：`source_path: &Path`，要移動的來源檔案或資料夾；`overwrite: bool`，是否覆蓋。
    /// 回傳：`io::Result<PasteOutcome>`；成功時包含目的路徑與覆蓋前備份。
    pub(crate) fn move_entry_with_history(
        &mut self,
        source_path: &Path,
        overwrite: bool,
    ) -> io::Result<PasteOutcome> {
        let outcome = move_path_into_dir(source_path, &self.cwd, overwrite, true)?;
        self.reload()?;
        self.select_path(&outcome.target_path);
        Ok(outcome)
    }

    /// 將來源移到指定目錄並回傳 Undo 所需結果，不要求目標是目前 panel。
    ///
    /// 參數：`source_path: &Path`，來源路徑；`target_dir: &Path`，目的目錄。
    /// 回傳：`io::Result<PasteOutcome>`，成功時可直接寫入操作歷史。
    pub(crate) fn move_path_to_dir_with_history(
        source_path: &Path,
        target_dir: &Path,
    ) -> io::Result<PasteOutcome> {
        move_path_into_dir(source_path, target_dir, false, true)
    }

    /// 依照輸入路徑建立新項目。
    ///
    /// 規則：
    /// - 結尾有 `/` 代表建立資料夾。
    /// - 沒有 `/` 結尾代表建立檔案。
    /// - 中間若包含路徑分隔符，會先建立缺少的父目錄。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要建立項目的目標 pane。
    /// - `input: &str`，使用者輸入的建立路徑。
    ///
    /// 回傳：`io::Result<String>`。
    /// - 成功時回傳新項目的顯示名稱。
    /// - 失敗時回傳名稱無效、已存在或建立失敗的錯誤。
    pub(crate) fn create_entry(&mut self, input: &str) -> io::Result<String> {
        let request = parse_create_input(input)?;
        let new_path = self.cwd.join(&request.relative_path);

        if request.is_directory {
            fs::create_dir_all(
                new_path
                    .parent()
                    .filter(|parent| *parent != self.cwd.as_path())
                    .unwrap_or(&self.cwd),
            )?;
            fs::create_dir(&new_path)?;
        } else {
            if let Some(parent) = new_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&new_path)?;
        }

        self.reload()?;
        let focus_path = if request.relative_path.components().count() > 1 {
            match request.relative_path.components().next() {
                Some(Component::Normal(first)) => self.cwd.join(first),
                _ => new_path.clone(),
            }
        } else {
            new_path.clone()
        };
        self.select_path(&focus_path);

        Ok(request.display_name)
    }

    /// 將選取狀態移到指定路徑，方便在建立或貼上後立刻聚焦新項目。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要更新選取狀態的 pane。
    /// - `path: &Path`，希望聚焦的新項目路徑。
    ///
    /// 回傳：`()`
    pub(crate) fn select_path(&mut self, path: &Path) {
        if let Some(index) = self.visible_indices.iter().position(|visible_index| {
            self.entries
                .get(*visible_index)
                .map(|candidate| candidate.path == path)
                .unwrap_or(false)
        }) {
            self.selected = index;
            self.list_state.select(Some(index));
        }
        self.preview_scroll = 0;
    }

    /// 套用新的 filter 字串與模式，並立即更新可見清單。
    pub(crate) fn set_filter_query(&mut self, query: &str, mode: FilterMode) {
        let trimmed = query.trim();
        self.filter_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        self.filter_mode = mode;
        self.refresh_visible_entries();
    }

    /// 清除目前的 filter，恢復成顯示全部項目。
    pub(crate) fn clear_filter(&mut self) {
        self.filter_query = None;
        self.refresh_visible_entries();
    }

    /// 將 pane 切換到指定路徑所在的目錄，並把游標聚焦到該項目。
    ///
    /// 參數：
    /// - `path: &Path`，要在列表中顯示並選中的目標路徑。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表 pane 已切到正確目錄並聚焦項目。
    /// - 失敗時代表重新載入目錄內容時發生 I/O 錯誤。
    pub(crate) fn reveal_path(&mut self, path: &Path) -> io::Result<()> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };

        self.cwd = parent.to_path_buf();
        self.bookmark_target = BookmarkTarget::LocalPath(self.cwd.clone());
        self.filter_query = None;
        self.reload()?;
        self.select_path(path);
        Ok(())
    }

    /// 將 pane 直接切換到指定路徑；若是目錄就進入該目錄，若是檔案就聚焦其所在位置。
    ///
    /// 參數：
    /// - `path: &Path`，要前往的目標路徑，可以是目錄或檔案。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表 pane 已切換到對應位置。
    /// - 失敗時代表目錄不存在或重新載入內容時發生 I/O 錯誤。
    pub(crate) fn go_to_path(&mut self, path: &Path) -> io::Result<()> {
        if path.is_dir() {
            self.cwd = path.to_path_buf();
            self.bookmark_target = BookmarkTarget::LocalPath(self.cwd.clone());
            self.filter_query = None;
            self.reload()?;
            self.move_top();
            Ok(())
        } else {
            self.reveal_path(path)
        }
    }

    /// 直接覆蓋目前 pane 的可書籤化目標，供 SMB 這類非本機來源在切換完成後回填。
    pub(crate) fn set_bookmark_target(&mut self, target: BookmarkTarget) {
        self.bookmark_target = target;
    }

    /// 判斷目前是否仍處於過濾後的列表狀態。
    pub(crate) fn has_active_filter(&self) -> bool {
        self.filter_query.is_some()
    }

    /// 切換目前 pane 是否顯示隱藏檔。
    pub(crate) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh_visible_entries();
    }

    /// 直接設定目前 pane 是否顯示隱藏檔，而不是做切換。
    ///
    /// 參數：
    /// - `show_hidden: bool`，`true` 表示顯示隱藏檔，`false` 表示隱藏。
    ///
    /// 回傳：`()`
    pub(crate) fn set_show_hidden(&mut self, show_hidden: bool) {
        self.show_hidden = show_hidden;
        self.refresh_visible_entries();
    }

    /// 切換到下一個排序模式，並立即重排目前列表。
    pub(crate) fn set_sort_mode(&mut self, sort_mode: SortMode) {
        self.sort_mode = sort_mode;
        self.line_mode = None;
        if matches!(sort_mode, SortMode::Random) {
            self.random_seed = self.random_seed.wrapping_add(1);
        }
        self.sort_entries();
        self.refresh_visible_entries();
    }

    /// 設定目前 pane 的 linemode，只改變右側欄位顯示，不改變排序順序。
    ///
    /// 參數：
    /// - `line_mode: LineMode`，要套用的 linemode。
    ///
    /// 回傳：`()`
    pub(crate) fn set_line_mode(&mut self, line_mode: LineMode) {
        self.line_mode = Some(line_mode);
    }

    /// 將未曾計算容量的直接子目錄初始化為預估中的 ~0B。
    ///
    /// 參數：`self: &mut PaneState`，目前要補齊顯示資料的 pane。
    /// 回傳：`() `；已有容量資料的子目錄會完整保留既有數值，不重設為 0。
    pub(crate) fn init_directory_sizes_if_missing(&mut self) {
        for entry in &mut self.entries {
            if entry.is_dir && entry.directory_size.is_none() {
                entry.directory_size = Some(0);
                entry.directory_size_complete = false;
            }
        }
    }

    /// 清除目錄大小快取，準備接收新的背景遞迴掃描結果。
    ///
    /// 參數：`self: &mut PaneState`，目前要補齊顯示資料的 pane。
    /// 回傳：`() `；只改記憶體狀態，不執行任何同步檔案系統 I/O。
    pub(crate) fn clear_directory_sizes(&mut self) {
        for entry in &mut self.entries {
            if entry.is_dir {
                entry.directory_size = Some(0);
                entry.directory_size_complete = false;
            }
        }
    }

    /// 套用背景 worker 對單一直接子目錄計算出的遞迴大小。
    ///
    /// 參數：`path` 是目前 pane 直接子項目的完整路徑；`size` 是已統計 byte；
    /// `complete` 表示該子樹是否已全部走訪完成。
    /// 回傳：`bool`；找到對應項目並更新時為 `true`，路徑已離開列表時為 `false`。
    pub(crate) fn update_directory_size(&mut self, path: &Path, size: u64, complete: bool) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_dir && entry.path == path)
        else {
            return false;
        };
        entry.directory_size = Some(size);
        entry.directory_size_complete = complete;
        true
    }

    /// 回傳右側欄位目前實際應顯示的資料種類。
    ///
    /// 回傳：
    /// - 若目前已有 linemode，就優先使用 linemode。
    /// - 否則退回排序模式預設的右側欄位。
    pub(crate) fn active_detail_kind(&self) -> SortDetailKind {
        self.line_mode
            .map(LineMode::detail_kind)
            .unwrap_or_else(|| self.sort_mode.detail_kind())
    }

    /// 回傳 pane 標題尾端目前應顯示的模式文字。
    ///
    /// 回傳：
    /// - 若目前有 linemode，格式為 `linemode: ...`。
    /// - 否則顯示 `sort: ...`。
    pub(crate) fn title_mode_label(&self) -> String {
        match self.line_mode {
            Some(line_mode) => format!("linemode: {}", line_mode.label()),
            None => format!("sort: {}", self.sort_mode.label()),
        }
    }

    /// 重新計算目前實際應該顯示的項目與選取位置。
    fn refresh_visible_entries(&mut self) {
        self.visible_indices = match &self.filter_query {
            Some(query) => {
                let is_fuzzy = matches!(self.filter_mode, FilterMode::Fuzzy);
                let candidates = self.filter_candidates(query, is_fuzzy);
                if is_fuzzy {
                    fuzzy_matched_indices(&candidates, query, |index| {
                        self.entries[*index].name.clone().into()
                    })
                    .into_iter()
                    .map(|matched_index| candidates[matched_index])
                    .collect()
                } else {
                    candidates
                        .into_iter()
                        .filter(|index| normal_filter_matches(&self.entries[*index].name, query))
                        .collect()
                }
            }
            None => {
                self.filter_cache = None;
                self.base_visible_candidates()
            }
        };

        if let Some(query) = &self.filter_query {
            let is_fuzzy = matches!(self.filter_mode, FilterMode::Fuzzy);
            self.filter_cache = Some(FilterCache {
                query: query.clone(),
                is_fuzzy,
                show_hidden: self.show_hidden,
                entry_revision: self.entry_revision,
                matched_indices: self.visible_indices.clone(),
            });
        }

        if self.visible_indices.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
        } else {
            self.selected = self
                .selected
                .min(self.visible_indices.len().saturating_sub(1));
            self.list_state.select(Some(self.selected));
        }
        self.preview_scroll = 0;
    }

    /// 依照目前排序模式重排完整項目列表。
    fn sort_entries(&mut self) {
        sort_file_entries(&mut self.entries, self.sort_mode, self.random_seed);
        self.bump_entry_revision();
    }

    fn base_visible_candidates(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.show_hidden || !is_hidden_name(&entry.name))
            .map(|(index, _)| index)
            .collect()
    }

    fn filter_candidates(&self, query: &str, is_fuzzy: bool) -> Vec<usize> {
        if let Some(cache) = &self.filter_cache {
            let is_incremental_narrowing =
                query.starts_with(&cache.query) && query.len() > cache.query.len();
            if is_incremental_narrowing
                && cache.is_fuzzy == is_fuzzy
                && cache.show_hidden == self.show_hidden
                && cache.entry_revision == self.entry_revision
            {
                return cache.matched_indices.clone();
            }
        }
        self.base_visible_candidates()
    }

    fn bump_entry_revision(&mut self) {
        self.entry_revision = self.entry_revision.wrapping_add(1);
        self.filter_cache = None;
    }
}

/// 一般 filter 以空白拆成多個詞，每個詞都必須是檔名的一段連續文字。
/// ASCII 使用零配置的大小寫不敏感比較；非 ASCII 則保留 Unicode 大小寫語意。
fn normal_filter_matches(name: &str, query: &str) -> bool {
    query
        .split_whitespace()
        .all(|term| contains_case_insensitive(name, term))
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_ascii() {
        let needle = needle.as_bytes();
        return haystack
            .as_bytes()
            .windows(needle.len())
            .any(|candidate| candidate.eq_ignore_ascii_case(needle));
    }

    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// 依照指定的排序模式與種子對項目清單進行就地排序（可在背景執行緒執行）。
pub(crate) fn sort_file_entries(entries: &mut [FileEntry], sort_mode: SortMode, random_seed: u64) {
    if entries.len() <= 1 {
        return;
    }

    let comparator = |left: &FileEntry, right: &FileEntry| {
        if matches!(sort_mode, SortMode::Random) {
            match (left.is_dir, right.is_dir) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => random_key(left, random_seed).cmp(&random_key(right, random_seed)),
            }
        } else {
            compare_entries(left, right, sort_mode)
        }
    };

    let worker_count = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);

    if entries.len() < 1000 || worker_count <= 1 {
        entries.sort_unstable_by(comparator);
        return;
    }

    let chunk_size = entries.len().div_ceil(worker_count);
    thread::scope(|scope| {
        for chunk in entries.chunks_mut(chunk_size) {
            scope.spawn(move || {
                chunk.sort_unstable_by(comparator);
            });
        }
    });

    // 多路合併（Pairwise Merge）
    let mut step = chunk_size;
    let mut scratch = Vec::with_capacity(entries.len());
    while step < entries.len() {
        scratch.clear();
        let mut i = 0;
        while i < entries.len() {
            let mid = (i + step).min(entries.len());
            let end = (i + 2 * step).min(entries.len());
            if mid < end {
                let left_slice = &entries[i..mid];
                let right_slice = &entries[mid..end];
                let mut l = 0;
                let mut r = 0;
                while l < left_slice.len() && r < right_slice.len() {
                    if comparator(&left_slice[l], &right_slice[r]) != Ordering::Greater {
                        scratch.push(left_slice[l].clone());
                        l += 1;
                    } else {
                        scratch.push(right_slice[r].clone());
                        r += 1;
                    }
                }
                scratch.extend_from_slice(&left_slice[l..]);
                scratch.extend_from_slice(&right_slice[r..]);
            } else {
                scratch.extend_from_slice(&entries[i..mid]);
            }
            i = end;
        }
        entries.clone_from_slice(&scratch);
        step *= 2;
    }
}

/// 將子目錄名稱接到現有 `smb://host/share[/path]` URL 後方，供 pane 在 SMB 內導航時更新書籤目標。
fn append_smb_url_segment(base: &str, segment: &str) -> String {
    let encoded = percent_encode_path_segment(segment);
    if base.ends_with('/') {
        format!("{base}{encoded}")
    } else {
        format!("{base}/{encoded}")
    }
}

/// 從 `smb://host/share[/path]` URL 回推上一層；若已在 share 根目錄則回傳 `None`。
fn smb_parent_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    let prefix = "smb://";
    let rest = trimmed.strip_prefix(prefix)?;
    let mut segments = rest.split('/').collect::<Vec<_>>();
    if segments.len() <= 2 {
        return None;
    }
    segments.pop();
    Some(format!("{prefix}{}", segments.join("/")))
}

/// 將路徑片段轉成能安全放進 SMB URL 的最小百分比編碼格式。
fn percent_encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }
    encoded
}

/// 判斷檔名是否屬於隱藏檔或隱藏資料夾。
fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// 依照 pane 目前設定的排序模式重排清單，並維持資料夾優先。
fn compare_entries(left: &FileEntry, right: &FileEntry, sort_mode: SortMode) -> Ordering {
    match (left.is_dir, right.is_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => {
            let primary = match sort_mode {
                SortMode::Alphabetical { reverse } => compare_with_reverse(
                    compare_ascii_case_insensitive(&left.name, &right.name),
                    reverse,
                ),
                SortMode::Natural { reverse } => {
                    compare_with_reverse(natural_cmp(&left.name, &right.name), reverse)
                }
                SortMode::Size { reverse } => {
                    compare_with_reverse(left.size.cmp(&right.size), reverse)
                }
                SortMode::Modified { reverse } => {
                    compare_with_reverse(left.modified.cmp(&right.modified), reverse)
                }
                SortMode::Created { reverse } => {
                    compare_with_reverse(left.created.cmp(&right.created), reverse)
                }
                SortMode::Extension { reverse } => compare_with_reverse(
                    file_extension(left)
                        .cmp(&file_extension(right))
                        .then_with(|| compare_ascii_case_insensitive(&left.name, &right.name)),
                    reverse,
                ),
                SortMode::Random => Ordering::Equal,
            };

            if primary == Ordering::Equal {
                left.name.cmp(&right.name)
            } else {
                primary
            }
        }
    }
}

/// 零記憶體配置的 ASCII 不分大小寫字串比較。
fn compare_ascii_case_insensitive(left: &str, right: &str) -> Ordering {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let mut i = 0;
    while i < left_bytes.len() && i < right_bytes.len() {
        let bl = left_bytes[i].to_ascii_lowercase();
        let br = right_bytes[i].to_ascii_lowercase();
        if bl != br {
            return bl.cmp(&br);
        }
        i += 1;
    }
    left_bytes.len().cmp(&right_bytes.len())
}

/// 依照 reverse 旗標決定是否翻轉比較結果。
fn compare_with_reverse(ordering: Ordering, reverse: bool) -> Ordering {
    if reverse {
        ordering.reverse()
    } else {
        ordering
    }
}

/// 取出檔案副檔名作為小寫字串，資料夾則回傳空字串。
fn file_extension(entry: &FileEntry) -> String {
    if entry.is_dir {
        String::new()
    } else {
        entry
            .path
            .extension()
            .map(|extension| extension.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    }
}

/// 用比較接近自然排序的方式比較兩個名稱，讓數字片段能按數值排序。
fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let (mut left_index, mut right_index) = (0usize, 0usize);

    while left_index < left_bytes.len() && right_index < right_bytes.len() {
        if left_bytes[left_index].is_ascii_digit() && right_bytes[right_index].is_ascii_digit() {
            let left_end = ascii_digit_run_end(left_bytes, left_index);
            let right_end = ascii_digit_run_end(right_bytes, right_index);
            let ordering = compare_ascii_number_runs(
                &left_bytes[left_index..left_end],
                &right_bytes[right_index..right_end],
            );
            if ordering != Ordering::Equal {
                return ordering;
            }
            left_index = left_end;
            right_index = right_end;
            continue;
        }

        // Fast path for ASCII characters (avoids Unicode lookup overhead)
        if left_bytes[left_index].is_ascii() && right_bytes[right_index].is_ascii() {
            let c_left = left_bytes[left_index].to_ascii_lowercase();
            let c_right = right_bytes[right_index].to_ascii_lowercase();
            if c_left != c_right {
                return c_left.cmp(&c_right);
            }
            left_index += 1;
            right_index += 1;
            continue;
        }

        let left_character = left[left_index..]
            .chars()
            .next()
            .expect("index is inside UTF-8 string");
        let right_character = right[right_index..]
            .chars()
            .next()
            .expect("index is inside UTF-8 string");
        let ordering = left_character
            .to_lowercase()
            .cmp(right_character.to_lowercase());
        if ordering != Ordering::Equal {
            return ordering;
        }
        left_index += left_character.len_utf8();
        right_index += right_character.len_utf8();
    }

    (left_bytes.len() - left_index).cmp(&(right_bytes.len() - right_index))
}

/// 找出 ASCII 數字片段的尾端索引，過程不建立暫存字串。
///
/// 參數：`bytes: &[u8]` 是完整 UTF-8 檔名；`start: usize` 必須指向 ASCII 數字。
/// 回傳：`usize`，代表第一個非數字 byte 的索引，或 `bytes.len()`。
fn ascii_digit_run_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    end
}

/// 比較兩段任意長度的 ASCII 數字，不轉成整數，也不配置 heap 記憶體。
///
/// 參數：`left/right: &[u8]` 都只包含 `0..=9`。
/// 回傳：`Ordering`；先比較去掉前置零後的位數，再比較內容，數值相等時較短原始片段
/// 排在前面。這可處理超過 `u64` 的檔名數字，並避免大型目錄排序時大量建立字串。
fn compare_ascii_number_runs(left: &[u8], right: &[u8]) -> Ordering {
    let left_trimmed = left
        .iter()
        .position(|byte| *byte != b'0')
        .map(|index| &left[index..])
        .unwrap_or(&[]);
    let right_trimmed = right
        .iter()
        .position(|byte| *byte != b'0')
        .map(|index| &right[index..])
        .unwrap_or(&[]);

    left_trimmed
        .len()
        .cmp(&right_trimmed.len())
        .then_with(|| left_trimmed.cmp(right_trimmed))
        .then_with(|| left.len().cmp(&right.len()))
}

/// 根據路徑內容產生一個基本種子，供隨機排序使用。
fn seed_from_path(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

/// 使用隨機排序時，依照目前種子為每個項目產生穩定但可變動的排序鍵。
fn random_key(entry: &FileEntry, seed: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    entry.path.hash(&mut hasher);
    hasher.finish()
}

/// 計算指定資料夾內的子項目數量。
///
/// 計算指定資料夾的項目數量，並設有上限以防大型目錄阻塞 TUI 主執行緒。
///
/// 參數：
/// - `path: &Path`，要計算內容數量的資料夾路徑。
///
/// 回傳：`String`，讀取成功時為項目數量（超過 64 時顯示 `64+`），失敗時回傳 `0`。
fn count_items(path: &Path) -> String {
    let Ok(read_dir) = fs::read_dir(path) else {
        return String::from("0");
    };
    let mut count = 0;
    for entry in read_dir {
        if entry.is_ok() {
            count += 1;
            if count > 64 {
                return String::from("64+");
            }
        }
    }
    count.to_string()
}

/// 為資料夾產生較完整的預覽內容，包含路徑、項目數與部分子項目名稱。
///
/// 參數：
/// - `entry: &FileEntry`，目前被預覽的資料夾項目。
/// - `max_lines: usize`，預覽區最多可顯示的列數。
///
/// 回傳：`Vec<Line<'static>>`，可直接渲染的資料夾摘要內容。
fn preview_directory(entry: &FileEntry, max_lines: usize) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(format!("path: {}", entry.path.display())),
        Line::from(format!("items: {}", count_items(&entry.path))),
    ];

    if max_lines <= lines.len() {
        lines.truncate(max_lines);
        return lines;
    }

    let remaining = max_lines.saturating_sub(lines.len()).min(50);
    if remaining == 0 {
        return lines;
    }

    match fs::read_dir(&entry.path) {
        Ok(read_dir) => {
            let mut child_names = Vec::new();
            for child in read_dir {
                if let Ok(child) = child {
                    child_names.push(child.file_name().to_string_lossy().to_string());
                    if child_names.len() >= remaining {
                        break;
                    }
                }
            }

            if child_names.is_empty() {
                lines.push(Line::from("empty directory"));
            } else {
                lines.push(Line::from("contents:"));
                for name in child_names
                    .into_iter()
                    .take(max_lines.saturating_sub(lines.len()))
                {
                    lines.push(Line::from(format!("  {name}")));
                }
            }
        }
        Err(_) => lines.push(Line::from("unable to read directory contents")),
    }

    lines.truncate(max_lines);
    lines
}

/// 讀取指定檔案並產生預覽內容。
///
/// 參數：
/// - `path: &Path`，要預覽的檔案路徑。
/// - `max_lines: usize`，最多要顯示的行數。
///
/// 回傳：`Vec<Line<'static>>`。
/// - 成功時回傳可直接渲染的預覽內容。
/// - 若檔案過大、非文字或無法讀取，則回傳說明訊息。
fn preview_file(path: &Path, max_lines: usize) -> Vec<Line<'static>> {
    let Ok(metadata) = fs::metadata(path) else {
        return vec![Line::from("unable to read metadata")];
    };

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase());

    if metadata.len() > 128 * 1024 {
        return vec![Line::from("preview skipped for files larger than 128 KiB")];
    }

    let Ok(bytes) = fs::read(path) else {
        return vec![Line::from("unable to read file contents")];
    };

    if let Some(image_summary) = preview_image_summary(&bytes, extension.as_deref()) {
        let mut lines = Vec::new();
        lines.extend(image_summary.into_iter().map(Line::from));
        lines.truncate(max_lines);
        return lines;
    }

    match String::from_utf8(bytes) {
        Ok(contents) => {
            let content_lines: Vec<&str> = contents.lines().collect();
            if content_lines.is_empty() {
                return vec![Line::from("[empty file]")];
            }

            let mut lines = Vec::new();
            let truncated = content_lines.len() > max_lines;
            for (index, line) in content_lines.into_iter().take(max_lines).enumerate() {
                lines.push(Line::from(format!("{:>3} {}", index + 1, line)));
            }

            if truncated && !lines.is_empty() {
                let last_index = lines.len() - 1;
                lines[last_index] = Line::from("...");
            }

            lines.truncate(max_lines);
            lines
        }
        Err(_) => {
            let mut lines = Vec::new();
            if let Some(binary_label) = preview_binary_label(extension.as_deref()) {
                lines.push(Line::from(format!("format: {binary_label}")));
            }
            lines.push(Line::from("binary or non-utf8 file"));
            lines.truncate(max_lines);
            lines
        }
    }
}

/// 專門為搜尋結果建立 preview 片段，即使檔案很大也能直接看到命中附近內容。
fn build_search_preview_lines(
    path: &Path,
    viewport_height: usize,
    query: &str,
    current_match_line: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let snippet = read_search_snippet(
        path,
        query,
        current_match_line,
        viewport_height.max(1),
        theme,
    );
    if snippet.is_empty() {
        vec![Line::from("no matching snippet available")]
    } else {
        snippet
    }
}

/// 讀取命中行附近的片段內容，讓搜尋 preview 能直接顯示上下文。
fn read_search_snippet(
    path: &Path,
    query: &str,
    current_match_line: usize,
    snippet_height: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let context_before = snippet_height.saturating_sub(1) / 2;
    let context_after = snippet_height.saturating_sub(context_before + 1);
    let start_line = current_match_line.saturating_sub(context_before).max(1);
    let end_line = current_match_line.saturating_add(context_after);

    reader
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            if line_number < start_line || line_number > end_line {
                return None;
            }
            let Ok(line) = line else {
                return None;
            };
            let numbered = format!("{:>3} {}", line_number, line);
            let current_match_start = if line_number == current_match_line {
                numbered.to_lowercase().find(&query.to_lowercase())
            } else {
                None
            };
            Some(highlight_preview_line(
                numbered,
                &query.to_lowercase(),
                theme,
                line_number == current_match_line,
                current_match_start,
            ))
        })
        .collect()
}

/// 為常見圖片檔案產生摘要資訊，顯示格式與尺寸。
fn preview_image_summary(bytes: &[u8], extension: Option<&str>) -> Option<Vec<String>> {
    let image_info = detect_image_info(bytes, extension)?;
    let mut lines = vec![format!("format: {}", image_info.format)];

    if let Some((width, height)) = image_info.dimensions {
        lines.push(format!("dimensions: {} x {}", width, height));
    }

    lines.push(String::from("image preview is not available in terminal"));
    Some(lines)
}

/// 將命中的搜尋字串套用到 preview 行內容上，讓目前查詢結果更容易辨識。
fn highlight_preview_matches(
    lines: Vec<Line<'static>>,
    query: &str,
    theme: Theme,
    current_match_index: Option<usize>,
) -> Vec<Line<'static>> {
    let lower_query = query.to_lowercase();
    if lower_query.is_empty() {
        return lines;
    }

    let match_positions = preview_match_positions(&lines, &lower_query);
    let current_match = current_match_index.and_then(|index| match_positions.get(index).copied());

    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let text = line.to_string();
            if !is_preview_searchable_line(&text) {
                return Line::from(text);
            }
            highlight_preview_line(
                text,
                &lower_query,
                theme,
                current_match
                    .map(|(line_index, _)| line_index == index)
                    .unwrap_or(false),
                current_match
                    .filter(|(line_index, _)| *line_index == index)
                    .map(|(_, start)| start),
            )
        })
        .collect()
}

/// 計算 preview 中每一個搜尋命中的實際位置。
fn preview_match_positions(lines: &[Line<'static>], lower_query: &str) -> Vec<(usize, usize)> {
    if lower_query.is_empty() {
        return Vec::new();
    }

    let mut positions = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let text = line.to_string();
        if !is_preview_searchable_line(&text) {
            continue;
        }
        let lower_text = text.to_lowercase();
        let mut cursor = 0usize;

        while cursor <= lower_text.len() {
            let Some(found) = lower_text
                .get(cursor..)
                .and_then(|segment| segment.find(lower_query))
            else {
                break;
            };
            let start = cursor + found;
            positions.push((line_index, start));
            cursor = start.saturating_add(lower_query.len().max(1));
        }
    }

    positions
}

/// 判斷這一行是否屬於 preview 中真正可搜尋的內容區。
fn is_preview_searchable_line(text: &str) -> bool {
    let trimmed = text.trim_start();
    let digit_count = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();

    digit_count > 0
        && trimmed
            .chars()
            .nth(digit_count)
            .is_some_and(char::is_whitespace)
}

/// 將單一 preview 文字行轉成帶高亮的 `Line`。
fn highlight_preview_line(
    text: String,
    lower_query: &str,
    theme: Theme,
    is_current_line: bool,
    current_match_start: Option<usize>,
) -> Line<'static> {
    let lower_text = text.to_lowercase();
    if !lower_text.contains(lower_query) {
        return if is_current_line {
            Line::styled(text, theme.preview_current_line_style())
        } else {
            Line::from(text)
        };
    }

    let line_style = if is_current_line {
        theme.preview_current_line_style()
    } else {
        Style::default()
    };
    let match_style = if is_current_line {
        Style::default()
            .bg(theme.preview_current_line_bg)
            .fg(theme.preview_match_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.preview_match_style().add_modifier(Modifier::BOLD)
    };
    let current_match_style = Style::default()
        .bg(theme.preview_match_bg)
        .fg(theme.preview_match_fg)
        .add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut cursor = 0usize;

    while cursor <= lower_text.len() {
        let Some(found) = lower_text
            .get(cursor..)
            .and_then(|segment| segment.find(lower_query))
        else {
            break;
        };
        let start = cursor + found;
        let end = start.saturating_add(lower_query.len());

        if let Some(head) = text.get(cursor..start) {
            if !head.is_empty() {
                spans.push(Span::styled(head.to_string(), line_style));
            }
        }

        if let Some(body) = text.get(start..end) {
            let style = if current_match_start == Some(start) {
                current_match_style
            } else {
                match_style
            };
            spans.push(Span::styled(body.to_string(), style));
        }

        cursor = end;
    }

    if let Some(tail) = text.get(cursor..) {
        if !tail.is_empty() {
            spans.push(Span::styled(tail.to_string(), line_style));
        }
    }

    Line::from(spans)
}

/// 依照副檔名為常見二進位檔案補上格式描述。
fn preview_binary_label(extension: Option<&str>) -> Option<&'static str> {
    match extension.unwrap_or_default() {
        "zip" => Some("zip archive"),
        "pdf" => Some("pdf document"),
        "png" => Some("png image"),
        "jpg" | "jpeg" => Some("jpeg image"),
        "gif" => Some("gif image"),
        "webp" => Some("webp image"),
        _ => None,
    }
}

/// 保存圖片檔案的格式與尺寸資訊，供 preview 區使用。
struct ImageInfo {
    format: &'static str,
    dimensions: Option<(u32, u32)>,
}

/// 從檔案位元組與副檔名推測是否為常見圖片，並嘗試取出尺寸。
fn detect_image_info(bytes: &[u8], extension: Option<&str>) -> Option<ImageInfo> {
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some(ImageInfo {
            format: "png image",
            dimensions: Some((width, height)),
        });
    }

    if bytes.len() >= 10 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some(ImageInfo {
            format: "gif image",
            dimensions: Some((width, height)),
        });
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(ImageInfo {
            format: "webp image",
            dimensions: None,
        });
    }

    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        return Some(ImageInfo {
            format: "jpeg image",
            dimensions: jpeg_dimensions(bytes),
        });
    }

    match extension.unwrap_or_default() {
        "png" => Some(ImageInfo {
            format: "png image",
            dimensions: None,
        }),
        "jpg" | "jpeg" => Some(ImageInfo {
            format: "jpeg image",
            dimensions: None,
        }),
        "gif" => Some(ImageInfo {
            format: "gif image",
            dimensions: None,
        }),
        "webp" => Some(ImageInfo {
            format: "webp image",
            dimensions: None,
        }),
        _ => None,
    }
}

/// 從 JPEG 檔頭中掃描 SOF 區塊，盡量取出圖片尺寸。
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut index = 2usize;

    while index + 8 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }

        let marker = bytes[index + 1];
        index += 2;

        if marker == 0xD8 || marker == 0xD9 {
            continue;
        }

        if index + 2 > bytes.len() {
            break;
        }

        let segment_length = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
        if segment_length < 2 || index + segment_length > bytes.len() {
            break;
        }

        if matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        ) && index + 7 < bytes.len()
        {
            let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]) as u32;
            return Some((width, height));
        }

        index += segment_length;
    }

    None
}

/// 將單一路徑複製到目標資料夾，支援檔案與整個資料夾樹。
///
/// 參數：
/// - `source_path: &Path`，來源檔案或資料夾。
/// - `target_dir: &Path`，貼上目標資料夾。
///
/// 回傳：`io::Result<String>`，成功時回傳可顯示的名稱。
fn copy_path_into_dir(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<PasteOutcome> {
    let file_name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
    })?;
    let target_path = target_path_for_paste(source_path, target_dir, file_name, overwrite)?;
    let backup_path = if overwrite {
        copy_path_transactional_with_backup(source_path, &target_path, true, retain_backup)?
    } else {
        copy_path_direct_with_cleanup(source_path, &target_path, |source, target| {
            if source.is_dir() {
                copy_dir_recursive(source, target)
            } else {
                copy_file_and_verify(source, target)
            }
        })?;
        None
    };

    let display_name = if source_path.is_dir() {
        format!("{}/", target_path_file_name(&target_path))
    } else {
        target_path_file_name(&target_path)
    };
    Ok(PasteOutcome {
        display_name,
        target_path,
        backup_path,
    })
}

/// 將單一路徑複製到目標資料夾，並持續回報實際讀寫完成量。
///
/// 參數與 [`copy_path_into_dir`] 相同；`progress` 接收走訪與實際寫入進度事件。
/// 回傳：`io::Result<PasteOutcome>`；失敗時沿用 partial target 清理與 Undo 備份規則。
fn copy_path_into_dir_with_progress<F>(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
    progress: &mut F,
) -> io::Result<PasteOutcome>
where
    F: FnMut(TransferProgress),
{
    let file_name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
    })?;
    let target_path = target_path_for_paste(source_path, target_dir, file_name, overwrite)?;
    let backup_path = if overwrite {
        let staged_path = unique_transfer_path(&target_path, "part");
        let copy_result = copy_path_with_progress(source_path, &staged_path, progress);
        if let Err(error) = copy_result {
            let _ = remove_transfer_path(&staged_path);
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "copy {} to {} failed: {error}",
                    source_path.display(),
                    target_path.display()
                ),
            ));
        }
        commit_staged_copy(&staged_path, &target_path, true, retain_backup)?
    } else {
        copy_path_direct_with_cleanup(source_path, &target_path, |source, target| {
            copy_path_with_progress(source, target, progress)
        })?;
        None
    };

    let display_name = if source_path.is_dir() {
        format!("{}/", target_path_file_name(&target_path))
    } else {
        target_path_file_name(&target_path)
    };
    Ok(PasteOutcome {
        display_name,
        target_path,
        backup_path,
    })
}

/// 依來源型別選擇可回報進度的單檔或遞迴資料夾複製。
///
/// 參數：`source_path`、`target_path` 為來源與目標；`progress` 接收傳輸進度事件。
/// 回傳：`io::Result<()>`；任一子項目失敗會交由外層清理整批目標。
fn copy_path_with_progress<F>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(TransferProgress),
{
    if source_path.is_dir() {
        copy_dir_recursive_with_progress(source_path, target_path, progress)
    } else {
        let size = fs::metadata(source_path)?.len();
        progress(TransferProgress::BytesDiscovered(size));
        copy_file_native_with_progress(source_path, target_path, &mut |increment| {
            progress(TransferProgress::BytesCopied(increment));
        })
    }
}

/// 直接複製到正式目標，失敗時清除本次建立的部分內容。
///
/// 一般貼上採用這條跨平台原生檔案引擎路徑，
/// 不額外要求 SMB 伺服器允許 rename；目標名稱由上層保證原本不存在。
///
/// 參數：
/// - `source_path: &Path`，來源檔案或資料夾。
/// - `target_path: &Path`，本次新建立的正式目標路徑。
/// - `copy_to_target: F`，實際寫入函數，型別為 `FnOnce(&Path, &Path) -> io::Result<()>`。
///
/// 回傳：`io::Result<()>`；失敗時會盡力移除已建立的部分目標，避免佔用檔名。
fn copy_path_direct_with_cleanup<F>(
    source_path: &Path,
    target_path: &Path,
    copy_to_target: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    if target_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("paste target already exists: {}", target_path.display()),
        ));
    }
    if let Err(error) = copy_to_target(source_path, target_path) {
        let cleanup_result = remove_transfer_path(target_path);
        let cleanup_detail = cleanup_result
            .err()
            .map_or_else(String::new, |cleanup_error| {
                format!(
                    "; WARNING: partial target could not be removed: {} ({cleanup_error})",
                    target_path.display()
                )
            });
        return Err(io::Error::new(
            error.kind(),
            format!(
                "copy {} to {} failed: {error}{cleanup_detail}",
                source_path.display(),
                target_path.display()
            ),
        ));
    }
    Ok(())
}

/// 將單一路徑移動到目標資料夾。
///
/// 參數：
/// - `source_path: &Path`，來源檔案或資料夾。
/// - `target_dir: &Path`，貼上目標資料夾。
///
/// 回傳：`io::Result<String>`，成功時回傳可顯示的名稱。
fn move_path_into_dir(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<PasteOutcome> {
    move_path_into_dir_with_source_rename(
        source_path,
        target_dir,
        overwrite,
        retain_backup,
        |source, target| fs::rename(source, target),
    )
}

/// 使用可替換的來源 rename 實作 move 的原生快速路徑。
///
/// 覆蓋既有目標時，舊內容仍由本函數先移到 Undo backup；`rename_source` 只處理來源
/// 到正式目標的那一步。這種切分讓測試能穩定模擬 Windows／SMB 回傳 unsupported，
/// 不需要依賴測試機器真的掛載另一個檔案系統。
///
/// 參數：前四項與 [`move_path_into_dir`] 相同；`rename_source` 是來源 rename 函數。
/// 回傳：`io::Result<PasteOutcome>`；rename 失敗時會先恢復覆蓋前目標，再回傳原始錯誤。
fn move_path_into_dir_with_source_rename<R>(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
    rename_source: R,
) -> io::Result<PasteOutcome>
where
    R: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let file_name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
    })?;
    let target_path = target_path_for_paste(source_path, target_dir, file_name, overwrite)?;

    let backup_path = (overwrite && target_path.exists())
        .then(|| unique_transfer_path(&target_path, "undo-backup"));
    if let Some(backup_path) = &backup_path {
        fs::rename(&target_path, backup_path)?;
    }
    if let Err(error) = rename_source(source_path, &target_path) {
        if let Some(backup_path) = &backup_path {
            let _ = fs::rename(backup_path, &target_path);
        }
        return Err(error);
    }

    let display_name = if target_path.is_dir() {
        format!("{}/", target_path_file_name(&target_path))
    } else {
        target_path_file_name(&target_path)
    };
    let backup_path = if retain_backup {
        backup_path
    } else {
        if let Some(backup_path) = &backup_path {
            remove_transfer_path(backup_path)?;
        }
        None
    };
    Ok(PasteOutcome {
        display_name,
        target_path,
        backup_path,
    })
}

/// 執行交易式複製，並依需求把覆蓋前備份交給 Undo 歷史保存。
///
/// 參數：
/// - `source_path: &Path`，來源路徑。
/// - `target_path: &Path`，正式目標路徑。
/// - `overwrite: bool`，是否允許覆蓋。
/// - `retain_backup: bool`，成功後是否保留舊目標備份。
///
/// 回傳：`io::Result<Option<PathBuf>>`；有覆蓋且要求保留時回傳備份路徑。
fn copy_path_transactional_with_backup(
    source_path: &Path,
    target_path: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<Option<PathBuf>> {
    let staged_path = unique_transfer_path(target_path, "part");
    let copy_result = if source_path.is_dir() {
        copy_dir_recursive(source_path, &staged_path)
    } else {
        copy_file_and_verify(source_path, &staged_path)
    };
    if let Err(error) = copy_result {
        let _ = remove_transfer_path(&staged_path);
        return Err(io::Error::new(
            error.kind(),
            format!(
                "copy {} to {} failed: {error}",
                source_path.display(),
                target_path.display()
            ),
        ));
    }

    commit_staged_copy(&staged_path, target_path, overwrite, retain_backup)
}

/// 依照是否允許覆蓋，決定貼上時真正要使用的目標路徑。
///
/// 規則：
/// - `overwrite = false` 時，沿用原本的 `copy`, `copy 2` 命名策略。
/// - `overwrite = true` 且來源與目標不是同一個實體時，回傳原始名稱；
///   既有目標要由呼叫端依照複製或移動操作的安全規則處理。
/// - 若來源本來就在同一個目錄，為了避免覆蓋自己，會退回不覆蓋策略。
fn target_path_for_paste(
    source_path: &Path,
    target_dir: &Path,
    original_name: &std::ffi::OsStr,
    overwrite: bool,
) -> io::Result<PathBuf> {
    let original_name = original_name.to_string_lossy();
    let direct_target = target_dir.join(original_name.as_ref());

    let same_location = source_path.parent() == Some(target_dir) && direct_target == source_path;
    if !overwrite || same_location {
        return Ok(unique_target_path(
            target_dir,
            std::ffi::OsStr::new(original_name.as_ref()),
        ));
    }

    Ok(direct_target)
}

/// 先把來源完整複製到目標目錄內的暫存路徑，成功後才切換成正式名稱。
///
/// 參數：
/// - `source_path: &Path`，要複製的來源檔案或資料夾。
/// - `target_path: &Path`，使用者最後應看見的正式目標路徑。
/// - `overwrite: bool`，`true` 代表允許在傳輸完成後替換既有目標。
///
/// 回傳：`io::Result<()>`；成功時正式路徑已完整可用，失敗時不會留下
/// 只有部分內容的正式檔名，並會盡力清除內部暫存路徑。
#[cfg(test)]
fn copy_path_transactional(
    source_path: &Path,
    target_path: &Path,
    overwrite: bool,
) -> io::Result<()> {
    copy_path_transactional_with(source_path, target_path, overwrite, |source, staged| {
        if source.is_dir() {
            copy_dir_recursive(source, staged)
        } else {
            copy_file_and_verify(source, staged)
        }
    })
}

/// 執行可注入複製器的交易式複製核心，讓失敗清理規則可以被單元測試完整驗證。
///
/// 參數：
/// - `source_path: &Path`，來源路徑。
/// - `target_path: &Path`，正式目標路徑。
/// - `overwrite: bool`，是否允許替換既有目標。
/// - `copy_to_staged: F`，把來源寫入暫存路徑的函數，型別為
///   `FnOnce(&Path, &Path) -> io::Result<()>`。
///
/// 回傳：`io::Result<()>`，成功時已提交正式名稱；失敗時保留原目標並清理暫存資料。
#[cfg(test)]
fn copy_path_transactional_with<F>(
    source_path: &Path,
    target_path: &Path,
    overwrite: bool,
    copy_to_staged: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let staged_path = unique_transfer_path(target_path, "part");
    if let Err(error) = copy_to_staged(source_path, &staged_path) {
        let _ = remove_transfer_path(&staged_path);
        return Err(io::Error::new(
            error.kind(),
            format!(
                "copy {} to {} failed: {error}",
                source_path.display(),
                target_path.display()
            ),
        ));
    }

    if let Err(error) = commit_staged_copy(&staged_path, target_path, overwrite, false) {
        let _ = remove_transfer_path(&staged_path);
        return Err(error);
    }

    Ok(())
}

/// 使用平台原生 copy 複製單一檔案，並在完成後確認來源、回報值與目標大小一致。
///
/// 這條路徑在 macOS／Windows 保持單一的 local engine：不依 UNC、
/// `/Volumes` 或一般本機路徑切換成另一套手寫串流，而是統一交給 `std::fs::copy`。
/// 這可讓 Rust 標準函式庫與作業系統處理 SMB redirector、clone 與平台細節。
///
/// 參數：
/// - `source_path: &Path`，來源檔案。
/// - `staged_path: &Path`，要寫入的目標檔；可能是正式名稱或交易式暫存名稱。
///
/// 回傳：`io::Result<()>`；成功代表原生 copy 已返回，且兩端大小一致。
fn copy_file_and_verify(source_path: &Path, staged_path: &Path) -> io::Result<()> {
    copy_file_and_verify_with(source_path, staged_path, |source, target| {
        fs::copy(source, target)
    })
}

/// 使用平台原生 copy 複製檔案，並在 copy 執行期間輪詢目的檔大小更新背景進度。
///
/// 原生 copy 在 blocking worker 中執行，另一個非同步工作定期讀取目的檔
/// metadata；輪詢週期為 200ms，讓大型
/// 檔案在 task 面板中有較即時的百分比，但輪詢本身不接管或改寫資料傳輸。
///
/// 參數：`source_path`、`target_path` 為來源與目標；`progress` 在單檔完整完成後收到
/// 寫入增量。回傳：`io::Result<()>`；原生 copy 或大小驗證失敗時回傳原始 I/O 錯誤。
fn copy_file_native_with_progress<F>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
{
    let expected_size = fs::metadata(source_path)?.len();

    // 小檔案直接在既有 file worker 執行，避免每一筆檔案再建立一條監看 thread。
    // 對含數萬個小檔案的 build 目錄，這個分支是主要效能路徑。
    if expected_size < PROGRESSIVE_NATIVE_COPY_THRESHOLD_BYTES {
        return copy_file_with_native_fallback_known_size(
            source_path,
            target_path,
            expected_size,
            progress,
            |source, target| fs::copy(source, target),
        );
    }

    let mut native_reported = 0u64;
    let native_result = copy_file_native_with_progress_using(
        source_path,
        target_path,
        &mut |increment| {
            native_reported = native_reported.saturating_add(increment);
            progress(increment);
        },
        |source, target| fs::copy(source, target),
    );
    match native_result {
        Ok(()) => Ok(()),
        Err(error) if native_copy_supports_stream_fallback(&error) => {
            // macOS 的 copyfile 與 Windows redirector 在部分 SMB server 會留下 0-byte
            // 目標再回傳 not supported。串流重試前一定先移除，才能使用 create_new
            // 保證不會覆寫其他程序剛建立的檔案。
            remove_partial_file_for_fallback(target_path, &error)?;
            let mut fallback_reported = 0u64;
            copy_file_streaming_with_progress(source_path, target_path, &mut |increment| {
                fallback_reported = fallback_reported.saturating_add(increment);
                // 原生路徑已經把 partial 大小回報給 task，fallback 從零重寫時不能再
                // 重複累加同一段範圍；超過原生已回報量後才繼續增加百分比。
                let previous = fallback_reported.saturating_sub(increment);
                let newly_visible = fallback_reported.saturating_sub(native_reported)
                    - previous.saturating_sub(native_reported);
                if newly_visible > 0 {
                    progress(newly_visible);
                }
            })
        }
        Err(error) => Err(error),
    }
}

/// 先嘗試平台原生 copy，不支援時安全切換到跨平台分塊串流。
///
/// 參數：
/// - `source_path: &Path`，來源檔案。
/// - `target_path: &Path`，尚不存在的目的檔案。
/// - `progress: &mut F`，接收已完整寫入的 byte 增量。
/// - `native_copy: C`，可注入的平台原生 copy；正式環境使用 `std::fs::copy`。
///
/// 回傳：`io::Result<()>`；成功前一定驗證目的大小，原生 API 的一般權限或磁碟錯誤
/// 不會被 fallback 隱藏，只有明確的「不支援」錯誤才會改走串流。
#[cfg(test)]
fn copy_file_with_native_fallback<F, C>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
    native_copy: C,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
    C: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    let expected_size = fs::metadata(source_path)?.len();
    copy_file_with_native_fallback_known_size(
        source_path,
        target_path,
        expected_size,
        progress,
        native_copy,
    )
}

/// 使用呼叫端已取得的來源大小執行原生 copy 與串流 fallback。
///
/// 大量小檔案是目錄 copy 的主要成本；若外層為判斷進度策略已讀過 metadata，這裡不可
/// 再重複開啟來源檔。參數中的 `expected_size: u64` 是外層同一次操作取得的來源大小；
/// 其餘參數是來源、目標、進度 callback 與平台 copy。回傳成功前仍會驗證目的檔大小。
fn copy_file_with_native_fallback_known_size<F, C>(
    source_path: &Path,
    target_path: &Path,
    expected_size: u64,
    progress: &mut F,
    native_copy: C,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
    C: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    if target_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("copy target already exists: {}", target_path.display()),
        ));
    }

    match native_copy(source_path, target_path) {
        Ok(copied_size) if copied_size == expected_size => {
            progress(expected_size);
            Ok(())
        }
        Ok(copied_size) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("incomplete native copy: expected {expected_size} bytes, copied {copied_size}"),
        )),
        Err(error) if native_copy_supports_stream_fallback(&error) => {
            remove_partial_file_for_fallback(target_path, &error)?;
            copy_file_streaming_with_progress(source_path, target_path, progress)
        }
        Err(error) => Err(error),
    }
}

/// 判斷平台原生 copy 的錯誤是否適合改用一般 read/write 串流重試。
///
/// Rust 在 macOS 可能直接傳回 `ENOTSUP`（45），Windows SMB redirector 常見
/// `ERROR_INVALID_FUNCTION`（1）或 `ERROR_NOT_SUPPORTED`（50）。這些錯誤只表示伺服器
/// 不支援原生 copy 加速，不代表一般檔案寫入也失敗。
///
/// 參數：`error: &io::Error`，原生 copy 回傳的錯誤。
/// 回傳：`bool`，只有可安全降級的 unsupported 類型回傳 `true`。
fn native_copy_supports_stream_fallback(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }

    match error.raw_os_error() {
        #[cfg(unix)]
        Some(45) | Some(95) => true,
        #[cfg(windows)]
        Some(1) | Some(50) => true,
        _ => false,
    }
}

/// 清除原生 copy 失敗後可能留下的 0-byte 或 partial 目的檔。
///
/// 參數：`target_path: &Path` 是要重試的目標；`native_error: &io::Error` 是原始錯誤。
/// 回傳：`io::Result<()>`；清理失敗時同時保留原生與清理錯誤，不能在未知 partial
/// 上繼續寫入。
fn remove_partial_file_for_fallback(
    target_path: &Path,
    native_error: &io::Error,
) -> io::Result<()> {
    if !target_path.exists() {
        return Ok(());
    }
    fs::remove_file(target_path).map_err(|cleanup_error| {
        io::Error::new(
            cleanup_error.kind(),
            format!(
                "native copy is unsupported ({native_error}); removing partial target {} failed: {cleanup_error}",
                target_path.display()
            ),
        )
    })
}

/// 使用固定大小 buffer 跨平台串流複製單一檔案並即時回報進度。
///
/// 這是 SMB 不支援平台原生 copy 時的可靠 fallback，不是本機預設路徑。目的檔使用
/// `create_new`，因此不會意外覆寫其他程序在 fallback 前建立的同名項目；寫完會 flush、
/// 關閉 handle，再重新讀 metadata 驗證完整大小。
///
/// 參數：`source_path: &Path`、`target_path: &Path` 為來源與目的；`progress: &mut F`
/// 接收每一塊成功寫入的 byte 數。
/// 回傳：`io::Result<()>`；任何 read/write/flush/驗證錯誤都交由外層交易清理 partial。
fn copy_file_streaming_with_progress<F>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
{
    let mut source = BufReader::with_capacity(STREAM_COPY_BUFFER_BYTES, File::open(source_path)?);
    let expected_size = source.get_ref().metadata()?.len();
    let target_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_path)?;
    let mut target = BufWriter::with_capacity(STREAM_COPY_BUFFER_BYTES, target_file);
    let mut buffer = vec![0u8; STREAM_COPY_BUFFER_BYTES];
    let mut copied = 0u64;

    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        target.write_all(&buffer[..read])?;
        copied = copied.saturating_add(read as u64);
        progress(read as u64);
    }
    target.flush()?;
    drop(target);

    let source_size_after_copy = fs::metadata(source_path)?.len();
    let stored_size = fs::metadata(target_path)?.len();
    if copied != expected_size
        || source_size_after_copy != expected_size
        || stored_size != expected_size
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "incomplete streaming copy: expected {expected_size} bytes, source now {source_size_after_copy}, copied {copied}, stored {stored_size}"
            ),
        ));
    }

    // 權限複製是 best effort：部分 SMB server 可寫內容但拒絕 chmod。資料完整性已經
    // 驗證成功，不應因伺服器不支援 Unix 權限而把有效檔案判定為失敗。
    if let Ok(metadata) = fs::metadata(source_path) {
        let _ = fs::set_permissions(target_path, metadata.permissions());
    }
    Ok(())
}

/// 執行可注入原生 copy 的 progressive copy 核心，供 metadata 輪詢行為做回歸測試。
///
/// 參數：`source_path`、`target_path` 與 `progress` 和公開核心相同；`native_copy` 型別為
/// `FnOnce(&Path, &Path) -> io::Result<u64> + Send`，會在 scoped worker 中執行。
/// 回傳：`io::Result<()>`；copy 進行中依目標檔大小回報增量，結束後驗證完整大小。
fn copy_file_native_with_progress_using<F, C>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
    native_copy: C,
) -> io::Result<()>
where
    F: FnMut(u64) + ?Sized,
    C: FnOnce(&Path, &Path) -> io::Result<u64> + Send,
{
    if target_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("copy target already exists: {}", target_path.display()),
        ));
    }

    let expected_size = fs::metadata(source_path)?.len();
    let mut reported_size = 0u64;
    let copied_size = thread::scope(|scope| -> io::Result<u64> {
        let (done_sender, done_receiver) = mpsc::sync_channel(1);
        scope.spawn(move || {
            // 不能只靠固定 sleep 輪詢 `is_finished()`。小檔案通常在幾毫秒內完成，
            // 若每個檔案仍睡滿 200ms，大型 build 目錄會被人為拖慢到數十分鐘。
            // completion channel 會在 copy 返回時立即喚醒目前執行緒；只有大檔仍在
            // 傳輸時，timeout 才負責定期讀取 metadata 更新百分比。
            let _ = done_sender.send(native_copy(source_path, target_path));
        });

        loop {
            match done_receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let stored_size = fs::metadata(target_path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0)
                        .min(expected_size);
                    if stored_size > reported_size {
                        progress(stored_size - reported_size);
                        reported_size = stored_size;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(io::Error::other("native copy worker panicked"));
                }
            }
        }
    })?;
    let stored_size = fs::metadata(target_path)?.len();
    if copied_size != expected_size || stored_size != expected_size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "incomplete native copy: expected {expected_size} bytes, copied {copied_size}, stored {stored_size}"
            ),
        ));
    }
    if copied_size > reported_size {
        progress(copied_size - reported_size);
    }
    Ok(())
}

/// 執行可注入平台複製器的單檔驗證核心，供不完整寫入情境做回歸測試。
///
/// 參數：
/// - `source_path: &Path`，來源檔案。
/// - `staged_path: &Path`，本次新建立的目標檔案。
/// - `platform_copy: F`，平台複製函數，型別為
///   `FnOnce(&Path, &Path) -> io::Result<u64>`，回傳宣稱已複製的 byte 數。
///
/// 回傳：`io::Result<()>`；只有平台 copy 返回且來源、回報值、目標大小一致才成功。
fn copy_file_and_verify_with<F>(
    source_path: &Path,
    staged_path: &Path,
    platform_copy: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    let expected_size = File::open(source_path)?.metadata()?.len();
    copy_file_and_verify_with_known_size(source_path, staged_path, expected_size, platform_copy)
}

/// 使用已知來源大小驗證平台 copy，避免大量小檔案重複開啟來源。
///
/// 參數：`expected_size: u64` 必須由本次 copy 開始前的來源 metadata 取得；其他參數與
/// [`copy_file_and_verify_with`] 相同。回傳：`io::Result<()>`；平台回報量及落盤目標大小
/// 都正確才成功。目的檔會以 metadata 重新確認，因此 SMB 的 0-byte 假成功仍會被拒絕。
fn copy_file_and_verify_with_known_size<F>(
    source_path: &Path,
    staged_path: &Path,
    expected_size: u64,
    platform_copy: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<u64>,
{
    if staged_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("copy target already exists: {}", staged_path.display()),
        ));
    }

    let copied_size = platform_copy(source_path, staged_path)?;
    let stored_size = fs::metadata(staged_path)?.len();
    if copied_size != expected_size || stored_size != expected_size {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "incomplete copy: expected {expected_size} bytes, copied {copied_size}, stored {stored_size}"
            ),
        ));
    }
    Ok(())
}

/// 將已完成的暫存內容切換成正式名稱，覆蓋時先備份舊目標以便失敗回復。
///
/// 參數：
/// - `staged_path: &Path`，已完整寫入的暫存路徑。
/// - `target_path: &Path`，正式目標路徑。
/// - `overwrite: bool`，是否允許替換既有目標。
///
/// 回傳：`io::Result<()>`；提交失敗時會盡力把舊目標從備份改回原名。
fn commit_staged_copy(
    staged_path: &Path,
    target_path: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<Option<PathBuf>> {
    if target_path.exists() && !overwrite {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("paste target already exists: {}", target_path.display()),
        ));
    }

    let backup_path = target_path
        .exists()
        .then(|| unique_transfer_path(target_path, "backup"));
    if let Some(backup_path) = &backup_path {
        fs::rename(target_path, backup_path)?;
    }

    if let Err(error) = fs::rename(staged_path, target_path) {
        if let Some(backup_path) = &backup_path {
            let _ = fs::rename(backup_path, target_path);
        }
        return Err(io::Error::new(
            error.kind(),
            format!("finish paste to {} failed: {error}", target_path.display()),
        ));
    }

    if let Some(backup_path) = &backup_path
        && !retain_backup
    {
        remove_transfer_path(backup_path)?;
    }
    Ok(backup_path.filter(|_| retain_backup))
}

/// 清除 Undo 歷史不再需要的覆蓋備份。
///
/// 參數：`path: &Path`，由貼上結果交回的隱藏備份路徑。
/// 回傳：`io::Result<()>`；路徑不存在時視為已清理完成。
pub(crate) fn remove_undo_backup(path: &Path) -> io::Result<()> {
    remove_transfer_path(path)
}

/// 在正式目標旁產生不衝突的隱藏暫存路徑。
///
/// 參數：
/// - `target_path: &Path`，正式目標路徑，用來取得相同父目錄。
/// - `role: &str`，暫存用途，例如 `part` 或 `backup`。
///
/// 回傳：`PathBuf`，目前不存在且可供本次傳輸使用的路徑。
fn unique_transfer_path(target_path: &Path, role: &str) -> PathBuf {
    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
    loop {
        let sequence = TRANSFER_TEMP_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
        let candidate = parent.join(format!(
            ".panefm-transfer-{}-{sequence}.{role}",
            std::process::id()
        ));
        if !candidate.exists() {
            return candidate;
        }
    }
}

/// 刪除交易式複製使用的檔案或資料夾暫存路徑。
///
/// 參數：`path: &Path`，要清除的內部暫存路徑。
/// 回傳：`io::Result<()>`；路徑不存在視為已完成清理。
fn remove_transfer_path(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    remove_existing_target(path)
}

/// 刪除貼上覆蓋前已存在的目標路徑。
///
/// 參數：
/// - `target_path: &Path`，即將被覆蓋的既有檔案或資料夾。
///
/// 回傳：`io::Result<()>`。
/// - 成功時代表既有目標已被安全移除。
/// - 失敗時代表沒有權限、目標正被使用或刪除過程出錯。
fn remove_existing_target(target_path: &Path) -> io::Result<()> {
    if target_path.is_dir() {
        fs::remove_dir_all(target_path)
    } else {
        fs::remove_file(target_path)
    }
}

/// 遞迴複製整個資料夾，並逐檔執行同步與完整性驗證。
///
/// 參數：
/// - `source_dir: &Path`，來源資料夾。
/// - `target_dir: &Path`，目標資料夾。
///
/// 回傳：`io::Result<()>`；任一檔案未完整寫入就立即失敗，外層會清除整個 partial 目錄。
fn copy_dir_recursive(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    fs::create_dir(target_dir)?;

    for item in fs::read_dir(source_dir)? {
        let item = item?;
        let item_path = item.path();
        let next_target = target_dir.join(item.file_name());

        if item.file_type()?.is_dir() {
            copy_dir_recursive(&item_path, &next_target)?;
        } else {
            copy_file_and_verify(&item_path, &next_target)?;
        }
    }

    Ok(())
}

/// 遞迴複製資料夾並以檔案實際寫入量更新背景 task。
///
/// 參數：`source_dir`、`target_dir` 為來源與目標；`progress` 接收新增完成的 byte 數。
/// 回傳：`io::Result<()>`；任一檔案不完整就停止並由外層清除 partial 目錄。
fn copy_dir_recursive_with_progress<F>(
    source_dir: &Path,
    target_dir: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(TransferProgress),
{
    copy_dir_parallel_with_progress(
        source_dir,
        target_dir,
        progress,
        |source_path, target_path, expected_size, file_progress| {
            if expected_size < PROGRESSIVE_NATIVE_COPY_THRESHOLD_BYTES {
                copy_file_with_native_fallback_known_size(
                    source_path,
                    target_path,
                    expected_size,
                    file_progress,
                    |source, target| fs::copy(source, target),
                )
            } else {
                copy_file_native_with_progress(source_path, target_path, file_progress)
            }
        },
    )
}

/// 表示目錄走訪器交給 file worker 的單一複製工作。
#[derive(Debug)]
struct CopyFileJob {
    source_path: PathBuf,
    target_path: PathBuf,
    expected_size: u64,
}

/// 表示 file worker 完成一個工作後回傳給協調執行緒的結果。
#[derive(Debug)]
enum CopyFileResult {
    Discovered(u64),
    Progress(u64),
    Copied,
    Failed(io::Error),
}

/// 以有界工作佇列與固定 worker 數並行複製目錄中的檔案。
///
/// 這個流程採用成熟的 producer-worker scheduler：走訪器一邊發現檔案，一邊把單檔 copy 排給多個
/// worker，不會等上一個檔案完成後才處理下一個。`sync_channel` 限制尚未處理的工作量，
/// 因此即使目錄有數十萬個檔案，也不會一次把全部路徑保留在記憶體。
///
/// 參數：
/// - `source_dir: &Path`，來源目錄。
/// - `target_dir: &Path`，要建立的目標目錄。
/// - `progress: &mut F`，接收走訪器與 worker 回報的傳輸事件。
/// - `copy_file: C`，平台原生單檔複製函數；第三項是已知來源大小，第四項回報完成量。
///
/// 回傳：`io::Result<()>`；走訪、建立目錄或任一 worker 失敗時回傳第一個錯誤。
fn copy_dir_parallel_with_progress<F, C>(
    source_dir: &Path,
    target_dir: &Path,
    progress: &mut F,
    copy_file: C,
) -> io::Result<()>
where
    F: FnMut(TransferProgress),
    C: Fn(&Path, &Path, u64, &mut dyn FnMut(u64)) -> io::Result<()> + Sync,
{
    // 第一層目標一建立就通知 App 刷新目的 panel。這個 0 不是完成 byte 數，而是
    // 「目標已可見」訊號；真正百分比仍只由後續成功複製的 byte 累計。
    fs::create_dir(target_dir)?;
    progress(TransferProgress::TargetVisible);

    let (job_sender, job_receiver) = mpsc::sync_channel::<CopyFileJob>(COPY_FILE_WORKERS * 8);
    let job_receiver = Arc::new(Mutex::new(job_receiver));
    let (result_sender, result_receiver) = mpsc::channel::<CopyFileResult>();
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut traversal_error = None;

    thread::scope(|scope| {
        for _ in 0..COPY_FILE_WORKERS {
            let jobs = Arc::clone(&job_receiver);
            let results = result_sender.clone();
            let cancelled = Arc::clone(&cancelled);
            let copy_file = &copy_file;
            scope.spawn(move || {
                loop {
                    if cancelled.load(AtomicOrdering::Relaxed) {
                        break;
                    }
                    let job = match jobs.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => break,
                    };
                    let Ok(job) = job else {
                        break;
                    };
                    let mut file_progress = |increment| {
                        let _ = results.send(CopyFileResult::Progress(increment));
                    };
                    match copy_file(
                        &job.source_path,
                        &job.target_path,
                        job.expected_size,
                        &mut file_progress,
                    ) {
                        Ok(()) => {
                            if results.send(CopyFileResult::Copied).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            cancelled.store(true, AtomicOrdering::Relaxed);
                            let _ = results.send(CopyFileResult::Failed(error));
                            break;
                        }
                    }
                }
            });
        }
        // producer 不可保留一份永遠存活的 receiver；所有 worker 因錯誤退出後，
        // bounded sender 必須收到 disconnected，才能結束而不是永久等待空位。
        drop(job_receiver);
        // 走訪器必須和結果接收端同時執行。若在目前執行緒先完整走訪再讀 result，
        // 大型目錄會等掃描結束後才把 worker 進度交給 App，畫面因此長時間停在
        // RUNNING。獨立 producer 一邊建立目錄、一邊送出檔案工作，下面的 consumer
        // 則能從第一個檔案開始立即轉送進度。
        let producer_cancelled = Arc::clone(&cancelled);
        let producer_results = result_sender.clone();
        let producer = scope.spawn(move || {
            if let Err(error) = enqueue_copy_tree(
                source_dir,
                target_dir,
                &job_sender,
                &producer_results,
                producer_cancelled.as_ref(),
            ) {
                producer_cancelled.store(true, AtomicOrdering::Relaxed);
                let _ = producer_results.send(CopyFileResult::Failed(error));
            }
        });
        drop(result_sender);

        for result in result_receiver {
            match result {
                CopyFileResult::Discovered(size) => {
                    progress(TransferProgress::BytesDiscovered(size));
                }
                CopyFileResult::Progress(increment) => {
                    progress(TransferProgress::BytesCopied(increment));
                }
                CopyFileResult::Copied => {}
                CopyFileResult::Failed(error) if traversal_error.is_none() => {
                    traversal_error = Some(error);
                }
                CopyFileResult::Failed(_) => {}
            }
        }

        if producer.join().is_err() && traversal_error.is_none() {
            traversal_error = Some(io::Error::other("copy tree producer panicked"));
        }
    });

    match traversal_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// 以廣度優先方式建立目標目錄，並把每個一般檔案送入有界 worker 佇列。
///
/// 參數：`source_dir` 與 `target_dir` 為目前走訪層級；`jobs` 是工作 sender；
/// `results` 回報新發現的 byte；`cancelled` 在任一 worker 失敗後停止繼續發現新工作。
/// 回傳：`io::Result<()>`；讀取來源或建立目標失敗時保留原始作業系統錯誤。
fn enqueue_copy_tree(
    source_dir: &Path,
    target_dir: &Path,
    jobs: &mpsc::SyncSender<CopyFileJob>,
    results: &mpsc::Sender<CopyFileResult>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let mut directories = VecDeque::from([(source_dir.to_path_buf(), target_dir.to_path_buf())]);
    while let Some((current_source, current_target)) = directories.pop_front() {
        for item in fs::read_dir(&current_source)? {
            if cancelled.load(AtomicOrdering::Relaxed) {
                return Ok(());
            }
            let item = item?;
            let source_path = item.path();
            let target_path = current_target.join(item.file_name());
            if item.file_type()?.is_dir() {
                fs::create_dir(&target_path)?;
                directories.push_back((source_path, target_path));
            } else {
                let expected_size = item.metadata()?.len();
                results
                    .send(CopyFileResult::Discovered(expected_size))
                    .map_err(|_| io::Error::other("copy result channel disconnected"))?;
                send_copy_job(
                    jobs,
                    cancelled,
                    CopyFileJob {
                        source_path,
                        target_path,
                        expected_size,
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// 將工作放入有界佇列，並在其他 worker 失敗後立即停止等待。
///
/// 阻塞式 `send` 會自然施加 backpressure，避免忙等耗盡 CPU；外層不保留額外 receiver，
/// 因此所有 worker 因錯誤停止時 channel 會斷線，走訪器可立即結束而不會永久卡住。
///
/// 參數：`jobs` 為有界佇列；`cancelled` 是共享取消狀態；`job` 是待排入的檔案工作。
/// 回傳：`io::Result<()>`；佇列斷線時回傳錯誤，已取消時不再排入並正常返回。
fn send_copy_job(
    jobs: &mpsc::SyncSender<CopyFileJob>,
    cancelled: &AtomicBool,
    job: CopyFileJob,
) -> io::Result<()> {
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Ok(());
    }
    match jobs.send(job) {
        Ok(()) => Ok(()),
        Err(_) if cancelled.load(AtomicOrdering::Relaxed) => Ok(()),
        Err(_) => Err(io::Error::other("copy worker queue disconnected")),
    }
}

/// 計算檔案或資料夾樹中一般檔案的總 byte 數，供 task 百分比分母使用。
///
/// 參數：`path: &Path`，要統計的來源。
/// 回傳：`io::Result<u64>`；資料夾會遞迴加總，無法讀取時回傳原始 I/O 錯誤。
pub(crate) fn path_content_size(path: &Path) -> io::Result<u64> {
    let metadata = fs::metadata(path)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(path_content_size(&entry?.path())?);
    }
    Ok(total)
}

/// 根據目標資料夾現況，產生一個不與既有項目衝突的新路徑。
///
/// 參數：
/// - `target_dir: &Path`，要貼入項目的目錄。
/// - `original_name: &std::ffi::OsStr`，來源項目的原始檔名。
///
/// 回傳：`PathBuf`，可安全使用的新目標路徑。
fn unique_target_path(target_dir: &Path, original_name: &std::ffi::OsStr) -> PathBuf {
    let original_name = original_name.to_string_lossy();
    let initial_candidate = target_dir.join(original_name.as_ref());
    if !initial_candidate.exists() {
        return initial_candidate;
    }

    let (base_name, extension) = split_name_for_duplicate(&original_name);
    let mut duplicate_index = 1usize;

    loop {
        let candidate_name = if duplicate_index == 1 {
            duplicate_name(&base_name, extension.as_deref(), None)
        } else {
            duplicate_name(&base_name, extension.as_deref(), Some(duplicate_index))
        };
        let candidate_path = target_dir.join(candidate_name);
        if !candidate_path.exists() {
            return candidate_path;
        }
        duplicate_index += 1;
    }
}

/// 將檔名拆成「主名稱」與「副檔名」，方便產生重複貼上的新名稱。
///
/// 參數：
/// - `name: &str`，原始檔名。
///
/// 回傳：`(String, Option<String>)`。
/// - 第一個值是主名稱。
/// - 第二個值是副檔名，若沒有副檔名則為 `None`。
fn split_name_for_duplicate(name: &str) -> (String, Option<String>) {
    if name.starts_with('.') && !name[1..].contains('.') {
        return (name.to_string(), None);
    }

    match name.rsplit_once('.') {
        Some((base_name, extension)) if !base_name.is_empty() => {
            (base_name.to_string(), Some(extension.to_string()))
        }
        _ => (name.to_string(), None),
    }
}

/// 依照主名稱、副檔名與重複次數產生新的不衝突檔名。
///
/// 參數：
/// - `base_name: &str`，檔案主名稱。
/// - `extension: Option<&str>`，副檔名。
/// - `duplicate_index: Option<usize>`，若有重複次數，會附加在 `copy` 後面。
///
/// 回傳：`String`，可直接作為新檔名的字串。
fn duplicate_name(
    base_name: &str,
    extension: Option<&str>,
    duplicate_index: Option<usize>,
) -> String {
    let mut candidate = String::from(base_name);
    candidate.push_str(" copy");

    if let Some(index) = duplicate_index {
        candidate.push(' ');
        candidate.push_str(&index.to_string());
    }

    if let Some(extension) = extension {
        candidate.push('.');
        candidate.push_str(extension);
    }

    candidate
}

/// 取出路徑最後一段作為顯示名稱，若缺少檔名則回傳空字串。
///
/// 參數：
/// - `path: &Path`，要讀取名稱的路徑。
///
/// 回傳：`String`，最後一段檔名。
fn target_path_file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 驗證新建立項目的名稱是否可用，避免空白名稱或直接包含路徑分隔符。
///
/// 參數：
/// - `name: &str`，使用者輸入的新名稱。
///
/// 回傳：`io::Result<&str>`。
/// - 成功時回傳去除前後空白後的名稱。
/// - 失敗時回傳名稱無效的錯誤。
fn validate_new_entry_name(name: &str) -> io::Result<&str> {
    let trimmed_name = name.trim();
    if trimmed_name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "name cannot be empty",
        ));
    }

    Ok(trimmed_name)
}

/// 描述一次建立請求解析後的結果。
struct CreateRequest {
    relative_path: PathBuf,
    display_name: String,
    is_directory: bool,
}

/// 解析建立輸入，決定要建立檔案還是資料夾，並驗證路徑是否安全。
fn parse_create_input(input: &str) -> io::Result<CreateRequest> {
    let trimmed = validate_new_entry_name(input)?;
    let is_directory = trimmed.ends_with('/');
    let normalized = trimmed.trim_end_matches('/');
    if normalized.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "name cannot be empty",
        ));
    }

    let relative_path = PathBuf::from(normalized);
    for component in relative_path.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path must stay inside current directory",
                ));
            }
        }
    }

    let display_name = if is_directory {
        format!("{normalized}/")
    } else {
        normalized.to_string()
    };

    Ok(CreateRequest {
        relative_path,
        display_name,
        is_directory,
    })
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

/// 快速讀取目錄：在背景多執行緒讀取 metadata 並完成自然排序，直接送出 100% 正確排序的完整清單，徹底避免畫面列表跳動。
pub(crate) fn stream_dir_entries_with_cancellation<F>(
    path: &Path,
    sort_mode: SortMode,
    random_seed: u64,
    cancelled: &AtomicBool,
    mut on_progress: F,
) -> io::Result<()>
where
    F: FnMut(DirectoryLoadProgress) -> bool,
{
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    let read_dir = fs::read_dir(path)?;
    let mut items = Vec::new();
    let mut first_batch = Vec::new();
    let mut sent_first_chunk = false;

    for dir_entry_result in read_dir {
        if cancelled.load(AtomicOrdering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "directory load cancelled",
            ));
        }

        let item = dir_entry_result?;
        let file_type = item.file_type().ok();
        let is_dir = file_type.map(|t| t.is_dir()).unwrap_or(false);
        let name = item.file_name().to_string_lossy().into_owned();
        let entry_path = item.path();

        if !sent_first_chunk {
            first_batch.push(FileEntry {
                name,
                path: entry_path,
                is_dir,
                size: 0,
                directory_size: None,
                directory_size_complete: false,
                modified: SystemTime::UNIX_EPOCH,
                created: SystemTime::UNIX_EPOCH,
                readonly: false,
                unix_mode: None,
            });
            if first_batch.len() >= 128 {
                sort_file_entries(&mut first_batch, sort_mode, random_seed);
                let chunk = std::mem::take(&mut first_batch);
                if !on_progress(DirectoryLoadProgress::Batch {
                    entries: chunk,
                    is_first_chunk: true,
                }) {
                    return Ok(());
                }
                sent_first_chunk = true;
            }
        }

        items.push(item);
    }

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    // 第一批暫存清單只負責讓大型目錄立刻可見；Complete 則一定要在背景補齊
    // metadata。不能因為目前採 natural 排序就省略 metadata，因為使用者可能在進入
    // 目錄前已啟用 `ms`，或進入後才切換 size/permissions/mtime/btime。舊實作會讓
    // 這些背景載入的項目永久保留 size = 0，而直接建立的新 panel 卻有正確資料，造成
    // 同一路徑的兩個 panel 顯示不一致。這裡仍在 worker 執行緒平行讀取，不會阻塞 TUI。
    let mut final_entries = read_metadata_for_dir_entries(items, cancelled)?;

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    // 在背景執行緒進行自然排序，避免在主 UI 執行緒排序數萬筆檔案造成卡頓
    sort_file_entries(&mut final_entries, sort_mode, random_seed);

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    let _ = on_progress(DirectoryLoadProgress::Complete(final_entries));
    Ok(())
}

/// 讀取指定目錄，並整理成可顯示的檔案項目清單。
///
/// 參數：
/// - `path: &Path`，要掃描的目錄路徑。
///
/// 回傳：`io::Result<Vec<FileEntry>>`。
/// - 成功時回傳已排序的檔案與資料夾清單。
/// - 失敗時回傳讀取目錄或 metadata 時的 I/O 錯誤。
pub(crate) fn read_dir_entries(path: &Path) -> io::Result<Vec<FileEntry>> {
    read_dir_entries_with_cancellation(path, &AtomicBool::new(false))
}

/// 讀取指定目錄並支援即時取消，整理成可顯示的檔案項目清單。
///
/// 參數：
/// - `path: &Path`，要掃描的目錄路徑。
/// - `cancelled: &AtomicBool`，外部通知提早中斷的取消旗標。
///
/// 回傳：`io::Result<Vec<FileEntry>>`。
/// - 成功時回傳檔案與資料夾清單。
/// - 若中途被取消，回傳 `ErrorKind::Interrupted` 的 I/O 錯誤。
/// - 失敗時回傳讀取目錄或 metadata 時的 I/O 錯誤。
pub(crate) fn read_dir_entries_with_cancellation(
    path: &Path,
    cancelled: &AtomicBool,
) -> io::Result<Vec<FileEntry>> {
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }
    let items = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }
    read_metadata_for_dir_entries(items, cancelled)
}

/// 讀取指定項目清單的完整 metadata，並支援中途取消。
fn read_metadata_for_dir_entries(
    items: Vec<fs::DirEntry>,
    cancelled: &AtomicBool,
) -> io::Result<Vec<FileEntry>> {
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(8)
        .min(items.len().max(1));

    // 小目錄直接處理可避免建立 thread 的成本；大型目錄則把 metadata 系統呼叫分散到
    // 有上限的 worker。這段仍需等清單完成才能排序，但不再讓數萬筆 metadata 串行阻塞。
    if items.len() < 512 || worker_count == 1 {
        let mut entries = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            if index % 64 == 0 && cancelled.load(AtomicOrdering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "directory load cancelled",
                ));
            }
            entries.push(file_entry_from_dir_entry(item)?);
        }
        return Ok(entries);
    }

    let chunk_size = items.len().div_ceil(worker_count);
    let mut chunks = items.into_iter();
    let results = thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..worker_count {
            let chunk = chunks.by_ref().take(chunk_size).collect::<Vec<_>>();
            if chunk.is_empty() {
                break;
            }
            workers.push(scope.spawn(move || {
                let mut chunk_entries = Vec::with_capacity(chunk.len());
                for (index, item) in chunk.into_iter().enumerate() {
                    if index % 64 == 0 && cancelled.load(AtomicOrdering::Relaxed) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "directory load cancelled",
                        ));
                    }
                    chunk_entries.push(file_entry_from_dir_entry(item)?);
                }
                Ok(chunk_entries)
            }));
        }

        workers
            .into_iter()
            .map(|worker| {
                worker.join().map_err(|_| {
                    io::Error::other("directory metadata worker terminated unexpectedly")
                })?
            })
            .collect::<io::Result<Vec<_>>>()
    })?;

    if cancelled.load(AtomicOrdering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "directory load cancelled",
        ));
    }

    Ok(results.into_iter().flatten().collect())
}

/// 將單一 `DirEntry` 轉成 PaneFM 列表資料。
///
/// 參數：`item: fs::DirEntry`，由目前目錄讀出的項目。
/// 回傳：`io::Result<FileEntry>`；metadata 無法讀取時保留原始作業系統錯誤。
fn file_entry_from_dir_entry(item: fs::DirEntry) -> io::Result<FileEntry> {
    let file_type = item.file_type()?;
    let metadata = item.metadata()?;
    let entry_path = item.path();
    Ok(FileEntry {
        name: item.file_name().to_string_lossy().into_owned(),
        path: entry_path,
        is_dir: file_type.is_dir(),
        size: metadata.len(),
        // 遞迴目錄容量由 App 的背景 worker 計算；一般列表載入不可同步掃描。
        directory_size: None,
        directory_size_complete: false,
        modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        created: metadata.created().unwrap_or(SystemTime::UNIX_EPOCH),
        readonly: metadata.permissions().readonly(),
        unix_mode: read_unix_mode(&metadata),
    })
}

/// 讀取目前平台可提供的 Unix 權限位元，供 linemode permissions 顯示。
///
/// 參數：
/// - `metadata: &fs::Metadata`，目前項目的 metadata。
///
/// 回傳：`Option<u32>`。
/// - 在 Unix 平台回傳完整 mode bit。
/// - 在其他平台回傳 `None`，讓 UI 採用跨平台 fallback 顯示。
#[cfg(unix)]
fn read_unix_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;

    Some(metadata.mode())
}

/// 在非 Unix 平台上，目前沒有標準庫可直接讀完整 rwx 權限，因此回傳 `None`。
#[cfg(not(unix))]
fn read_unix_mode(_: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File, OpenOptions},
        io::{self, Write},
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;

    use super::{
        DirectoryLoadProgress, PaneState, SortMode, TransferProgress,
        copy_dir_parallel_with_progress, copy_file_and_verify, copy_file_and_verify_with,
        copy_file_native_with_progress, copy_file_native_with_progress_using,
        copy_file_with_native_fallback, copy_path_direct_with_cleanup,
        copy_path_transactional_with, natural_cmp, read_dir_entries,
        read_dir_entries_with_cancellation, stream_dir_entries_with_cancellation,
    };
    use crate::file_manager::entry::FileEntry;
    use crate::file_manager::search::GlobalSearchEntry;
    use crate::theme::Theme;

    #[test]
    /// 驗證 pane 重新載入目錄時，資料夾會排在檔案前面。
    ///
    /// 參數：無。
    /// 回傳：無；若排序規則錯誤則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_lists_directories_before_files() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("nested")).expect("nested dir");
        fs::write(dir.path().join("alpha.txt"), "hello").expect("file");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let names: Vec<String> = pane.entries.iter().map(FileEntry::display_name).collect();

        assert_eq!(
            names,
            vec![String::from("nested/"), String::from("alpha.txt")]
        );
    }

    #[test]
    /// 驗證自然排序不需要把數字轉成 `u64`，超長數字與前置零仍有穩定順序。
    ///
    /// 保護目的：大型 build 目錄含有數萬個帶 hash 或數字的檔名；自然排序改為零配置
    /// 比較器後，必須同時保留 `file2 < file10` 的語意，且不可因數字溢位退回文字排序。
    fn natural_compare_handles_large_numbers_without_allocating_numeric_strings() {
        assert!(natural_cmp("file2", "file10").is_lt());
        assert!(natural_cmp("file0002", "file2").is_gt());
        assert!(
            natural_cmp(
                "build999999999999999999999999",
                "build1000000000000000000000000"
            )
            .is_lt()
        );
        assert!(natural_cmp("中文2", "中文10").is_lt());
    }

    #[test]
    /// 驗證超過平行化門檻的大型目錄仍完整讀取每一筆 metadata。
    ///
    /// 保護目的：大型目錄改成多 worker 後，chunk 邊界不能遺漏或重複項目；檔案大小也
    /// 必須保持準確，否則 size linemode、排序與後續傳輸估算都會得到錯誤資料。
    fn large_directory_metadata_loading_keeps_every_entry_and_size() {
        let directory = tempdir().expect("tempdir");
        for index in 0..520usize {
            fs::write(
                directory.path().join(format!("entry-{index}.bin")),
                [index as u8],
            )
            .expect("write fixture");
        }

        let entries = read_dir_entries(directory.path()).expect("read large directory");

        assert_eq!(entries.len(), 520);
        assert!(entries.iter().all(|entry| entry.size == 1));
    }

    #[test]
    /// 驗證一般目錄載入不會同步遞迴統計大小，避免本機大型目錄或 SMB 首次瀏覽卡住。
    ///
    /// 參數：無。
    /// 回傳：無；若預設載入已填入目錄容量，測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_defers_directory_sizes_until_requested() {
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("one.txt"), "one").expect("first child");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");

        assert_eq!(directory.directory_size, None);
        assert!(!directory.directory_size_complete);
    }

    #[test]
    /// 驗證背景掃描的部分大小與完成狀態可以分階段更新同一個目錄。
    ///
    /// 參數：無。
    /// 回傳：無；若部分值、最終值或完成旗標不正確，測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_applies_incremental_directory_size_snapshot() {
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("one.txt"), "one").expect("first child");
        fs::write(nested.join("two.txt"), "two").expect("second child");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        assert!(pane.update_directory_size(&nested, 3, false));
        let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");

        assert_eq!(directory.directory_size, Some(3));
        assert!(!directory.directory_size_complete);

        assert!(pane.update_directory_size(&nested, 6, true));
        let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");
        assert_eq!(directory.directory_size, Some(6));
        assert!(directory.directory_size_complete);
    }

    #[test]
    /// 驗證 watcher 或背景貼上觸發列表 reload 時，不會把正在顯示的目錄大小清空。
    ///
    /// 保護目的：`ms` 掃描可能持續數秒；若每次外部檔案事件都重設快取，畫面會反覆
    /// 跳回 `…` 或 `~0B`，大型目錄甚至永遠無法顯示完成值。
    fn pane_reload_preserves_directory_size_cache_for_existing_paths() {
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).expect("nested dir");
        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        assert!(pane.update_directory_size(&nested, 42_000, true));

        fs::write(dir.path().join("new.txt"), "new").expect("external file");
        pane.reload().expect("reload");

        let directory = pane
            .entries
            .iter()
            .find(|entry| entry.path == nested)
            .expect("dir");
        assert_eq!(directory.directory_size, Some(42_000));
        assert!(directory.directory_size_complete);
    }

    #[test]
    /// 驗證 pane 可以正確進入子目錄並返回父目錄。
    ///
    /// 參數：無。
    /// 回傳：無；若目錄切換行為錯誤則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_enters_and_leaves_directories() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("alpha")).expect("alpha dir");
        let child = dir.path().join("child");
        fs::create_dir(&child).expect("child dir");
        fs::write(child.join("note.txt"), "hello").expect("note");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.move_down_by(1);
        pane.enter_selected().expect("enter child");
        assert_eq!(pane.cwd, child);

        pane.go_parent().expect("back parent");
        assert_eq!(pane.cwd, dir.path());
        assert_eq!(
            pane.selected_entry().map(FileEntry::display_name),
            Some(String::from("child/"))
        );
    }

    #[test]
    /// 驗證 `PaneState` 可以正確刪除目前選取的檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若檔案未被刪除或狀態未更新則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_delete_selected_file_removes_it() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let removed = pane.delete_selected().expect("delete");

        assert_eq!(removed, Some(String::from("alpha.txt")));
        assert!(!file_path.exists());
        assert!(pane.entries.is_empty());
    }

    #[test]
    /// 驗證 `PaneState` 可以正確重新命名目前選取的檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若檔案未改名或狀態未更新則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_rename_selected_file_updates_entry() {
        let dir = tempdir().expect("tempdir");
        let old_path = dir.path().join("alpha.txt");
        let new_path = dir.path().join("beta.txt");
        fs::write(&old_path, "hello").expect("file");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let renamed = pane.rename_selected("beta.txt").expect("rename");

        assert_eq!(renamed, Some(String::from("beta.txt")));
        assert!(!old_path.exists());
        assert!(new_path.exists());
        assert_eq!(
            pane.selected_entry().map(FileEntry::display_name),
            Some(String::from("beta.txt"))
        );
    }

    #[test]
    /// 驗證同一個目錄內重複複製檔案時，會自動產生不衝突的新檔名。
    ///
    /// 參數：無。
    /// 回傳：無；若重複名稱處理錯誤則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_copy_into_same_directory_creates_duplicate_file_name() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let first_copy = pane
            .copy_entry_into_current_dir(&file_path)
            .expect("first copy");
        let second_copy = pane
            .copy_entry_into_current_dir(&file_path)
            .expect("second copy");

        assert_eq!(first_copy, "alpha copy.txt");
        assert_eq!(second_copy, "alpha copy 2.txt");
        assert!(dir.path().join("alpha copy.txt").exists());
        assert!(dir.path().join("alpha copy 2.txt").exists());
    }

    #[test]
    /// 驗證貼上前取得的預計目標路徑與同名複製規則完全一致。
    /// 保護目的：避免錯誤訊息顯示原始檔名，但實際失敗位置是 `copy` 名稱而誤導 SMB 除錯。
    fn pane_state_planned_paste_target_uses_actual_duplicate_name() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");
        fs::write(dir.path().join("alpha copy.txt"), "existing").expect("existing copy");
        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

        let planned = pane
            .planned_paste_target(&file_path, false)
            .expect("planned target");

        assert_eq!(planned, dir.path().join("alpha copy 2.txt"));
        assert!(!planned.exists());
    }

    #[test]
    /// 驗證同一個目錄內重複複製資料夾時，也會自動產生不衝突的新名稱。
    ///
    /// 參數：無。
    /// 回傳：無；若資料夾重複名稱處理錯誤則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_copy_into_same_directory_creates_duplicate_directory_name() {
        let dir = tempdir().expect("tempdir");
        let folder_path = dir.path().join("docs");
        fs::create_dir(&folder_path).expect("folder");
        fs::write(folder_path.join("note.txt"), "hello").expect("note");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let first_copy = pane
            .copy_entry_into_current_dir(&folder_path)
            .expect("first copy");
        let second_copy = pane
            .copy_entry_into_current_dir(&folder_path)
            .expect("second copy");

        assert_eq!(first_copy, "docs copy/");
        assert_eq!(second_copy, "docs copy 2/");
        assert!(dir.path().join("docs copy").is_dir());
        assert!(dir.path().join("docs copy 2").is_dir());
        assert!(dir.path().join("docs copy").join("note.txt").exists());
        assert!(dir.path().join("docs copy 2").join("note.txt").exists());
    }

    #[test]
    /// 驗證交易式複製成功後只留下正式檔名，不會把內部暫存檔暴露在目標目錄。
    ///
    /// 參數：無。
    /// 回傳：無；若正式內容錯誤或 `.panefm-transfer-*` 暫存路徑殘留則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn transactional_copy_commits_complete_file_without_temp_residue() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.zip");
        let target_dir = dir.path().join("target");
        let target = target_dir.join("source.zip");
        fs::create_dir(&target_dir).expect("target dir");
        fs::write(&source, b"complete zip bytes").expect("source");

        super::copy_path_transactional(&source, &target, false).expect("transactional copy");

        assert_eq!(
            fs::read(&target).expect("target content"),
            b"complete zip bytes"
        );
        assert!(!directory_has_transfer_temp(&target_dir));
    }

    #[test]
    /// 模擬 SMB 寫入部分內容後失敗，驗證正式檔名與暫存檔都不會殘留。
    ///
    /// 參數：無。
    /// 回傳：無；若失敗後仍佔用正式檔名，Finder 下一次複製可能被迫產生 `copy` 名稱。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn transactional_copy_failure_removes_partial_staged_file() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.zip");
        let target = dir.path().join("target.zip");
        fs::write(&source, b"complete zip bytes").expect("source");

        let result = copy_path_transactional_with(&source, &target, false, |_, staged| {
            fs::write(staged, b"partial")?;
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "simulated SMB disconnect",
            ))
        });

        assert_eq!(
            result.expect_err("copy must fail").kind(),
            std::io::ErrorKind::ConnectionReset
        );
        assert!(!target.exists());
        assert!(!directory_has_transfer_temp(dir.path()));
    }

    #[test]
    /// 模擬一般 SMB 複製直接寫入部分正式內容後失敗，驗證該檔名會被立即清除。
    ///
    /// 參數：無。
    /// 回傳：無；若部分檔案仍存在，Finder 後續複製就可能自動改成 `copy` 名稱。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn direct_copy_failure_removes_partial_target() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.zip");
        let target = dir.path().join("target.zip");
        fs::write(&source, b"complete zip bytes").expect("source");

        let result = copy_path_direct_with_cleanup(&source, &target, |_, target| {
            fs::write(target, b"partial")?;
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "simulated SMB disconnect",
            ))
        });

        assert_eq!(
            result.expect_err("copy must fail").kind(),
            std::io::ErrorKind::ConnectionReset
        );
        assert!(!target.exists());
    }

    #[test]
    /// 驗證跨平台原生 copy 會完整複製內容，並回報正確 byte 數。
    ///
    /// 參數：無。
    /// 回傳：無；若目標內容或進度累計與來源不同，測試失敗。
    /// 保護目的：本機、Windows UNC 與 macOS 掛載路徑統一改用原生 API 後，仍須驗證
    /// 回報進度與目標內容，避免把不完整檔案當成成功。
    fn native_copy_verifies_content_and_reports_progress() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.bin");
        let target = dir.path().join("target.bin");
        let bytes = b"native local copy content";
        fs::write(&source, bytes).expect("source");
        let mut completed = 0u64;

        copy_file_native_with_progress(&source, &target, &mut |increment| {
            completed = completed.saturating_add(increment);
        })
        .expect("native copy");

        assert_eq!(completed, bytes.len() as u64);
        assert_eq!(fs::read(&target).expect("target"), bytes);
    }

    #[test]
    /// 驗證平台原生 copy 回傳「不支援」且留下 0-byte 目標時，會清理該目標並改走
    /// 分塊串流，最後仍得到完整內容與正確進度。
    ///
    /// 保護目的：部分 macOS／Windows SMB server 不支援原生 copy 加速；PaneFM 過去
    /// 會直接顯示失敗或留下 0 KB ZIP。這個測試確保 fallback 是跨平台傳檔的必要
    /// 正確性路徑，而不是只在特定公司環境手動驗證。
    fn unsupported_native_copy_falls_back_to_verified_streaming_copy() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.zip");
        let target = dir.path().join("target.zip");
        let payload = vec![0x5a; 2 * 1024 * 1024 + 37];
        fs::write(&source, &payload).expect("source");
        let mut reported = 0u64;

        copy_file_with_native_fallback(
            &source,
            &target,
            &mut |increment| reported = reported.saturating_add(increment),
            |_, target| {
                File::create(target).expect("simulated partial target");
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "native copy unsupported by share",
                ))
            },
        )
        .expect("stream fallback");

        assert_eq!(reported, payload.len() as u64);
        assert_eq!(fs::read(&target).expect("target bytes"), payload);
    }

    #[test]
    /// 驗證原生 copy 的一般權限錯誤不會被錯誤地改成串流重試。
    ///
    /// 保護目的：fallback 只能處理「API 不支援」；若權限、磁碟空間或連線本身失敗，
    /// 必須保留原始 OS error，避免第二次寫入掩蓋真正原因或造成額外 partial 檔案。
    fn native_copy_permission_error_does_not_trigger_stream_fallback() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.bin");
        let target = dir.path().join("target.bin");
        fs::write(&source, b"protected").expect("source");
        let mut reported = 0u64;

        let error = copy_file_with_native_fallback(
            &source,
            &target,
            &mut |increment| reported = reported.saturating_add(increment),
            |_, _| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "permission denied by share",
                ))
            },
        )
        .expect_err("permission error must remain visible");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(reported, 0);
        assert!(!target.exists());
    }

    #[test]
    /// 驗證原生 copy 尚未完成時，metadata 輪詢就能先回報部分進度。
    ///
    /// 參數：無。
    /// 回傳：無；若 progress 只能在整個 copy 結束後一次跳到 100%，或累計 byte 不等於
    /// 來源大小，測試就會失敗。
    /// 保護目的：PaneFM 改用平台原生 copy 後，不能為了顯示百分比退回手寫串流；
    /// 此測試保護「copy 引擎與進度 metadata 輪詢互相獨立」的核心設計。
    fn native_copy_polls_destination_metadata_before_completion() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.bin");
        let target = dir.path().join("target.bin");
        let bytes = b"12345678";
        fs::write(&source, bytes).expect("source");
        let mut increments = Vec::new();

        copy_file_native_with_progress_using(
            &source,
            &target,
            &mut |increment| increments.push(increment),
            |source, target| {
                let source_bytes = fs::read(source)?;
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(target)?;
                output.write_all(&source_bytes[..4])?;
                output.flush()?;
                thread::sleep(Duration::from_millis(450));
                output.write_all(&source_bytes[4..])?;
                output.flush()?;
                Ok(source_bytes.len() as u64)
            },
        )
        .expect("progressive native copy");

        assert!(increments.len() >= 2, "應在完成前至少回報一次部分進度");
        assert_eq!(increments.iter().sum::<u64>(), bytes.len() as u64);
        assert_eq!(fs::read(&target).expect("target"), bytes);
    }

    #[test]
    /// 驗證快速完成的小檔案 copy 不會因進度輪詢而被強制延遲 200ms。
    ///
    /// 參數：無。
    /// 回傳：無；四個各耗時約 10ms 的 copy 若累計超過 500ms，測試失敗。
    /// 保護目的：舊實作使用 `is_finished` 後固定 sleep 200ms，導致每個小檔都可能
    /// 多等一次完整輪詢週期；只有三個 worker 時，數千個檔案會被拖到數十分鐘。
    fn completed_small_file_copy_wakes_progress_waiter_immediately() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.bin");
        fs::write(&source, b"small file").expect("source");
        let started = Instant::now();

        for index in 0..4 {
            let target = dir.path().join(format!("target-{index}.bin"));
            copy_file_native_with_progress_using(
                &source,
                &target,
                &mut |_| {},
                |source, target| {
                    thread::sleep(Duration::from_millis(10));
                    fs::copy(source, target)
                },
            )
            .expect("small native copy");
        }

        assert!(
            started.elapsed() < Duration::from_millis(500),
            "small copies were delayed by progress polling: {:?}",
            started.elapsed()
        );
    }

    #[test]
    /// 驗證同磁碟移動資料夾會直接使用 rename，成功前不遞迴掃描內容或假造 byte 進度。
    ///
    /// 參數：無。
    /// 回傳：無；若來源仍存在、目標內容遺失或 rename 路徑回報 byte 進度則測試失敗。
    /// 保護目的：大型目錄本可由檔案系統瞬間改名，過去卻先呼叫 `path_content_size`
    /// 走訪整棵樹，導致 cut/paste 平白停頓數秒。
    fn same_device_directory_move_renames_without_prescanning_for_progress() {
        let dir = tempdir().expect("tempdir");
        let source_parent = dir.path().join("source-parent");
        let target_parent = dir.path().join("target-parent");
        let source = source_parent.join("build-output");
        fs::create_dir_all(&source).expect("source directory");
        fs::create_dir(&target_parent).expect("target parent");
        fs::write(source.join("artifact.bin"), b"artifact").expect("source file");
        let mut progress_calls = Vec::new();

        let outcome = PaneState::move_path_to_dir_with_history_progress(
            &source,
            &target_parent,
            false,
            &mut |increment| progress_calls.push(increment),
        )
        .expect("same-device move");

        assert!(!source.exists());
        assert_eq!(
            fs::read(outcome.target_path.join("artifact.bin")).expect("moved content"),
            b"artifact"
        );
        assert!(progress_calls.is_empty());
    }

    #[test]
    /// 模擬 SMB 不支援 rename，驗證 move 會改用原生 copy 後刪除來源。
    ///
    /// 參數：無。
    /// 回傳：無；若 rename 錯誤直接中止、目標內容不完整、來源提早刪除或進度不正確，
    /// 測試就會失敗。
    /// 保護目的：Windows UNC 與 macOS 掛載 share 可能拒絕 rename，但仍允許 copy；move
    /// 不可因此失效，而且只有完整 copy 通過大小驗證後才可移除來源。
    fn unsupported_rename_falls_back_to_copy_then_removes_source() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source = source_dir.join("archive.zip");
        let bytes = b"complete archive bytes";
        fs::write(&source, bytes).expect("source file");
        let mut completed = 0u64;

        let outcome = PaneState::move_path_to_dir_with_history_progress_using_rename(
            &source,
            &target_dir,
            false,
            &mut |event| {
                if let TransferProgress::BytesCopied(increment) = event {
                    completed = completed.saturating_add(increment);
                }
            },
            |_, _| Err(std::io::Error::from_raw_os_error(45)),
        )
        .expect("copy fallback");

        assert!(!source.exists());
        assert_eq!(fs::read(&outcome.target_path).expect("target"), bytes);
        assert_eq!(completed, bytes.len() as u64);
    }

    #[test]
    /// 驗證目錄複製會同時執行多個單檔工作，而不是退化成逐檔等待。
    ///
    /// 參數：無。
    /// 回傳：無；若目標未先變成可見、最高同時工作數小於 2、內容遺失或進度錯誤，
    /// 測試就會失敗。
    /// 保護目的：PaneFM 過去複製包含大量小檔案的 `target` 目錄時只用一條 worker，
    /// 小檔案處理會大幅變慢；此測試防止後續重構再次移除並行 scheduler。
    fn directory_copy_uses_multiple_workers_and_preserves_content() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        fs::create_dir(&source).expect("source dir");
        for index in 0..9 {
            fs::write(
                source.join(format!("file-{index}.txt")),
                format!("data-{index}"),
            )
            .expect("source file");
        }

        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let active_for_copy = Arc::clone(&active);
        let maximum_for_copy = Arc::clone(&maximum);
        let mut completed = 0u64;
        let mut target_became_visible = false;
        copy_dir_parallel_with_progress(
            &source,
            &target,
            &mut |event| match event {
                TransferProgress::TargetVisible => target_became_visible = target.is_dir(),
                TransferProgress::BytesCopied(increment) => {
                    completed = completed.saturating_add(increment);
                }
                TransferProgress::BytesDiscovered(_) => {}
            },
            move |from, to, _, progress| {
                let running = active_for_copy.fetch_add(1, Ordering::SeqCst) + 1;
                maximum_for_copy.fetch_max(running, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(15));
                let result = fs::copy(from, to).map(|copied| progress(copied));
                active_for_copy.fetch_sub(1, Ordering::SeqCst);
                result
            },
        )
        .expect("parallel directory copy");

        assert!(target_became_visible);
        assert!(maximum.load(Ordering::SeqCst) >= 2);
        let expected_bytes = (0..9)
            .map(|index| format!("data-{index}").len() as u64)
            .sum::<u64>();
        assert_eq!(completed, expected_bytes);
        for index in 0..9 {
            assert_eq!(
                fs::read_to_string(target.join(format!("file-{index}.txt"))).expect("target file"),
                format!("data-{index}")
            );
        }
    }

    #[test]
    /// 驗證複製一般專案目錄時會完整包含巢狀 `.tfm`，不會擅自略過使用者資料。
    ///
    /// 保護目的：即使 `.tfm` 可能很大，傳輸引擎仍必須靠高效率排程解決，不可用檔名
    /// 規則刪減複製內容，否則副本和來源不一致且可能遺失使用者需要的資料。
    fn directory_copy_preserves_nested_internal_state() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("project");
        let target = dir.path().join("project-copy");
        fs::create_dir_all(source.join(".tfm/trash/items")).expect("internal state");
        fs::write(source.join("source.rs"), b"project source").expect("project file");
        fs::write(source.join(".tfm/trash/items/large.bin"), b"internal trash")
            .expect("trash file");

        copy_dir_parallel_with_progress(&source, &target, &mut |_| {}, |from, to, _, progress| {
            let copied = fs::copy(from, to)?;
            progress(copied);
            Ok(())
        })
        .expect("copy project");

        assert_eq!(
            fs::read(target.join("source.rs")).expect("copied source"),
            b"project source"
        );
        assert_eq!(
            fs::read(target.join(".tfm/trash/items/large.bin")).expect("nested state copy"),
            b"internal trash"
        );
    }

    #[test]
    /// 驗證 producer 尚在排入大型目錄內容時，已完成檔案的進度就會立刻送到呼叫端。
    ///
    /// 參數：無。
    /// 回傳：無；若第一筆進度必須等大部分工作都排完或複製完才出現，測試失敗。
    /// 保護目的：舊流程在目前執行緒完整呼叫 `enqueue_copy_tree` 後才讀 result channel，
    /// 大型 `target` 目錄會長時間只顯示 RUNNING。此測試確保走訪 producer 與進度
    /// consumer 保持真正並行，第一批檔案完成時 UI 就有資料可更新。
    fn directory_copy_reports_progress_while_producer_is_still_working() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        fs::create_dir(&source).expect("source dir");
        for index in 0..100 {
            fs::write(source.join(format!("file-{index}.txt")), b"data").expect("source file");
        }

        let finished = Arc::new(AtomicUsize::new(0));
        let finished_for_copy = Arc::clone(&finished);
        let mut finished_when_first_progress_arrived = None;
        copy_dir_parallel_with_progress(
            &source,
            &target,
            &mut |event| {
                if matches!(event, TransferProgress::BytesCopied(increment) if increment > 0)
                    && finished_when_first_progress_arrived.is_none()
                {
                    finished_when_first_progress_arrived = Some(finished.load(Ordering::SeqCst));
                }
            },
            move |from, to, _, progress| {
                thread::sleep(Duration::from_millis(5));
                let copied = fs::copy(from, to)?;
                finished_for_copy.fetch_add(1, Ordering::SeqCst);
                progress(copied);
                Ok(())
            },
        )
        .expect("parallel directory copy");

        assert!(
            finished_when_first_progress_arrived.is_some_and(|count| count < 20),
            "first progress arrived too late: {finished_when_first_progress_arrived:?}"
        );
        assert_eq!(finished.load(Ordering::SeqCst), 100);
    }

    #[test]
    /// 驗證任一並行 worker 失敗後，有界工作佇列會停止接受新檔案並回傳錯誤。
    ///
    /// 參數：無。
    /// 回傳：無；若模擬 I/O 錯誤被忽略或函數無法結束，測試失敗。
    /// 保護目的：大量 SMB 工作塞滿佇列時，若 server 斷線，阻塞式 send 曾可能永遠
    /// 等待已停止的 worker；此測試保護錯誤取消路徑，避免背景 task 永久停在 RUNNING。
    fn parallel_directory_copy_stops_when_a_worker_fails() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        fs::create_dir(&source).expect("source dir");
        for index in 0..40 {
            fs::write(source.join(format!("file-{index}.txt")), b"data").expect("source file");
        }

        let result =
            copy_dir_parallel_with_progress(&source, &target, &mut |_| {}, |_, _, _, _| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "simulated SMB disconnect",
                ))
            });

        assert_eq!(
            result.expect_err("copy must fail").kind(),
            std::io::ErrorKind::ConnectionReset
        );
    }

    #[test]
    /// 驗證可靠複製會完整寫入資料，且回傳前已關閉目標檔案 handle。
    ///
    /// 參數：無。
    /// 回傳：無；若內容不一致或 Windows 因 handle 尚未關閉而無法改名，測試就會失敗。
    /// 保護目的：避免未來移除平台 copy 後的同步與大小驗證，導致 SMB 尚未完成就被 UI 視為成功。
    fn verified_file_copy_is_complete_and_closed_before_returning() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.zip");
        let target = dir.path().join("target.zip");
        let renamed = dir.path().join("renamed.zip");
        fs::write(&source, b"complete zip bytes").expect("source");

        copy_file_and_verify(&source, &target).expect("verified copy");
        fs::rename(&target, &renamed).expect("target handle must be closed");

        assert_eq!(
            fs::read(&renamed).expect("renamed target content"),
            b"complete zip bytes"
        );
    }

    #[test]
    /// 模擬 SMB 複製 API 宣稱已寫入完整 byte 數，實際目的檔卻仍是 0-byte。
    ///
    /// 參數：無。
    /// 回傳：無；若驗證流程錯把 0-byte 目的檔當成成功，或失敗後仍留下正式檔名，
    /// 測試就會失敗。
    /// 保護目的：重現公司 Windows 傳到 SMB 後，PaneFM 看得到檔名但 macOS 端讀到
    /// 0 KB 的問題，確保 UI 不會回報成功且 partial target 一定會被清除。
    fn smb_zero_byte_copy_is_rejected_and_partial_target_is_removed() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.zip");
        let target = dir.path().join("target.zip");
        let source_bytes = b"zip content that must reach the SMB server";
        fs::write(&source, source_bytes).expect("source");

        let result = copy_path_direct_with_cleanup(&source, &target, |source, target| {
            copy_file_and_verify_with(source, target, |_, target| {
                fs::write(target, [])?;
                Ok(source_bytes.len() as u64)
            })
        });

        let error = result.expect_err("0-byte SMB target must fail verification");
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(error.to_string().contains("stored 0"));
        assert!(!target.exists(), "failed target must not occupy its name");
    }

    #[test]
    /// 模擬資料夾傳輸建立數個 partial 子項目後失敗，驗證整棵未完成目錄會被清除。
    ///
    /// 參數：無。
    /// 回傳：無；若目標資料夾或其中任何 partial 檔案仍存在，測試就會失敗。
    /// 保護目的：避免單檔清理正常、但 SMB 資料夾貼上失敗時仍留下殘缺目錄佔用名稱。
    fn direct_directory_copy_failure_removes_partial_tree() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        fs::create_dir(&source).expect("source dir");

        let result = copy_path_direct_with_cleanup(&source, &target, |_, target| {
            fs::create_dir_all(target.join("nested"))?;
            fs::write(target.join("nested").join("partial.zip"), b"partial")?;
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "simulated SMB disconnect",
            ))
        });

        assert_eq!(
            result.expect_err("directory copy must fail").kind(),
            std::io::ErrorKind::ConnectionReset
        );
        assert!(!target.exists());
    }

    #[test]
    /// 驗證覆蓋傳輸在新內容尚未完成前不會刪除既有目標檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若模擬傳輸失敗後舊內容被刪除或改寫則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn transactional_overwrite_failure_preserves_existing_target() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.zip");
        let target = dir.path().join("target.zip");
        fs::write(&source, b"new content").expect("source");
        fs::write(&target, b"existing content").expect("existing target");

        let result = copy_path_transactional_with(&source, &target, true, |_, staged| {
            fs::write(staged, b"partial new content")?;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "simulated SMB timeout",
            ))
        });

        assert!(result.is_err());
        assert_eq!(
            fs::read(&target).expect("preserved target"),
            b"existing content"
        );
        assert!(!directory_has_transfer_temp(dir.path()));
    }

    /// 檢查測試目錄中是否仍存在交易式複製的內部暫存路徑。
    ///
    /// 參數：`path: &Path`，要掃描的測試目錄。
    /// 回傳：`bool`；找到 `.panefm-transfer-*` 名稱時回傳 `true`。
    fn directory_has_transfer_temp(path: &std::path::Path) -> bool {
        fs::read_dir(path)
            .expect("read test directory")
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".panefm-transfer-")
            })
    }

    #[test]
    /// 驗證 `PaneState` 可以依照一般名稱建立新檔案並將焦點移到新檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若檔案未建立或選取狀態錯誤則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_create_plain_file_adds_new_entry() {
        let dir = tempdir().expect("tempdir");
        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

        let created = pane.create_entry("note.txt").expect("create file");

        assert_eq!(created, "note.txt");
        assert!(dir.path().join("note.txt").exists());
        assert_eq!(
            pane.selected_entry().map(FileEntry::display_name),
            Some(String::from("note.txt"))
        );
    }

    #[test]
    /// 驗證 `PaneState` 可以依照結尾 `/` 建立新資料夾並將焦點移到新資料夾。
    ///
    /// 參數：無。
    /// 回傳：無；若資料夾未建立或選取狀態錯誤則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_create_directory_from_trailing_slash_adds_new_entry() {
        let dir = tempdir().expect("tempdir");
        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

        let created = pane.create_entry("workspace/").expect("create directory");

        assert_eq!(created, "workspace/");
        assert!(dir.path().join("workspace").is_dir());
        assert_eq!(
            pane.selected_entry().map(FileEntry::display_name),
            Some(String::from("workspace/"))
        );
    }

    #[test]
    /// 驗證巢狀建立會自動補齊父目錄，並在最後建立檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若父目錄未建立或檔案未建立則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_create_nested_file_builds_parent_directories() {
        let dir = tempdir().expect("tempdir");
        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

        let created = pane
            .create_entry("test/gg.txt")
            .expect("create nested file");

        assert_eq!(created, "test/gg.txt");
        assert!(dir.path().join("test").is_dir());
        assert!(dir.path().join("test").join("gg.txt").exists());
        assert_eq!(
            pane.selected_entry().map(FileEntry::display_name),
            Some(String::from("test/"))
        );
    }

    #[test]
    /// 驗證預設不顯示隱藏檔，切換後才會出現在列表中。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_toggle_hidden_changes_visible_entries() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".secret"), "s").expect("hidden");
        fs::write(dir.path().join("alpha.txt"), "a").expect("normal");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let initial_names: Vec<String> = pane
            .visible_entries()
            .into_iter()
            .map(FileEntry::display_name)
            .collect();
        assert_eq!(initial_names, vec![String::from("alpha.txt")]);

        pane.toggle_hidden();
        let toggled_names: Vec<String> = pane
            .visible_entries()
            .into_iter()
            .map(FileEntry::display_name)
            .collect();
        assert_eq!(
            toggled_names,
            vec![String::from(".secret"), String::from("alpha.txt")]
        );
    }

    #[test]
    /// 驗證排序模式會提供對應的人類可讀標籤。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn sort_mode_labels_match_expected_names() {
        assert_eq!(
            SortMode::Alphabetical { reverse: false }.label(),
            "alphabetical"
        );
        assert_eq!(SortMode::Size { reverse: true }.label(), "size (reverse)");
        assert_eq!(SortMode::Modified { reverse: false }.label(), "modified");
    }

    #[test]
    /// 驗證切換到大小排序後，較大的檔案會排在前面。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_sort_by_size_reorders_files() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("small.txt"), "a").expect("small");
        fs::write(dir.path().join("large.txt"), "abcdef").expect("large");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.set_sort_mode(SortMode::Size { reverse: true });

        let names: Vec<String> = pane
            .visible_entries()
            .into_iter()
            .map(FileEntry::display_name)
            .collect();
        assert_eq!(
            names,
            vec![String::from("large.txt"), String::from("small.txt")]
        );
        assert_eq!(pane.sort_mode, SortMode::Size { reverse: true });
    }

    #[test]
    /// 驗證資料夾 preview 會包含摘要資訊與部分子項目名稱。
    ///
    /// 參數：無。
    /// 回傳：無；若 preview 缺少目錄摘要或子項目清單則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_directory_preview_shows_summary_and_children() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("nested")).expect("nested dir");
        fs::write(dir.path().join("nested").join("alpha.txt"), "hello").expect("alpha");
        fs::write(dir.path().join("nested").join("beta.txt"), "world").expect("beta");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(6, Theme::default())
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(preview.iter().any(|line| line.contains("path: ")));
        assert!(preview.iter().any(|line| line.contains("items: 2")));
        assert!(preview.iter().any(|line| line == "contents:"));
        assert!(preview.iter().any(|line| line.contains("alpha.txt")));
    }

    #[test]
    /// 驗證文字檔 preview 會直接顯示帶有行號的檔案內容，不再插入額外資訊區。
    ///
    /// 參數：無。
    /// 回傳：無；若 preview 沒有顯示 metadata 或內容行號則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_file_preview_shows_metadata_and_numbered_lines() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("notes.txt"),
            "first line\nsecond line\nthird line\n",
        )
        .expect("notes");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(4, Theme::default())
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(preview.iter().any(|line| line == "  1 first line"));
        assert!(preview.iter().any(|line| line == "  2 second line"));
        assert!(!preview.iter().any(|line| line.contains("path: ")));
    }

    #[test]
    /// 驗證圖片 preview 會顯示圖片格式、尺寸與終端摘要訊息。
    ///
    /// 參數：無。
    /// 回傳：無；若圖片摘要資訊缺少格式或尺寸則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_image_preview_shows_format_and_dimensions() {
        let dir = tempdir().expect("tempdir");
        let png_bytes = vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x02, 0x80, 0x00, 0x00, 0x01, 0xE0,
        ];
        fs::write(dir.path().join("wallpaper.png"), png_bytes).expect("png");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(8, Theme::default())
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(preview.iter().any(|line| line == "format: png image"));
        assert!(preview.iter().any(|line| line == "dimensions: 640 x 480"));
    }

    #[test]
    /// 驗證常見設定檔會顯示對應的 kind 標籤，方便快速辨識檔案類型。
    ///
    /// 參數：無。
    /// 回傳：無；若 preview 沒有顯示預期的類型標籤則測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_config_preview_shows_kind_label() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("config.toml"), "theme = \"nightfox\"\n").expect("toml");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(4, Theme::default())
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(
            preview
                .iter()
                .any(|line| line == "  1 theme = \"nightfox\"")
        );
    }

    #[test]
    /// 驗證可針對指定路徑建立 preview，並套用搜尋高亮，供搜尋列表下方預覽使用。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_search_preview_for_path_supports_search_highlight() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("notes.txt");
        fs::write(&path, "alpha\nbeta target\ngamma\n").expect("notes");

        let preview = PaneState::search_preview_for_entry(
            &GlobalSearchEntry {
                path: path.clone(),
                relative_path: String::from("notes.txt"),
                is_dir: false,
                match_line_number: Some(2),
                match_column: Some(6),
                match_preview: Some(String::from("beta target")),
            },
            8,
            "target",
            None,
            None,
            false,
            Theme::default(),
        );
        let preview_text = preview
            .lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(preview_text.iter().any(|line| line == "  2 beta target"));
        assert!(preview.lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref() == "target")
        }));

        let title = PaneState::preview_title_for_path(&path, false, Some("target"));
        assert_eq!(title, "Preview: notes.txt  [/target]");
    }

    #[test]
    /// 驗證一般 preview 不會再顯示舊的資訊區，搜尋也只會針對檔案內容運作。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_preview_search_ignores_metadata_lines() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("test copy.md"), "this is body text\n").expect("notes");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.set_preview_viewport_height(6);
        pane.set_preview_search_query("t");
        pane.preview_scroll = 0;

        let preview = pane.preview_lines(12, Theme::default());
        let preview_text = preview
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert!(!preview_text.iter().any(|line| line.contains("Information")));
        assert!(!preview_text.iter().any(|line| line.contains("path: ")));
        assert!(preview.iter().any(|line| {
            line.spans.iter().any(|span| {
                span.content.as_ref() == "t"
                    && span.style.fg == Some(Theme::default().preview_match_fg)
            })
        }));
    }

    #[test]
    /// 驗證搜尋 preview 會讓所有命中維持紅字，只有目前焦點命中帶黃色背景。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_search_preview_marks_current_match_line() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("notes.txt");
        fs::write(&path, "alpha\nbeta target\ngamma target\n").expect("notes");

        let theme = Theme::default();
        let preview = PaneState::search_preview_for_entry(
            &GlobalSearchEntry {
                path: path.clone(),
                relative_path: String::from("notes.txt"),
                is_dir: false,
                match_line_number: Some(2),
                match_column: Some(6),
                match_preview: Some(String::from("beta target")),
            },
            10,
            "target",
            Some(0),
            Some(3),
            false,
            theme,
        );

        let target_spans = preview
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter(|span| span.content.as_ref() == "target")
            .collect::<Vec<_>>();

        assert_eq!(target_spans.len(), 2);
        assert!(target_spans.iter().any(|span| {
            span.style.bg == Some(theme.preview_match_bg)
                && span.style.fg == Some(theme.preview_match_fg)
        }));
        assert!(target_spans.iter().any(|span| {
            span.style.bg != Some(theme.preview_match_bg)
                && span.style.fg == Some(theme.preview_match_fg)
        }));
        assert!(preview.lines.iter().any(|line| {
            line.spans
                .iter()
                .any(|span| span.style.bg == Some(theme.preview_current_line_bg))
        }));
    }

    #[test]
    /// 驗證搜尋 preview 即使遇到大檔案，也會顯示命中片段而不是只顯示 skipped 訊息。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_search_preview_for_large_file_shows_match_snippet() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("large.txt");
        let mut content = String::new();
        for _ in 0..9000 {
            content.push_str("padding padding padding padding\n");
        }
        content.push_str("needle appears here\n");
        fs::write(&path, content).expect("large");

        let preview = PaneState::search_preview_for_entry(
            &GlobalSearchEntry {
                path: path.clone(),
                relative_path: String::from("large.txt"),
                is_dir: false,
                match_line_number: Some(9001),
                match_column: Some(1),
                match_preview: Some(String::from("needle appears here")),
            },
            8,
            "needle",
            None,
            None,
            false,
            Theme::default(),
        );

        let text = preview
            .lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        assert!(text.iter().any(|line| line.contains("needle appears here")));
        assert!(
            !text
                .iter()
                .any(|line| line.contains("preview skipped for files larger than 128 KiB"))
        );
    }

    #[test]
    /// 驗證當 cancelled 旗標被設定時，read_dir_entries_with_cancellation 會立即中斷並回傳 Interrupted。
    /// 保護目的：確保使用者在大型目錄快速切換時，舊的 worker 會提早退出，不會持續佔用磁碟 I/O。
    fn read_dir_entries_with_cancellation_aborts_promptly() {
        let dir = tempdir().expect("tempdir");
        for i in 0..10 {
            fs::write(dir.path().join(format!("file_{i}.txt")), b"data").expect("write");
        }

        let cancelled = AtomicBool::new(true);
        let result = read_dir_entries_with_cancellation(dir.path(), &cancelled);
        assert!(result.is_err());
        assert_eq!(
            result.expect_err("must error").kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    /// 驗證 stream_dir_entries_with_cancellation 會回傳正確排序且 metadata 完整的清單。
    ///
    /// 保護目的：背景導航採 natural 排序時仍須補齊檔案大小。舊實作為了縮短載入時間，
    /// 在 natural 排序的 Complete 事件也把 size 固定為 0，導致已啟用 `ms` 的舊 panel
    /// 永久顯示 0B；同一路徑另開的新 panel 卻正常。此測試同時保護排序與非零大小。
    fn stream_dir_entries_with_cancellation_loads_and_completes_sorted() {
        let dir = tempdir().expect("tempdir");
        for i in 0..300 {
            fs::write(dir.path().join(format!("file_{i:03}.txt")), b"sample").expect("write");
        }

        let cancelled = AtomicBool::new(false);
        let mut got_complete = false;

        let result = stream_dir_entries_with_cancellation(
            dir.path(),
            SortMode::Natural { reverse: false },
            0,
            &cancelled,
            |progress| {
                match progress {
                    DirectoryLoadProgress::Batch { .. } => {}
                    DirectoryLoadProgress::Complete(entries) => {
                        assert_eq!(entries.len(), 300);
                        assert_eq!(entries[0].name, "file_000.txt");
                        assert_eq!(entries[299].name, "file_299.txt");
                        assert!(
                            entries.iter().all(|entry| entry.size == 6),
                            "natural 排序的背景完整結果也必須包含真實檔案大小"
                        );
                        got_complete = true;
                    }
                }
                true
            },
        );

        assert!(result.is_ok());
        assert!(got_complete, "必須完成最終 Complete 步驟");
    }
}
