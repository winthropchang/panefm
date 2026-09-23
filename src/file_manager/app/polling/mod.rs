pub(crate) mod directory;
pub(crate) mod jobs;
pub(crate) mod search;
pub(crate) mod tasks;
pub(crate) mod updater;
pub(crate) mod watcher;

use std::time::{Duration, Instant};

use super::*;

impl App {
    /// 輪詢非同步背景 VCS（Git / SVN）狀態查詢結果，並更新對應 pane。
    ///
    /// 回傳：`bool`；若有任何 pane 的版控資訊真正更新則回傳 `true`。
    pub(crate) fn poll_vcs_status(&mut self) -> bool {
        let mut changed = false;
        while let Some(response) = self.vcs_manager.try_recv_response() {
            if let Some(pane) = self.panes.get_mut(&response.pane_id)
                && pane.cwd == response.directory
            {
                let is_same = match (&pane.vcs_info, &response.info) {
                    (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b) || a == b,
                    (None, None) => true,
                    _ => false,
                };
                if !is_same {
                    pane.set_vcs_info(response.info);
                    changed = true;
                }
            }
        }
        changed
    }

    /// 非阻塞接收背景 global search 的增量結果，並更新目前搜尋 panel 與 task。
    ///
    /// 參數：無；資料由 `global_search_rx` channel 取得。
    /// 回傳：`bool`，若任何背景任務產生畫面變更或狀態列更新則回傳 `true`。
    pub(crate) fn poll_background_tasks(&mut self) -> bool {
        let mut has_changes = false;
        if self.poll_filesystem_watcher() {
            has_changes = true;
        }
        if self.poll_network_goto() {
            has_changes = true;
        }
        if self.poll_file_jobs() {
            has_changes = true;
        }
        if self.poll_directory_load_jobs() {
            has_changes = true;
        }
        if self.poll_directory_size_jobs() {
            has_changes = true;
        }
        if self.poll_diff_job() {
            has_changes = true;
        }
        if self.poll_update_check() {
            has_changes = true;
        }
        if self.poll_in_app_update() {
            has_changes = true;
        }
        if self.poll_vcs_status() {
            has_changes = true;
        }

        let Some(receiver) = &self.global_search_rx else {
            return has_changes;
        };
        let messages: Vec<GlobalSearchEvent> = receiver.try_iter().take(8).collect();
        if messages.is_empty() {
            return has_changes;
        }
        has_changes = true;

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

        has_changes
    }

    /// 檢查目前是否有正在執行或尚未完成的背景非同步任務。
    pub(crate) fn has_active_background_tasks(&self) -> bool {
        self.global_search_rx.is_some()
            || self.diff_job_rx.is_some()
            || self.network_goto_rx.is_some()
            || self.update_check_rx.is_some()
            || self.in_app_update_rx.is_some()
            || !self.file_job_receivers.is_empty()
            || !self.directory_load_jobs.is_empty()
            || !self.directory_size_jobs.is_empty()
            || self.filesystem_refresh_deadline.is_some()
            || !self.pending_watched_directories.is_empty()
    }

    /// 依據背景任務忙碌狀態與 debounce 截止時間，計算下一次 `event::poll` 的等待時間。
    pub(crate) fn next_event_timeout(&self, default_poll_rate: Duration) -> Duration {
        if self.has_active_background_tasks() {
            if let Some(deadline) = self.filesystem_refresh_deadline {
                let now = Instant::now();
                if deadline <= now {
                    Duration::from_millis(1)
                } else {
                    (deadline - now).min(default_poll_rate)
                }
            } else {
                default_poll_rate
            }
        } else {
            // 待機閒置時放寬等待時間，讓作業系統使執行緒休眠，消除待機 CPU 負擔
            Duration::from_millis(300)
        }
    }
}
