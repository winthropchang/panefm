use std::{
    fs, io,
    path::{Path, PathBuf},
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
}
