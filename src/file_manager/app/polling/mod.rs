pub(crate) mod directory;
pub(crate) mod jobs;
pub(crate) mod search;
pub(crate) mod tasks;
pub(crate) mod updater;
pub(crate) mod watcher;

use super::*;

impl App {
    /// 輪詢非同步背景 VCS（Git / SVN）狀態查詢結果，並更新對應 pane。
    pub(crate) fn poll_vcs_status(&mut self) {
        while let Some(response) = self.vcs_manager.try_recv_response() {
            if let Some(pane) = self.panes.get_mut(&response.pane_id)
                && pane.cwd == response.directory
            {
                pane.set_vcs_info(response.info);
            }
        }
    }

    /// 非阻塞接收背景 global search 的增量結果，並更新目前搜尋 panel 與 task。
    ///
    /// 參數：無；資料由 `global_search_rx` channel 取得。
    /// 回傳：`()`, 每輪最多處理八筆訊息，避免大量結果讓主事件迴圈失去回應。
    /// 每個訊息都核對 panel id 與 query，舊搜尋取消後晚到的 chunk 會被捨棄，不能
    /// 混入使用者後來啟動的新搜尋。
    pub(crate) fn poll_background_tasks(&mut self) {
        self.poll_filesystem_watcher();
        self.poll_network_goto();
        self.poll_file_jobs();
        self.poll_directory_load_jobs();
        self.poll_directory_size_jobs();
        self.poll_diff_job();
        self.poll_update_check();
        self.poll_in_app_update();
        self.poll_vcs_status();

        let Some(receiver) = &self.global_search_rx else {
            return;
        };
        let messages: Vec<GlobalSearchEvent> = receiver.try_iter().take(8).collect();
        if messages.is_empty() {
            return;
        }

        let mut finished = false;
        let mut completed_search_task = None;
        for message in messages {
            match message {
                GlobalSearchEvent::Chunk {
                    pane_id,
                    query,
                    mut entries,
                } => {
                    let Some(search) = &mut self.global_search else {
                        continue;
                    };
                    if search.pane_id != pane_id || search.buffer != query {
                        continue;
                    }
                    // 搜尋結果採穩定串流列表：既有內容不重排，新資料只加到尾端。
                    // 這可避免使用者正在移動時，游標所在畫面行被新批次推來推去。
                    search.results.append(&mut entries);
                    search.results.truncate(200);
                    search.selected = search
                        .selected
                        .min(global_search_visible_len(search).saturating_sub(1));
                    search.searched = true;
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        search.editing,
                        search.searched,
                        true,
                    );
                }
                GlobalSearchEvent::Done { pane_id, query } => {
                    if let Some(search) = &mut self.global_search {
                        if search.pane_id != pane_id || search.buffer != query {
                            continue;
                        }
                        search.loading = false;
                        search.searched = true;
                        if let Some(task_id) = search.task_id.take() {
                            completed_search_task = Some((task_id, search.results.len()));
                        }
                        self.status = global_search_status(
                            search.mode,
                            &search.buffer,
                            search.results.len(),
                            search.editing,
                            search.searched,
                            search.loading,
                        );
                    } else if let Some(task_id) = self.active_global_search_task_id {
                        completed_search_task = Some((task_id, 0));
                    }
                    finished = true;
                }
                GlobalSearchEvent::MissingTool {
                    pane_id,
                    query,
                    tool,
                } => {
                    let search_context = self
                        .global_search
                        .as_ref()
                        .filter(|search| search.pane_id == pane_id && search.buffer == query)
                        .map(|search| (search.task_id, search.mode));
                    self.global_search = None;
                    self.cancel_global_search_worker();
                    let task_id = search_context
                        .and_then(|(task_id, _)| task_id)
                        .or(self.active_global_search_task_id.take());
                    if let Some(task_id) = task_id {
                        self.finish_task(task_id, TaskState::Failed, format!("missing {tool}"));
                    }
                    self.pending_action = Some(PendingAction::ToolPanel {
                        pane_id,
                        selected: 0,
                    });
                    let mode = search_context
                        .map(|(_, mode)| mode)
                        .unwrap_or(SearchMode::Path);
                    self.status = missing_search_tool_status(mode, &tool);
                }
            }
        }

        if let Some((task_id, result_count)) = completed_search_task {
            self.active_global_search_task_id = None;
            self.finish_task(task_id, TaskState::Done, format!("{result_count} results"));
        }

        if finished {
            self.cancel_global_search_worker();
        }
    }
}
