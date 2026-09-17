use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use super::super::*;

impl App {
    /// 將 filter 文字套用到指定 pane。
    pub(crate) fn apply_filter_buffer(&mut self, filter: &FilterState) {
        if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
            pane.set_filter_query(&filter.buffer, filter.mode);
        }
    }

    /// 判斷目前畫面上是否有正在輸入中的文字框。
    pub(crate) fn is_text_input_active(&self) -> bool {
        if self.command_mode {
            return true;
        }
        if self.filter.as_ref().is_some_and(|filter| filter.editing) {
            return true;
        }
        if self
            .preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
        {
            return true;
        }
        if self.list_find.is_some() {
            return true;
        }
        if self
            .global_search
            .as_ref()
            .is_some_and(|s| s.editing || s.filter.editing)
        {
            return true;
        }
        matches!(
            self.pending_action,
            Some(
                PendingAction::Rename { .. }
                    | PendingAction::CreateEntry { .. }
                    | PendingAction::RegexRename { .. }
                    | PendingAction::TrashPanel {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::HelpPanel {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::TaskPanel {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::BookmarkList {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::ZoxideList {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
            )
        )
    }

    /// 將 preview search 文字套用到指定 pane，並讓 preview 跳到命中位置。
    pub(crate) fn apply_preview_search_buffer(&mut self, search: &PreviewSearchState) {
        if let Some(pane) = self.panes.get_mut(&search.pane_id) {
            pane.set_preview_search_query(&search.buffer);
        }
    }

    /// 將列表內 find-next 文字套用到指定 pane，並把游標移到第一個命中項目。
    pub(crate) fn apply_list_find_buffer(&mut self, search: &ListFindState) {
        if let Some(pane) = self.panes.get_mut(&search.pane_id) {
            pane.set_list_find_query(&search.buffer);
            if pane.has_list_find()
                && let Some(target) = pane.list_find_match_indices().first().copied()
            {
                pane.selected = target;
                pane.list_state.select(Some(target));
                pane.preview_scroll = 0;
            }
        }
    }

    /// 啟動一個背景 global search 工作，避免在大型目錄中阻塞主介面。
    pub(crate) fn start_global_search(&mut self, search: &mut GlobalSearchState) -> io::Result<()> {
        if let Some(task_id) = search.task_id.take() {
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("replaced by new query"),
            );
        }
        self.cancel_global_search_worker();

        let pane_id = search.pane_id;
        let root_dir = search.root_dir.clone();
        let mode = search.mode;
        let query = search.buffer.clone();
        let show_hidden = self
            .panes
            .get(&pane_id)
            .map(|pane| pane.show_hidden)
            .unwrap_or(false);
        let limit = self.config.search.global_search_limit;
        let chunk_size = self.config.search.global_search_chunk_size;

        let (tx, rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let task_id = self.push_task(
            pane_id,
            "search",
            format!(
                "{}: {}",
                mode.status_label(),
                if query.is_empty() { "<all>" } else { &query }
            ),
            format!("root: {}", root_dir.display()),
            vec![root_dir.display().to_string()],
            None,
        );
        thread::spawn(move || {
            match mode {
                SearchMode::Path => stream_search_entries(
                    pane_id,
                    &root_dir,
                    show_hidden,
                    &query,
                    limit,
                    chunk_size,
                    worker_cancelled,
                    tx,
                ),
                SearchMode::Content => stream_content_search_entries(
                    pane_id,
                    &root_dir,
                    show_hidden,
                    &query,
                    limit,
                    chunk_size,
                    worker_cancelled,
                    tx,
                ),
            };
        });

        search.loading = true;
        search.searched = false;
        search.selected = 0;
        search.results.clear();
        search.task_id = Some(task_id);
        self.global_search_rx = Some(rx);
        self.global_search_cancelled = Some(cancelled);
        self.active_global_search_task_id = Some(task_id);
        Ok(())
    }

    /// 要求目前的 global search 背景工作停止，避免使用者離開畫面後仍持續掃描。
    pub(crate) fn cancel_global_search_worker(&mut self) {
        if let Some(cancelled) = self.global_search_cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.global_search_rx = None;
    }

    /// 關閉 global search 畫面，並同步停止正在進行中的背景搜尋。
    pub(crate) fn cancel_global_search(&mut self) {
        let mut cancelled_task_id = None;
        if let Some(search) = &mut self.global_search
            && let Some(task_id) = search.task_id.take()
        {
            cancelled_task_id = Some(task_id);
            self.finish_task(task_id, TaskState::Cancelled, String::from("cancelled"));
        }
        if let Some(task_id) = self.active_global_search_task_id.take()
            && cancelled_task_id != Some(task_id)
        {
            self.finish_task(task_id, TaskState::Cancelled, String::from("cancelled"));
        }
        self.cancel_global_search_worker();
        self.global_search = None;
        self.status = String::from("normal mode");
    }

    /// 回傳指定 pane 目前 preview 搜尋命中的數量。
    pub(crate) fn preview_match_count(&self, pane_id: usize) -> usize {
        self.panes
            .get(&pane_id)
            .map(PaneState::preview_match_count)
            .unwrap_or(0)
    }

    /// 清除目前 preview 的搜尋狀態；若有清除任何內容則回傳 `true`。
    pub(crate) fn clear_preview_search_if_active(&mut self) -> bool {
        if let Some(search) = self.preview_search.take() {
            if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                pane.clear_preview_search();
            }
            self.status = String::from("preview search cleared");
            return true;
        }

        if let Some(pane) = self.panes.get_mut(&self.focused_pane)
            && pane.is_preview_active()
            && pane.has_preview_search()
        {
            pane.clear_preview_search();
            self.status = String::from("preview search cleared");
            return true;
        }

        false
    }

    /// 清除目前焦點 pane 上的列表內 find-next 結果；若有清除任何內容則回傳 `true`。
    pub(crate) fn clear_list_find_if_active(&mut self) -> bool {
        if let Some(search) = self.list_find.take() {
            if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                pane.clear_list_find();
            }
            self.status = String::from("normal mode");
            return true;
        }

        if let Some(pane) = self.panes.get_mut(&self.focused_pane)
            && pane.has_list_find()
        {
            pane.clear_list_find();
            self.status = String::from("normal mode");
            return true;
        }

        false
    }

    /// 計算指定 pane 目前列表內 find-next 的命中數量。
    pub(crate) fn list_find_match_count(&self, pane_id: usize) -> usize {
        self.panes
            .get(&pane_id)
            .map(|pane| pane.list_find_match_indices().len())
            .unwrap_or(0)
    }

    /// 在目前焦點 pane 中跳到下一個或上一個列表內 find-next 命中結果。
    pub(crate) fn jump_list_find_match(
        &mut self,
        forward: bool,
        count: usize,
    ) -> io::Result<String> {
        let pane = self.current_pane_mut()?;
        let Some(query) = pane.list_find_query().map(str::to_string) else {
            return Ok(String::from("normal mode"));
        };

        let mut found = false;
        for _ in 0..count.max(1) {
            found = if forward {
                pane.jump_to_next_list_find_match()
            } else {
                pane.jump_to_previous_list_find_match()
            };
            if !found {
                break;
            }
        }
        let count = pane.list_find_match_indices().len();

        Ok(if found {
            list_find_locked_status(&query, count)
        } else {
            format!("find next: {query} (0)")
        })
    }

    /// 在 preview mode 中跳到下一個或上一個搜尋結果，並回傳狀態訊息。
    pub(crate) fn jump_preview_match(&mut self, forward: bool, count: usize) -> io::Result<String> {
        let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
            return Ok(String::from("panel no longer exists"));
        };
        if !pane.is_preview_active() {
            return Ok(String::from("preview mode is not active"));
        }

        let Some(query) = pane.preview_search_query().map(str::to_string) else {
            return Ok(String::from("preview search is empty"));
        };

        let mut found = false;
        for _ in 0..count.max(1) {
            found = if forward {
                pane.jump_to_next_preview_match()
            } else {
                pane.jump_to_previous_preview_match()
            };
            if !found {
                break;
            }
        }
        let count = pane.preview_match_count();

        Ok(if found {
            format!("preview search: {query} ({count})")
        } else {
            format!("preview search: {query} (0)")
        })
    }

    /// 將目前 global search 選到的結果打開到原 pane 中，並把游標移到該項目。
    pub(crate) fn open_global_search_result(
        &mut self,
        search: GlobalSearchState,
    ) -> io::Result<()> {
        let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
        let Some(entry) = visible.get(search.selected).cloned() else {
            self.status = String::from("global search: no result selected");
            self.global_search = Some(search);
            return Ok(());
        };

        if !self.panes.contains_key(&search.pane_id) {
            self.status = String::from("panel no longer exists");
            return Ok(());
        }

        self.reveal_path_and_track(search.pane_id, &entry.path)?;
        if let Some(task_id) = search.task_id.or(self.active_global_search_task_id.take()) {
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("stopped after opening a result"),
            );
        }
        self.cancel_global_search_worker();
        self.global_search = None;
        if let Some(pane) = self.panes.get_mut(&search.pane_id) {
            pane.set_preview_active(false);
        }
        self.status = format!("search opened: {}", entry.relative_path);
        Ok(())
    }

    /// 依照目前內容搜尋選到的檔案，計算搜尋 preview 的狀態列文字。
    pub(crate) fn search_preview_status_for(&self, search: &GlobalSearchState) -> String {
        let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
        let Some(entry) = visible.get(search.selected) else {
            return global_search_status(
                search.mode,
                &search.buffer,
                search.results.len(),
                false,
                search.searched,
                search.loading,
            );
        };
        let matches = PaneState::search_preview_match_positions(&entry.path, &search.buffer);
        if matches.is_empty() {
            return format!("preview search: {} (0)", search.buffer);
        }
        let current_match = search.preview_current_match.unwrap_or(matches[0]);
        let current = matches
            .iter()
            .position(|line| *line == current_match)
            .map(|index| index + 1)
            .unwrap_or(matches.len());
        format!(
            "preview search: {} ({}/{})",
            search.buffer,
            current,
            matches.len()
        )
    }

    /// 在 global search 的 preview focus 模式中，跳到下一個或上一個命中位置。
    pub(crate) fn move_search_preview_match(&self, search: &mut GlobalSearchState, forward: bool) {
        let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
        let Some(entry) = visible.get(search.selected) else {
            search.preview_scroll = None;
            return;
        };
        let matches = PaneState::search_preview_match_positions(&entry.path, &search.buffer);
        if matches.is_empty() {
            search.preview_current_match = None;
            search.preview_scroll = Some(0);
            return;
        }
        let current = search.preview_current_match.unwrap_or(matches[0]);
        let target = if forward {
            matches
                .iter()
                .copied()
                .find(|line| *line > current)
                .unwrap_or(matches[0])
        } else {
            matches
                .iter()
                .rev()
                .copied()
                .find(|line| *line < current)
                .unwrap_or(*matches.last().unwrap_or(&matches[0]))
        };
        search.preview_current_match = Some(target);
        search.preview_scroll = Some(target);
    }
}
