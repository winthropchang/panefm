use std::io;

use super::super::*;

impl App {
    /// 建立新的 task 紀錄並加入 task log，回傳這筆任務的 id。
    ///
    /// 參數：
    /// - `pane_id: usize`：啟動工作的 panel 編號。
    /// - `kind: &'static str`：供取消、搜尋與診斷使用的穩定工作種類。
    /// - `title: String`：使用者可閱讀的操作說明，例如 `copy 2 item(s)`。
    /// - `detail: String`：執行中說明，完成時可由 [`Self::finish_task`] 改成結果或錯誤。
    /// - `source_locations: Vec<String>`：工作實際讀取或修改的所有來源路徑／URI。
    /// - `destination_location: Option<String>`：工作寫入或跳轉的目的位置。
    ///
    /// 回傳：`usize`，新 task 的唯一 id。來源與目的地會獨立持久化，因此即使完成時
    /// `detail` 被結果覆寫，task 面板仍能完整說明檔案操作方向。
    pub(crate) fn push_task(
        &mut self,
        pane_id: usize,
        kind: &'static str,
        title: String,
        detail: String,
        source_locations: Vec<String>,
        destination_location: Option<String>,
    ) -> usize {
        let id = self.next_task_id;
        self.next_task_id += 1;
        self.task_log.push(TaskRecord {
            id,
            pane_id,
            kind: kind.to_string(),
            title,
            detail,
            source_locations,
            destination_location,
            state: TaskState::Running,
            progress_percent: None,
            completed_bytes: None,
            total_bytes: None,
            started_at_unix_ms: unix_time_ms_now(),
            finished_at_unix_ms: None,
        });
        if self.task_log.len() > 200 {
            let overflow = self.task_log.len() - 200;
            self.task_log.drain(0..overflow);
        }
        self.persist_task_history_best_effort();
        id
    }

    /// 更新指定 task 的最終狀態與說明文字。
    pub(crate) fn finish_task(&mut self, task_id: usize, state: TaskState, detail: String) {
        let mut changed = false;
        if let Some(task) = self.task_log.iter_mut().find(|task| task.id == task_id) {
            task.state = state;
            task.detail = detail;
            if state == TaskState::Done && task.progress_percent.is_some() {
                task.progress_percent = Some(100);
            }
            if state == TaskState::Done
                && let Some(total_bytes) = task.total_bytes
            {
                task.completed_bytes = Some(total_bytes.max(task.completed_bytes.unwrap_or(0)));
                task.total_bytes = task.completed_bytes;
            }
            task.finished_at_unix_ms = Some(unix_time_ms_now());
            changed = true;
        }
        if changed {
            self.persist_task_history_best_effort();
        }
    }

    /// 更新背景檔案工作的原始 byte 進度，並允許走訪期間依新發現的總量校正。
    ///
    /// 參數：`task_id: usize` 為 task 編號；`completed_bytes: u64` 為已完成量；
    /// `total_bytes: u64` 為預估總量。
    /// 回傳：`() `；找不到 task 或總量為零時不修改狀態。
    pub(crate) fn update_task_progress(
        &mut self,
        task_id: usize,
        completed_bytes: u64,
        total_bytes: u64,
    ) {
        let mut changed = false;
        if let Some(task) = self.task_log.iter_mut().find(|task| task.id == task_id) {
            let total_bytes = total_bytes.max(completed_bytes);
            if task.completed_bytes != Some(completed_bytes)
                || task.total_bytes != Some(total_bytes)
            {
                task.completed_bytes = Some(completed_bytes);
                task.total_bytes = Some(total_bytes);
                task.progress_percent = if total_bytes == 0 {
                    Some(0)
                } else {
                    Some(
                        completed_bytes
                            .saturating_mul(100)
                            .checked_div(total_bytes)
                            .unwrap_or(0)
                            .min(99) as u8,
                    )
                };
                changed = true;
            }
        }
        if changed {
            self.persist_task_history_best_effort();
        }
    }

    /// 立即保存目前 task 歷史；失敗時保留應用程式運作並把原因顯示在狀態列。
    ///
    /// 參數：無。
    /// 回傳：`() `。持久化錯誤不應讓進行中的檔案工作崩潰，但必須讓使用者知道
    /// 關閉後可能看不到最新歷史。
    pub(crate) fn persist_task_history_best_effort(&mut self) {
        if let Err(error) = save_task_history(&self.task_history_path, &self.task_log) {
            self.status = format!("task history save failed: {error}");
        }
    }

