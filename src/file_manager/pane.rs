use std::{
    fs, io,
    path::{Component, Path, PathBuf},
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
        let mut pane = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            list_state: ListState::default(),
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
        if self.entries.is_empty() {
            self.selected = 0;
            self.list_state.select(None);
        } else {
            self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            self.list_state.select(Some(self.selected));
        }
        Ok(())
    }

    /// 將列表選取游標向上移動一格。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被移動游標的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn move_up(&mut self) {
        if self.entries.is_empty() {
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
        if self.entries.is_empty() {
            return;
        }
        self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
        self.list_state.select(Some(self.selected));
    }

    /// 將列表選取游標跳到最上方。
    ///
    /// 參數：
    /// - `self: &mut PaneState`，要被更新的 pane。
    ///
    /// 回傳：`()`
    pub(crate) fn move_top(&mut self) {
        if self.entries.is_empty() {
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
        if self.entries.is_empty() {
            return;
        }
        self.selected = self.entries.len() - 1;
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
        self.entries.get(self.selected)
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
            Some(entry) if entry.is_dir => vec![
                Line::from(format!("dir: {}", entry.path.display())),
                Line::from(format!("items: {}", count_items(&entry.path))),
            ],
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
            .entries
            .iter()
            .position(|candidate| candidate.path == new_path)
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
            .entries
            .iter()
            .position(|candidate| candidate.path == pasted_path)
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
            .entries
            .iter()
            .position(|candidate| candidate.path == moved_path)
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
        if let Some(index) = self.entries.iter().position(|candidate| candidate.path == path) {
            self.selected = index;
            self.list_state.select(Some(index));
        }
    }
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

    if metadata.len() > 128 * 1024 {
        return vec![
            Line::from(format!("file: {}", path.display())),
            Line::from(format!("size: {} bytes", metadata.len())),
            Line::from("preview skipped for files larger than 128 KiB"),
        ];
    }

    match fs::read_to_string(path) {
        Ok(contents) => contents
            .lines()
            .take(max_lines.max(1))
            .map(|line| Line::from(line.to_owned()))
            .collect(),
        Err(_) => vec![
            Line::from(format!("file: {}", path.display())),
            Line::from(format!("size: {} bytes", metadata.len())),
            Line::from("binary or non-utf8 file"),
        ],
    }
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
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::PaneState;
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
}
