use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use ratatui::widgets::ListState;

use super::{
    PaneState,
    fs_read::read_dir_entries,
    sort::seed_from_path,
    types::{FilterMode, LineMode, SortDetailKind, SortMode},
};
use crate::file_manager::{bookmark::BookmarkTarget, entry::FileEntry, trash::TrashStore};

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
            preview_open: false,
            preview_focused: false,
            preview_diff_mode: false,
            preview_diff_cache: std::sync::Arc::new(std::sync::Mutex::new(None)),
            preview_scroll: 0,
            preview_cursor: 0,
            preview_viewport_width: 80,
            preview_viewport_height: 4,
            preview_image_cache: std::sync::Arc::new(std::sync::Mutex::new(None)),
            preview_content_cache: std::sync::Arc::new(std::sync::Mutex::new(
                crate::file_manager::preview::PreviewContentCacheMap::new(32),
            )),
            preview_image_loader: std::sync::Arc::new(std::sync::Mutex::new(None)),
            preview_search_query: None,
            preview_current_match: None,
            list_find_query: None,
            marked_paths: BTreeSet::new(),
            filter_cache: None,
            entry_revision: 0,
            vcs_info: None,
        };
        pane.reload()?;
        Ok(pane)
    }

    /// 設定此 Pane 目前目錄的 VCS 資訊。
    pub(crate) fn set_vcs_info(
        &mut self,
        info: Option<std::sync::Arc<crate::file_manager::vcs::VcsRepoInfo>>,
    ) {
        self.vcs_info = info;
    }

    /// 取得指定路徑在當前 VCS 下的檔案狀態。
    pub(crate) fn vcs_status_for_path(
        &self,
        path: &std::path::Path,
    ) -> Option<crate::file_manager::vcs::VcsFileStatus> {
        self.vcs_info
            .as_ref()
            .and_then(|info| info.status_for_path(path))
    }

    /// 取得當前目錄的 VCS 標籤（如 `git:main` 或 `svn:trunk:r42`）。
    pub(crate) fn vcs_header_label(&self) -> Option<String> {
        self.vcs_info
            .as_ref()
            .map(|info| info.format_header_label())
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
        let previous_preview_scroll = self.preview_scroll;
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
        if let Some(ref path) = selected_path {
            self.select_path(path);
            if self.selected_entry().map(|e| &e.path) == Some(path) {
                self.preview_scroll = previous_preview_scroll;
                self.clamp_preview_scroll();
            }
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
    #[allow(dead_code)]
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
        self.preview_scroll = 0;
        self.filter_query = None;
        self.entries = Vec::new();
        self.bump_entry_revision();
        self.sort_entries();
        self.refresh_visible_entries();
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
        let previous_preview_scroll = self.preview_scroll;
        let previous_preview_cursor = self.preview_cursor;
        let preview_active = self.preview_open;
        let preview_path = if preview_active {
            self.selected_entry().map(|entry| entry.path.clone())
        } else {
            None
        };
        self.entries = entries;
        self.bump_entry_revision();
        self.sort_entries();
        self.refresh_visible_entries();
        if let Some(path) = selected_path {
            self.select_path(path);
        } else if let Some(ref path) = preview_path {
            self.select_path(path);
            if self.selected_entry().map(|e| &e.path) == Some(path) {
                self.preview_scroll = previous_preview_scroll;
                self.preview_cursor = previous_preview_cursor;
                self.clamp_preview_scroll();
            }
        }
    }

    /// 套用已在背景完成排序的清單，跳過主 UI 執行緒的排序運算。
    pub(crate) fn replace_entries_presorted(
        &mut self,
        entries: Vec<FileEntry>,
        selected_path: Option<&Path>,
    ) {
        let previous_preview_scroll = self.preview_scroll;
        let previous_preview_cursor = self.preview_cursor;
        let preview_active = self.preview_open;
        let preview_path = if preview_active {
            self.selected_entry().map(|entry| entry.path.clone())
        } else {
            None
        };
        self.entries = entries;
        self.bump_entry_revision();
        self.refresh_visible_entries();
        if let Some(path) = selected_path {
            self.select_path(path);
        } else if let Some(ref path) = preview_path {
            self.select_path(path);
            if self.selected_entry().map(|e| &e.path) == Some(path) {
                self.preview_scroll = previous_preview_scroll;
                self.preview_cursor = previous_preview_cursor;
                self.clamp_preview_scroll();
            }
        }
    }

    /// 增量追加載入中的目錄項目，並在保留目前可見游標索引的前提下即時更新畫面。
    pub(crate) fn extend_entries(&mut self, new_entries: Vec<FileEntry>) {
        let previous_preview_scroll = self.preview_scroll;
        let previous_preview_cursor = self.preview_cursor;
        let preview_active = self.preview_open;
        let preview_path = if preview_active {
            self.selected_entry().map(|entry| entry.path.clone())
        } else {
            None
        };
        self.entries.extend(new_entries);
        self.bump_entry_revision();
        self.refresh_visible_entries();
        if let Some(ref path) = preview_path {
            self.select_path(path);
            if self.selected_entry().map(|e| &e.path) == Some(path) {
                self.preview_scroll = previous_preview_scroll;
                self.preview_cursor = previous_preview_cursor;
                self.clamp_preview_scroll();
            }
        }
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

    /// 跳到指定路徑所在的目錄，並將選取游標聚焦到該路徑上。
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
