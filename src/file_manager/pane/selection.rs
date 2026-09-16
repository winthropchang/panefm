use std::path::Path;

use super::PaneState;
use crate::file_manager::entry::FileEntry;

impl PaneState {
    pub(crate) fn move_up_by(&mut self, count: usize) {
        if self.visible_indices.is_empty() {
            return;
        }
        let new_selected = self.selected.saturating_sub(count.max(1));
        if self.selected != new_selected {
            self.selected = new_selected;
            self.list_state.select(Some(self.selected));
            self.preview_scroll = 0;
            self.preview_cursor = 0;
        }
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
        let new_selected =
            (self.selected + count.max(1)).min(self.visible_indices.len().saturating_sub(1));
        if self.selected != new_selected {
            self.selected = new_selected;
            self.list_state.select(Some(self.selected));
            self.preview_scroll = 0;
            self.preview_cursor = 0;
        }
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
        if self.selected != 0 {
            self.selected = 0;
            self.list_state.select(Some(self.selected));
            self.preview_scroll = 0;
            self.preview_cursor = 0;
        }
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
        let target = self.visible_indices.len() - 1;
        if self.selected != target {
            self.selected = target;
            self.list_state.select(Some(self.selected));
            self.preview_scroll = 0;
            self.preview_cursor = 0;
        }
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
        let target = index.min(self.visible_indices.len().saturating_sub(1));
        if self.selected != target {
            self.selected = target;
            self.list_state.select(Some(self.selected));
            self.preview_scroll = 0;
            self.preview_cursor = 0;
        }
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
    /// 將選取狀態移到指定路徑，方便在建立或貼上後立刻聚焦新項目。
    pub(crate) fn select_path(&mut self, path: &Path) {
        let previous_selected_path = self.selected_entry().map(|entry| entry.path.clone());
        if let Some(index) = self.visible_indices.iter().position(|visible_index| {
            self.entries
                .get(*visible_index)
                .map(|candidate| candidate.path == path)
                .unwrap_or(false)
        }) {
            self.selected = index;
            self.list_state.select(Some(index));
        }
        if previous_selected_path.as_deref() != Some(path) {
            self.preview_scroll = 0;
            self.preview_search_query = None;
            self.preview_current_match = None;
        } else {
            self.clamp_preview_scroll();
        }
    }
}
