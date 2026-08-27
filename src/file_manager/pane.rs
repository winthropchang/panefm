//! 單一 panel 的目錄列表、排序、選取、預覽與檔案操作狀態。
//!
//! `PaneState` 是 PaneFM 的第一級物件：每個 split 都有獨立 cwd、游標、filter、
//! preview 與顯示模式。這一層不決定快捷鍵，也不繪製 popup；它提供可測試的資料
//! 操作給 `App`，並在變更檔案後重新載入列表與盡可能保留游標位置。

use std::{
    cmp::Ordering,
    collections::BTreeSet,
    collections::hash_map::DefaultHasher,
    fs::{self, File, OpenOptions},
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    time::SystemTime,
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

/// 表示單一 pane 的完整瀏覽狀態。
///
/// 每個 pane 都獨立維護自己的目錄、游標與列表狀態，
/// 這樣分割視窗後每個區塊才可以各自操作。
#[derive(Debug)]
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
        self.entries = read_dir_entries(&self.cwd)?;
        if matches!(self.active_detail_kind(), SortDetailKind::Size) {
            self.load_directory_child_counts();
        }
        self.marked_paths
            .retain(|path| self.entries.iter().any(|entry| &entry.path == path));
        self.sort_entries();
        self.refresh_visible_entries();
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

    /// 回到目前目錄的上一層。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要切換到父目錄的 pane。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已回到父目錄或目前已無父目錄。
    /// - 失敗時代表重新載入父目錄內容時發生錯誤。
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
    fn select_path(&mut self, path: &Path) {
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

    /// 套用新的 filter 字串，並立即更新可見清單。
    pub(crate) fn set_filter_query(&mut self, query: &str) {
        let trimmed = query.trim();
        self.filter_query = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
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

    /// 只在 size 欄位真正需要顯示時，才讀取各子目錄的直接項目數量。
    ///
    /// 參數：`self: &mut PaneState`，目前要補齊顯示資料的 pane。
    /// 回傳：`() `；無法讀取的子目錄會保留 `None`，由 UI 顯示 `?`。
    ///
    /// 這項操作刻意不放在一般 `reload()` 的預設路徑，因為 Windows SMB
    /// 每開啟一個子目錄都可能產生一次網路往返，會讓首次進入目錄明顯卡頓。
    pub(crate) fn load_directory_child_counts(&mut self) {
        for entry in &mut self.entries {
            if entry.is_dir && entry.child_count.is_none() {
                entry.child_count = count_directory_children(&entry.path).ok();
            }
        }
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
                let candidates: Vec<usize> = self
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| self.show_hidden || !is_hidden_name(&entry.name))
                    .map(|(index, _)| index)
                    .collect();
                fuzzy_matched_indices(&candidates, query, |index| {
                    self.entries[*index].name.clone()
                })
                .into_iter()
                .map(|matched_index| candidates[matched_index])
                .collect()
            }
            None => self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| self.show_hidden || !is_hidden_name(&entry.name))
                .map(|(index, _)| index)
                .collect(),
        };

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
        let sort_mode = self.sort_mode;
        let random_seed = self.random_seed;
        self.entries.sort_by(|left, right| {
            if matches!(sort_mode, SortMode::Random) {
                match (left.is_dir, right.is_dir) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => random_key(left, random_seed).cmp(&random_key(right, random_seed)),
                }
            } else {
                compare_entries(left, right, sort_mode)
            }
        });
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
                    left.name.to_lowercase().cmp(&right.name.to_lowercase()),
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
                        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())),
                    reverse,
                ),
                SortMode::Random => Ordering::Equal,
            };

            if primary == Ordering::Equal {
                left.name.to_lowercase().cmp(&right.name.to_lowercase())
            } else {
                primary
            }
        }
    }
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
    let left_parts = split_natural_parts(left);
    let right_parts = split_natural_parts(right);

    for (left_part, right_part) in left_parts.iter().zip(right_parts.iter()) {
        let ordering = match (left_part.parse::<u64>(), right_part.parse::<u64>()) {
            (Ok(left_number), Ok(right_number)) => left_number.cmp(&right_number),
            _ => left_part.to_lowercase().cmp(&right_part.to_lowercase()),
        };

        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left_parts.len().cmp(&right_parts.len())
}

