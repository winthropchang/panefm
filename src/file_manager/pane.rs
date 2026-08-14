use std::{
    cmp::Ordering,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    fs, io,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use ratatui::{text::Line, widgets::ListState};

use super::entry::FileEntry;

/// 表示單一 pane 的完整瀏覽狀態。
///
/// 每個 pane 都獨立維護自己的目錄、游標與列表狀態，
/// 這樣分割視窗後每個區塊才可以各自操作。
#[derive(Debug)]
pub(crate) struct PaneState {
    /// 目前 pane 正在瀏覽的目錄。
    pub(crate) cwd: PathBuf,
    /// 目前目錄下的檔案與資料夾清單。
    pub(crate) entries: Vec<FileEntry>,
    /// 目前選取項目的索引位置。
    pub(crate) selected: usize,
    /// `ratatui` 的列表狀態，供畫面渲染使用。
    pub(crate) list_state: ListState,
    /// 目前啟用中的過濾字串，`None` 代表沒有啟用 filter。
    pub(crate) filter_query: Option<String>,
    /// 目前實際顯示在列表中的項目索引。
    pub(crate) visible_indices: Vec<usize>,
    /// 是否顯示以 `.` 開頭的隱藏檔案與資料夾。
    pub(crate) show_hidden: bool,
    /// 目前使用中的排序模式。
    pub(crate) sort_mode: SortMode,
    /// 隨機排序時使用的種子，讓每次重新套用時都能洗牌。
    pub(crate) random_seed: u64,
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

/// 描述列表右側欄位目前應該顯示哪一種排序依據。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortDetailKind {
    None,
    Size,
    Modified,
    Created,
    Extension,
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
            cwd,
            entries: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
            filter_query: None,
            visible_indices: Vec::new(),
            show_hidden: false,
            sort_mode: SortMode::Natural { reverse: false },
            random_seed,
        };
        pane.reload()?;
        Ok(pane)
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
        self.sort_entries();
        self.refresh_visible_entries();
        Ok(())
    }

    /// 將列表選取游標向上移動一格。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被移動游標的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn move_up(&mut self) {
        if self.visible_indices.is_empty() {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        self.list_state.select(Some(self.selected));
    }

    /// 將列表選取游標向下移動一格。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被移動游標的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn move_down(&mut self) {
        if self.visible_indices.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.visible_indices.len().saturating_sub(1));
        self.list_state.select(Some(self.selected));
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
        if let Some(entry) = self.selected_entry()
            && entry.is_dir
        {
            self.cwd = entry.path.clone();
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
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.selected = 0;
            self.filter_query = None;
            self.reload()?;
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
    pub(crate) fn preview_lines(&self, max_lines: usize) -> Vec<Line<'static>> {
        match self.selected_entry() {
            Some(entry) if entry.is_dir => preview_directory(entry, max_lines),
            Some(entry) => preview_file(&entry.path, max_lines),
            None => vec![Line::from("empty directory")],
        }
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

        if let Some(index) = self
            .visible_indices
            .iter()
            .position(|visible_index| {
                self.entries
                    .get(*visible_index)
                    .map(|candidate| candidate.path == new_path)
                    .unwrap_or(false)
            })
        {
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
    pub(crate) fn copy_entry_into_current_dir(&mut self, source_path: &Path) -> io::Result<String> {
        let display_name = copy_path_into_dir(source_path, &self.cwd)?;
        self.reload()?;

        let pasted_path = self.cwd.join(trimmed_display_name(&display_name));
        if let Some(index) = self
            .visible_indices
            .iter()
            .position(|visible_index| {
                self.entries
                    .get(*visible_index)
                    .map(|candidate| candidate.path == pasted_path)
                    .unwrap_or(false)
            })
        {
            self.selected = index;
            self.list_state.select(Some(index));
        }

        Ok(display_name)
    }

    /// 將外部來源的檔案或資料夾移動到目前 pane 的目錄中。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要接收新項目的目標 pane。
    /// - `source_path: &Path`，原始檔案或資料夾路徑。
    ///
    /// 回傳：`io::Result<String>`。
    /// - 成功時回傳貼上後的顯示名稱。
    /// - 失敗時回傳檔案系統操作錯誤，例如目標已存在或來源不可用。
    pub(crate) fn move_entry_into_current_dir(&mut self, source_path: &Path) -> io::Result<String> {
        let display_name = move_path_into_dir(source_path, &self.cwd)?;
        self.reload()?;

        let moved_path = self.cwd.join(trimmed_display_name(&display_name));
        if let Some(index) = self
            .visible_indices
            .iter()
            .position(|visible_index| {
                self.entries
                    .get(*visible_index)
                    .map(|candidate| candidate.path == moved_path)
                    .unwrap_or(false)
            })
        {
            self.selected = index;
            self.list_state.select(Some(index));
        }

        Ok(display_name)
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

    /// 判斷目前是否仍處於過濾後的列表狀態。
    pub(crate) fn has_active_filter(&self) -> bool {
        self.filter_query.is_some()
    }

    /// 切換目前 pane 是否顯示隱藏檔。
    pub(crate) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh_visible_entries();
    }

    /// 切換到下一個排序模式，並立即重排目前列表。
    pub(crate) fn set_sort_mode(&mut self, sort_mode: SortMode) {
        self.sort_mode = sort_mode;
        if matches!(sort_mode, SortMode::Random) {
            self.random_seed = self.random_seed.wrapping_add(1);
        }
        self.sort_entries();
        self.refresh_visible_entries();
    }

    /// 重新計算目前實際應該顯示的項目與選取位置。
    fn refresh_visible_entries(&mut self) {
        self.visible_indices = match &self.filter_query {
            Some(query) => {
                let query = query.to_lowercase();
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| self.show_hidden || !is_hidden_name(&entry.name))
                    .filter(|(_, entry)| entry.name.to_lowercase().contains(&query))
                    .map(|(index, _)| index)
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
            self.selected = self.selected.min(self.visible_indices.len().saturating_sub(1));
            self.list_state.select(Some(self.selected));
        }
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
                for name in child_names.into_iter().take(max_lines.saturating_sub(lines.len())) {
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
    let mut lines = vec![
        Line::from(format!("path: {}", path.display())),
        Line::from(format!("size: {}", format_size_preview(metadata.len()))),
    ];

    if let Ok(modified) = metadata.modified() {
        lines.push(Line::from(format!(
            "modified: {}",
            format_system_time_preview(modified)
        )));
    }

    if max_lines <= lines.len() {
        lines.truncate(max_lines);
        return lines;
    }

    if let Some(kind_label) = preview_kind_label(extension.as_deref()) {
        lines.push(Line::from(format!("kind: {kind_label}")));
    }

    if metadata.len() > 128 * 1024 {
        lines.push(Line::from("preview skipped for files larger than 128 KiB"));
        lines.truncate(max_lines);
        return lines;
    }

    let Ok(bytes) = fs::read(path) else {
        lines.push(Line::from("unable to read file contents"));
        lines.truncate(max_lines);
        return lines;
    };

    if let Some(image_summary) = preview_image_summary(&bytes, extension.as_deref()) {
        lines.extend(image_summary.into_iter().map(Line::from));
        lines.truncate(max_lines);
        return lines;
    }

    match String::from_utf8(bytes) {
        Ok(contents) => {
            lines.push(Line::from(""));

            let available_content_lines = max_lines.saturating_sub(lines.len());
            if available_content_lines == 0 {
                return lines;
            }

            let content_lines: Vec<&str> = contents.lines().collect();
            if content_lines.is_empty() {
                lines.push(Line::from("[empty file]"));
                lines.truncate(max_lines);
                return lines;
            }

            let truncated = content_lines.len() > available_content_lines;
            for (index, line) in content_lines.into_iter().take(available_content_lines).enumerate()
            {
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
            if let Some(binary_label) = preview_binary_label(extension.as_deref()) {
                lines.push(Line::from(format!("format: {binary_label}")));
            }
            lines.push(Line::from("binary or non-utf8 file"));
            lines.truncate(max_lines);
            lines
        }
    }
}

/// 依照副檔名回傳較適合顯示在 preview 的檔案類型標籤。
fn preview_kind_label(extension: Option<&str>) -> Option<&'static str> {
    match extension.unwrap_or_default() {
        "rs" => Some("rust source"),
        "toml" => Some("toml config"),
        "json" => Some("json data"),
        "md" | "markdown" => Some("markdown document"),
        "yml" | "yaml" => Some("yaml config"),
        "txt" | "log" => Some("text file"),
        "sh" | "bash" | "zsh" => Some("shell script"),
        "js" => Some("javascript source"),
        "ts" => Some("typescript source"),
        "py" => Some("python source"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" => Some("image"),
        "zip" => Some("archive"),
        "pdf" => Some("pdf document"),
        _ => None,
    }
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
            0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF
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

/// 將檔案大小格式化成適合 preview 區閱讀的字串。
fn format_size_preview(size: u64) -> String {
    match size {
        0..=1023 => format!("{size} b"),
        1024..=1_048_575 => format!("{:.1} kb", size as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} mb", size as f64 / 1_048_576.0),
        _ => format!("{:.1} G", size as f64 / 1_073_741_824.0),
    }
}

/// 將系統時間格式化成 preview 區較容易閱讀的時間字串。
fn format_system_time_preview(value: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(value)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// 將單一路徑複製到目標資料夾，支援檔案與整個資料夾樹。
///
/// 參數：
/// - `source_path: &Path`，來源檔案或資料夾。
/// - `target_dir: &Path`，貼上目標資料夾。
///
/// 回傳：`io::Result<String>`，成功時回傳可顯示的名稱。
fn copy_path_into_dir(source_path: &Path, target_dir: &Path) -> io::Result<String> {
    let file_name = source_path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name"))?;
    let target_path = unique_target_path(target_dir, file_name);

    if source_path.is_dir() {
        copy_dir_recursive(source_path, &target_path)?;
        Ok(format!("{}/", target_path_file_name(&target_path)))
    } else {
        fs::copy(source_path, &target_path)?;
        Ok(target_path_file_name(&target_path))
    }
}

/// 將單一路徑移動到目標資料夾。
///
/// 參數：
/// - `source_path: &Path`，來源檔案或資料夾。
/// - `target_dir: &Path`，貼上目標資料夾。
///
/// 回傳：`io::Result<String>`，成功時回傳可顯示的名稱。
fn move_path_into_dir(source_path: &Path, target_dir: &Path) -> io::Result<String> {
    let file_name = source_path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name"))?;
    let target_path = unique_target_path(target_dir, file_name);

    fs::rename(source_path, &target_path)?;

    if target_path.is_dir() {
        Ok(format!("{}/", target_path_file_name(&target_path)))
    } else {
        Ok(target_path_file_name(&target_path))
    }
}

/// 遞迴複製整個資料夾，保留所有子目錄與檔案。
///
/// 參數：
/// - `source_dir: &Path`，來源資料夾。
/// - `target_dir: &Path`，目標資料夾。
///
/// 回傳：`io::Result<()>`。
fn copy_dir_recursive(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    fs::create_dir(target_dir)?;

    for item in fs::read_dir(source_dir)? {
        let item = item?;
        let item_path = item.path();
        let next_target = target_dir.join(item.file_name());

        if item.file_type()?.is_dir() {
            copy_dir_recursive(&item_path, &next_target)?;
        } else {
            fs::copy(&item_path, &next_target)?;
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
fn duplicate_name(base_name: &str, extension: Option<&str>, duplicate_index: Option<usize>) -> String {
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

/// 將畫面顯示用的名稱轉回實際檔案名稱，移除資料夾顯示用的尾端 `/`。
///
/// 參數：
/// - `display_name: &str`，畫面上顯示的檔名。
///
/// 回傳：`&str`，可用來組合實際路徑的名稱。
fn trimmed_display_name(display_name: &str) -> &str {
    display_name.trim_end_matches('/')
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
        entries.push(FileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: item.path(),
            is_dir: file_type.is_dir(),
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            created: metadata.created().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{PaneState, SortMode};
    use crate::file_manager::entry::FileEntry;

    #[test]
    /// 驗證 pane 重新載入目錄時，資料夾會排在檔案前面。
    ///
    /// 參數：無。
    /// 回傳：無；若排序規則錯誤則測試失敗。
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
    /// 驗證 pane 可以正確進入子目錄並返回父目錄。
    ///
    /// 參數：無。
    /// 回傳：無；若目錄切換行為錯誤則測試失敗。
    fn pane_state_enters_and_leaves_directories() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        fs::create_dir(&child).expect("child dir");
        fs::write(child.join("note.txt"), "hello").expect("note");

        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.enter_selected().expect("enter child");
        assert_eq!(pane.cwd, child);

        pane.go_parent().expect("back parent");
        assert_eq!(pane.cwd, dir.path());
    }

    #[test]
    /// 驗證 `PaneState` 可以正確刪除目前選取的檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若檔案未被刪除或狀態未更新則測試失敗。
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
    /// 驗證同一個目錄內重複複製資料夾時，也會自動產生不衝突的新名稱。
    ///
    /// 參數：無。
    /// 回傳：無；若資料夾重複名稱處理錯誤則測試失敗。
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
    /// 驗證 `PaneState` 可以依照一般名稱建立新檔案並將焦點移到新檔案。
    ///
    /// 參數：無。
    /// 回傳：無；若檔案未建立或選取狀態錯誤則測試失敗。
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
    fn pane_state_create_directory_from_trailing_slash_adds_new_entry() {
        let dir = tempdir().expect("tempdir");
        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

        let created = pane
            .create_entry("workspace/")
            .expect("create directory");

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
    fn sort_mode_labels_match_expected_names() {
        assert_eq!(SortMode::Alphabetical { reverse: false }.label(), "alphabetical");
        assert_eq!(SortMode::Size { reverse: true }.label(), "size (reverse)");
        assert_eq!(SortMode::Modified { reverse: false }.label(), "modified");
    }

    #[test]
    /// 驗證切換到大小排序後，較大的檔案會排在前面。
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
    fn pane_state_directory_preview_shows_summary_and_children() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("nested")).expect("nested dir");
        fs::write(dir.path().join("nested").join("alpha.txt"), "hello").expect("alpha");
        fs::write(dir.path().join("nested").join("beta.txt"), "world").expect("beta");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(6)
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(preview.iter().any(|line| line.starts_with("path: ")));
        assert!(preview.iter().any(|line| line == "items: 2"));
        assert!(preview.iter().any(|line| line == "contents:"));
        assert!(preview.iter().any(|line| line.contains("alpha.txt")));
    }

    #[test]
    /// 驗證文字檔 preview 會顯示檔案資訊與帶有行號的內容。
    ///
    /// 參數：無。
    /// 回傳：無；若 preview 沒有顯示 metadata 或內容行號則測試失敗。
    fn pane_state_file_preview_shows_metadata_and_numbered_lines() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("notes.txt"),
            "first line\nsecond line\nthird line\n",
        )
        .expect("notes");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(8)
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(preview.iter().any(|line| line.starts_with("path: ")));
        assert!(preview.iter().any(|line| line.starts_with("size: ")));
        assert!(preview.iter().any(|line| line.starts_with("modified: ")));
        assert!(preview.iter().any(|line| line == "  1 first line"));
        assert!(preview.iter().any(|line| line == "  2 second line"));
    }

    #[test]
    /// 驗證圖片 preview 會顯示圖片格式、尺寸與終端摘要訊息。
    ///
    /// 參數：無。
    /// 回傳：無；若圖片摘要資訊缺少格式或尺寸則測試失敗。
    fn pane_state_image_preview_shows_format_and_dimensions() {
        let dir = tempdir().expect("tempdir");
        let png_bytes = vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I',
            b'H', b'D', b'R', 0x00, 0x00, 0x02, 0x80, 0x00, 0x00, 0x01, 0xE0,
        ];
        fs::write(dir.path().join("wallpaper.png"), png_bytes).expect("png");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(8)
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(preview.iter().any(|line| line == "kind: image"));
        assert!(preview.iter().any(|line| line == "format: png image"));
        assert!(preview.iter().any(|line| line == "dimensions: 640 x 480"));
    }

    #[test]
    /// 驗證常見設定檔會顯示對應的 kind 標籤，方便快速辨識檔案類型。
    ///
    /// 參數：無。
    /// 回傳：無；若 preview 沒有顯示預期的類型標籤則測試失敗。
    fn pane_state_config_preview_shows_kind_label() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("config.toml"), "theme = \"nightfox\"\n").expect("toml");

        let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        let preview: Vec<String> = pane
            .preview_lines(6)
            .into_iter()
            .map(|line| line.to_string())
            .collect();

        assert!(preview.iter().any(|line| line == "kind: toml config"));
        assert!(preview.iter().any(|line| line == "  1 theme = \"nightfox\""));
    }
}