    /// 取消指定 task；目前支援 search worker 與尚未執行的 queued open / fzf jump。
    pub(crate) fn cancel_task_by_id(&mut self, task_id: usize) {
        if self.active_global_search_task_id == Some(task_id) {
            self.cancel_global_search();
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if self.active_network_goto_task_id == Some(task_id) {
            self.cancel_network_goto("cancelled from task panel");
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if self
            .pending_fzf_jump
            .as_ref()
            .map(|request| request.task_id)
            == Some(task_id)
        {
            self.pending_fzf_jump = None;
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("cancelled before fzf"),
            );
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if self.pending_launch.as_ref().map(|queued| queued.task_id) == Some(task_id) {
            self.pending_launch = None;
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("cancelled before launch"),
            );
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if let Some(task) = self.task_log.iter().find(|task| task.id == task_id) {
            if matches!(task.state, TaskState::Running) {
                self.status = format!("task {task_id} cannot be cancelled now");
            } else {
                self.status = format!("task {task_id} is already {}", task_state_label(task.state));
            }
        } else {
            self.status = format!("task {task_id} not found");
        }
    }

    /// 取消目前 panel 中所有仍可取消的背景任務。
    ///
    /// 參數：
    /// - `pane_id: usize`，要清理任務的 panel 編號。
    ///
    /// 回傳：`usize`，實際送出取消要求的任務數量。
    pub(crate) fn cancel_running_tasks_for_pane(&mut self, pane_id: usize) -> usize {
        let task_ids = self
            .task_log
            .iter()
            .filter(|task| task.pane_id == pane_id && matches!(task.state, TaskState::Running))
            .map(|task| task.id)
            .collect::<Vec<_>>();

        let mut cancelled = 0;
        for task_id in task_ids {
            let was_running = self
                .task_log
                .iter()
                .any(|task| task.id == task_id && matches!(task.state, TaskState::Running));
            self.cancel_task_by_id(task_id);
            if was_running
                && self
                    .task_log
                    .iter()
                    .any(|task| task.id == task_id && matches!(task.state, TaskState::Cancelled))
            {
                cancelled += 1;
            }
        }
        cancelled
    }

    /// 取得目前 pane 對應的任務清單，最新的排在最上面。
    pub(crate) fn tasks_for_pane(&self, pane_id: usize) -> Vec<TaskRecord> {
        self.task_log
            .iter()
            .filter(|task| task.pane_id == pane_id)
            .cloned()
            .rev()
            .collect()
    }

    /// 刪除指定的 task 清單（如果正在執行則先取消，並從 task_log 中移除）。
    pub(crate) fn delete_tasks_by_ids(&mut self, task_ids: &[usize]) -> usize {
        for id in task_ids {
            self.cancel_task_by_id(*id);
        }
        let initial_len = self.task_log.len();
        self.task_log.retain(|task| !task_ids.contains(&task.id));
        let removed = initial_len.saturating_sub(self.task_log.len());
        self.persist_task_history_best_effort();
        removed
    }

    /// 清空指定 pane 的所有任務（如果正在執行則先取消）。
    pub(crate) fn delete_all_tasks_for_pane(&mut self, pane_id: usize) -> usize {
        let pane_task_ids: Vec<usize> = self
            .task_log
            .iter()
            .filter(|t| t.pane_id == pane_id)
            .map(|t| t.id)
            .collect();
        self.delete_tasks_by_ids(&pane_task_ids)
    }

    fn is_transient_picker(action: &PendingAction) -> bool {
        matches!(
            action,
            PendingAction::GoPicker { .. }
                | PendingAction::WindowPicker { .. }
                | PendingAction::WindowResize { .. }
                | PendingAction::SortPicker { .. }
                | PendingAction::BookmarkPicker { .. }
                | PendingAction::LineModePicker { .. }
                | PendingAction::YankPicker { .. }
                | PendingAction::ThemeCommandPicker { .. }
                | PendingAction::ThemePicker { .. }
                | PendingAction::CopyPicker { .. }
                | PendingAction::OpenPicker { .. }
        )
    }

    /// 執行 help 面板中選到的功能，直接跳到對應模式或命令。
    pub(crate) fn execute_help_entry(
        &mut self,
        entries: &[HelpEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("help: no command selected");
            return Ok(());
        };

        self.pending_action = None;
        let mut should_restore_help_return = true;
        match entry.action {
            HelpAction::Command(command) => {
                if command.ends_with(' ') {
                    self.open_prefilled_command(command);
                    should_restore_help_return = false;
                } else {
                    self.execute_command(command)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    if matches!(
                        self.help_return,
                        Some(HelpReturnState::Pending(ref action)) if Self::is_transient_picker(action)
                    ) {
                        should_restore_help_return = false;
                        self.help_return = None;
                    }
                }
            }
            HelpAction::Delete => self.start_delete_confirmation(false),
            HelpAction::Filter => self.open_filter_input(FilterMode::Normal),
            HelpAction::FuzzyFilter => self.open_filter_input(FilterMode::Fuzzy),
            HelpAction::Sort => self.open_sort_picker(),
            HelpAction::Hidden => {
                self.toggle_hidden_files()?;
            }
            HelpAction::Visual => {
                self.open_visual_selection()?;
            }
            HelpAction::QuitHint => {
                self.status = String::from("use q in normal mode to quit");
            }
        }

        if should_restore_help_return && self.pending_action.is_none() && self.help_return.is_some()
        {
            self.restore_help_return_state(true)?;
        }
        Ok(())
    }
}