/// 將名稱拆成數字與文字片段，供自然排序比較使用。
fn split_natural_parts(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_digits = None;

    for character in value.chars() {
        let is_digit = character.is_ascii_digit();
        match in_digits {
            Some(flag) if flag == is_digit => current.push(character),
            Some(_) => {
                parts.push(std::mem::take(&mut current));
                current.push(character);
                in_digits = Some(is_digit);
            }
            None => {
                current.push(character);
                in_digits = Some(is_digit);
            }
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
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
/// 參數：
/// - `path: &Path`，要計算內容數量的資料夾路徑。
///
/// 回傳：`usize`，讀取成功時為項目數量，失敗時回傳 `0`。
fn count_items(path: &Path) -> usize {
    fs::read_dir(path).map(|iter| iter.count()).unwrap_or(0)
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

    let remaining = max_lines.saturating_sub(lines.len());
    if remaining == 0 {
        return lines;
    }

    match fs::read_dir(&entry.path) {
        Ok(read_dir) => {
            let child_names: Vec<String> = read_dir
                .filter_map(|child| child.ok())
                .map(|child| child.file_name().to_string_lossy().to_string())
                .take(remaining)
                .collect();

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

/// 直接複製到正式目標，失敗時清除本次建立的部分內容。
///
/// 一般貼上採用這條路徑，與 mature-reference 在 macOS/Windows 的本機檔案引擎一致，
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
    let file_name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
    })?;
    let target_path = target_path_for_paste(source_path, target_dir, file_name, overwrite)?;

    let backup_path = (overwrite && target_path.exists())
        .then(|| unique_transfer_path(&target_path, "undo-backup"));
    if let Some(backup_path) = &backup_path {
        fs::rename(&target_path, backup_path)?;
    }
    if let Err(error) = fs::rename(source_path, &target_path) {
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

/// 複製單一檔案，強制送出緩衝資料，並在關閉後重新確認目標大小。
///
/// `fs::copy` 會使用 Rust 標準函式庫針對目前平台提供的檔案複製實作，比自行用
/// `io::copy` 串流更能遵守 Windows UNC/SMB 與 macOS 掛載磁碟的原生語意。複製 API
/// 回傳後仍會重新開啟目標並執行 `sync_all`，最後只有來源大小、API 回報寫入量與
/// 目標大小完全一致才算成功。
///
/// 參數：
/// - `source_path: &Path`，來源檔案。
/// - `staged_path: &Path`，要寫入的目標檔；可能是正式名稱或交易式暫存名稱。
///
/// 回傳：`io::Result<()>`；成功代表資料已同步、寫入 handle 已關閉，且兩端大小一致。
fn copy_file_and_verify(source_path: &Path, staged_path: &Path) -> io::Result<()> {
    copy_file_and_verify_with(source_path, staged_path, |source, target| {
        fs::copy(source, target)
    })
}

/// 執行可注入平台複製器的單檔驗證核心，供 SMB 不完整寫入情境做回歸測試。
///
/// 參數：
/// - `source_path: &Path`，來源檔案。
/// - `staged_path: &Path`，本次新建立的目標檔案。
/// - `platform_copy: F`，平台複製函數，型別為
///   `FnOnce(&Path, &Path) -> io::Result<u64>`，回傳宣稱已複製的 byte 數。
///
/// 回傳：`io::Result<()>`；只有平台複製、同步及關閉後大小驗證全部成功才回傳 `Ok`。
fn copy_file_and_verify_with<F>(
    source_path: &Path,
    staged_path: &Path,
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

    let expected_size = File::open(source_path)?.metadata()?.len();
    let copied_size = platform_copy(source_path, staged_path)?;

    // 平台 copy 已關閉它自己的寫入 handle；重新開啟後同步，避免只把資料留在
    // Windows redirector 或 SMB client cache，卻先讓 UI 誤以為傳輸完成。
    let target_file = OpenOptions::new().write(true).open(staged_path)?;
    target_file.sync_all()?;
    drop(target_file);

    let source_size_after_copy = File::open(source_path)?.metadata()?.len();
    let stored_size = File::open(staged_path)?.metadata()?.len();
    if copied_size != expected_size
        || source_size_after_copy != expected_size
        || stored_size != expected_size
    {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "incomplete copy: expected {expected_size} bytes, source now {source_size_after_copy}, copied {copied_size}, stored {stored_size}"
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

/// 讀取指定目錄，並整理成可顯示的檔案項目清單。
///
/// 參數：
/// - `path: &Path`，要掃描的目錄路徑。
///
/// 回傳：`io::Result<Vec<FileEntry>>`。
/// - 成功時回傳已排序的檔案與資料夾清單。
/// - 失敗時回傳讀取目錄或 metadata 時的 I/O 錯誤。
fn read_dir_entries(path: &Path) -> io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    for item in fs::read_dir(path)? {
        let item = item?;
        let file_type = item.file_type()?;
        let metadata = item.metadata()?;
        let entry_path = item.path();
        entries.push(FileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: entry_path.clone(),
            is_dir: file_type.is_dir(),
            size: metadata.len(),
            // 子目錄數量需要額外開啟每一個資料夾；一般列表先延遲讀取。
            child_count: None,
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            created: metadata.created().unwrap_or(SystemTime::UNIX_EPOCH),
            readonly: metadata.permissions().readonly(),
            unix_mode: read_unix_mode(&metadata),
        });
    }

    Ok(entries)
}

/// 讀取單一目錄的直接子項目數量，供 linemode size 在資料夾時顯示。
///
/// 參數：
/// - `path: &Path`，要統計的目錄路徑。
///
/// 回傳：`io::Result<usize>`。
/// - 成功時回傳目前目錄直接包含的子項目數量。
/// - 失敗時回傳讀取子目錄時發生的 I/O 錯誤。
fn count_directory_children(path: &Path) -> io::Result<usize> {
    Ok(fs::read_dir(path)?.filter_map(Result::ok).count())
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
    use std::fs;

    use tempfile::tempdir;

    use super::{
        PaneState, SortMode, copy_file_and_verify, copy_file_and_verify_with,
        copy_path_direct_with_cleanup, copy_path_transactional_with,
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
    /// 驗證一般目錄載入不會預先打開每個子目錄統計數量，避免 SMB 首次瀏覽產生額外網路 I/O。
    ///
    /// 參數：無。
    /// 回傳：無；若預設載入已填入 `child_count`，測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_defers_directory_child_counts_until_requested() {
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("one.txt"), "one").expect("first child");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");

        assert_eq!(directory.child_count, None);
    }

    #[test]
    /// 驗證 size linemode 需要資料時，仍可延遲取得正確的子目錄項目數量。
    ///
    /// 參數：無。
    /// 回傳：無；若延遲載入後數量不正確，測試失敗。
    /// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
    fn pane_state_loads_directory_child_counts_on_demand() {
        let dir = tempdir().expect("tempdir");
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).expect("nested dir");
        fs::write(nested.join("one.txt"), "one").expect("first child");
        fs::write(nested.join("two.txt"), "two").expect("second child");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.load_directory_child_counts();
        let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");

        assert_eq!(directory.child_count, Some(2));
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
}
