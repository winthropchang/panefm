#![allow(unused_imports)]

use super::*;

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
            if pane.has_list_find() {
                if let Some(target) = pane.list_find_match_indices().first().copied() {
                    pane.selected = target;
                    pane.list_state.select(Some(target));
                    pane.preview_scroll = 0;
                }
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

    /// 為指定 panel 啟動非同步遞迴目錄大小掃描。
    ///
    /// 參數：`pane_id: usize`，要顯示 `ms` 大小的 panel 編號。
    /// 回傳：`() `；panel 不存在或已在計算相同目錄時不重複重啟，避免複製或頻繁變更時數字反覆歸零跳動。
    pub(crate) fn start_directory_size_scan(&mut self, pane_id: usize) {
        let (cwd, directories) = {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                return;
            };
            if self
                .directory_size_jobs
                .get(&pane_id)
                .is_some_and(|job| job.cwd == pane.cwd)
            {
                return;
            }
            pane.init_directory_sizes_if_missing();
            let cwd = pane.cwd.clone();
            let directories = pane
                .entries
                .iter()
                .filter(|entry| entry.is_dir)
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>();
            (cwd, directories)
        };
        self.cancel_directory_size_scan(pane_id);
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            scan_directory_sizes(directories, &worker_cancelled, &sender);
            let _ = sender.send(DirectorySizeEvent::Done);
        });
        self.directory_size_jobs.insert(
            pane_id,
            DirectorySizeJob {
                cwd,
                receiver,
                cancelled,
            },
        );
    }

    /// 在 panel 成功切換目錄後，同步目前目錄容量工作的生命週期。
    ///
    /// 參數：
    /// - `pane_id: usize`，剛完成目錄切換的 panel 編號。
    /// - `previous_cwd: &Path`，切換前的工作目錄，用來避免選到一般檔案時無謂重啟。
    ///
    /// 回傳：`() `。只有工作目錄真的改變才處理；`linemode size` 或 size 排序啟用時，
    /// 會取消舊目錄工作並立即替新列表填入 `~0B`、啟動新掃描。其他顯示模式則只取消
    /// 可能殘留的舊工作，避免背景執行緒繼續走訪已離開的目錄。
    pub(crate) fn restart_directory_size_scan_after_navigation(
        &mut self,
        pane_id: usize,
        previous_cwd: &Path,
    ) {
        let is_size_detail = {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                self.cancel_directory_size_scan(pane_id);
                return;
            };
            if pane.cwd == previous_cwd {
                return;
            }
            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                pane.clear_directory_sizes();
                true
            } else {
                false
            }
        };
        self.cancel_directory_size_scan(pane_id);
        if is_size_detail {
            self.start_directory_size_scan(pane_id);
        }
    }

    /// 取消指定 panel 尚未完成的大小掃描並丟棄其接收端。
    ///
    /// 參數：`pane_id: usize`，要停止掃描的 panel 編號。
    /// 回傳：`() `；沒有工作時安全地不做任何事。
    pub(crate) fn cancel_directory_size_scan(&mut self, pane_id: usize) {
        if let Some(job) = self.directory_size_jobs.remove(&pane_id) {
            job.cancelled.store(true, Ordering::Relaxed);
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

    /// 啟動單一 panel 的非阻塞目錄讀取，並先套用已有快取。
    ///
    /// 參數：`pane_id` 是導航來源 panel；`cwd` 是新目錄；`selected_path` 是回到父目錄
    /// 時應重新選取的子目錄。回傳：`() `；I/O 結果由 `poll_directory_load_jobs` 套用。
    pub(crate) fn start_directory_load(
        &mut self,
        pane_id: usize,
        cwd: PathBuf,
        selected_path: Option<PathBuf>,
    ) {
        self.cancel_directory_size_scan(pane_id);
        self.cancel_directory_load(pane_id);
        let (sort_mode, random_seed) = self
            .panes
            .get(&pane_id)
            .map(|pane| (pane.sort_mode, pane.random_seed))
            .unwrap_or((SortMode::Natural { reverse: false }, 0));
        if let Some(cached) = self.directory_entry_cache.get(&cwd).cloned()
            && let Some(pane) = self.panes.get_mut(&pane_id)
        {
            pane.replace_entries_presorted(cached, selected_path.as_deref());
        }

        let (sender, receiver) = mpsc::channel();
        let worker_cwd = cwd.clone();
        let worker_selection = selected_path.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            let thread_pane_id = pane_id;
            let thread_cwd = worker_cwd.clone();
            let thread_selection = worker_selection.clone();
            let thread_sender = sender.clone();
            let stream_result = crate::file_manager::pane::stream_dir_entries_with_cancellation(
                &worker_cwd,
                sort_mode,
                random_seed,
                &worker_cancelled,
                move |progress| {
                    thread_sender
                        .send(DirectoryLoadEvent {
                            pane_id: thread_pane_id,
                            cwd: thread_cwd.clone(),
                            selected_path: thread_selection.clone(),
                            result: Ok(progress),
                        })
                        .is_ok()
                },
            );
            if let Err(error) = stream_result {
                let _ = sender.send(DirectoryLoadEvent {
                    pane_id,
                    cwd: worker_cwd,
                    selected_path: worker_selection,
                    result: Err(error),
                });
            }
        });
        self.directory_load_jobs.insert(
            pane_id,
            DirectoryLoadJob {
                cwd: cwd.clone(),
                receiver,
                cancelled,
            },
        );
        self.status = format!("loading directory: {}", cwd.display());
    }

    /// 取消指定 panel 尚未完成的目錄載入工作。
    ///
    /// 參數：`pane_id: usize`，要停止載入的 panel 編號。
    /// 回傳：`() `；沒有工作時安全地不做任何事。
    pub(crate) fn cancel_directory_load(&mut self, pane_id: usize) {
        if let Some(job) = self.directory_load_jobs.remove(&pane_id) {
            job.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// 非阻塞接收大型目錄清單；分批套用快速發現項目，並在完成時更新快取與 size linemode。
    ///
    /// 參數：無。回傳：`() `；第一批（< 1ms）讓 UI 立即畫出列表與響應游標，後續增量批次平滑呈現。
    pub(crate) fn poll_directory_load_jobs(&mut self) {
        let pane_ids = self.directory_load_jobs.keys().copied().collect::<Vec<_>>();
        for pane_id in pane_ids {
            let mut job_done = false;
            let Some((job_cwd, events)) = self
                .directory_load_jobs
                .get(&pane_id)
                .map(|job| (job.cwd.clone(), job.receiver.try_iter().collect::<Vec<_>>()))
            else {
                continue;
            };
            for event in events {
                if event.cwd != job_cwd {
                    continue;
                }
                match event.result {
                    Ok(DirectoryLoadProgress::Batch {
                        entries,
                        is_first_chunk,
                    }) => {
                        if let Some(pane) = self.panes.get_mut(&event.pane_id)
                            && pane.cwd == event.cwd
                        {
                            if is_first_chunk {
                                pane.replace_entries_presorted(
                                    entries,
                                    event.selected_path.as_deref(),
                                );
                            } else {
                                pane.extend_entries(entries);
                            }
                            // `ms` 可能在目錄清單尚未載入完成時就已啟用。每批新加入的
                            // 目錄都要立刻取得部分容量狀態，否則右側欄位會一直空白，直到
                            // 完整清單載入並重新啟動容量掃描後才第一次出現內容。
                            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                                pane.init_directory_sizes_if_missing();
                            }
                            self.status = format!(
                                "loading directory: {} ({} items)",
                                event.cwd.display(),
                                pane.entries.len()
                            );
                        }
                    }
                    Ok(DirectoryLoadProgress::Complete(entries)) => {
                        job_done = true;
                        self.directory_entry_cache
                            .insert(event.cwd.clone(), entries.clone());
                        let mut restart_size_scan = false;
                        if let Some(pane) = self.panes.get_mut(&event.pane_id)
                            && pane.cwd == event.cwd
                        {
                            pane.replace_entries_presorted(entries, event.selected_path.as_deref());
                            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                                pane.init_directory_sizes_if_missing();
                                restart_size_scan = true;
                            }
                            self.status = format!("opened directory: {}", event.cwd.display());
                        }
                        if restart_size_scan {
                            // 載入期間可能已有只包含首批目錄的同 cwd 掃描。先取消舊工作
                            // 才能確保完整清單中的每個直接子目錄都會被納入新一輪計算。
                            self.cancel_directory_size_scan(event.pane_id);
                            self.start_directory_size_scan(event.pane_id);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                        job_done = true;
                    }
                    Err(error) => {
                        job_done = true;
                        if self
                            .panes
                            .get(&event.pane_id)
                            .is_some_and(|pane| pane.cwd == event.cwd)
                        {
                            self.status = format!("open directory failed: {error}");
                        }
                    }
                }
            }
            if job_done {
                self.directory_load_jobs.remove(&pane_id);
            }
        }
    }

    /// 非阻塞套用各 panel 的目錄大小快照。
    ///
    /// 參數：無，資料來自 `directory_size_jobs`。
    /// 回傳：`() `；每個 job 每幀最多處理 64 筆，避免大量小目錄拖慢鍵盤事件。
    pub(crate) fn poll_directory_size_jobs(&mut self) {
        let pane_ids = self.directory_size_jobs.keys().copied().collect::<Vec<_>>();
        for pane_id in pane_ids {
            let mut finished = false;
            let mut updates = Vec::new();
            let Some(job) = self.directory_size_jobs.get(&pane_id) else {
                continue;
            };
            let job_cwd = job.cwd.clone();
            for event in job.receiver.try_iter().take(64) {
                match event {
                    DirectorySizeEvent::Update {
                        path,
                        bytes,
                        complete,
                    } => updates.push((path, bytes, complete)),
                    DirectorySizeEvent::Done => finished = true,
                }
            }
            let cwd_matches = self
                .panes
                .get(&pane_id)
                .is_some_and(|pane| pane.cwd == job_cwd);
            if cwd_matches && let Some(pane) = self.panes.get_mut(&pane_id) {
                for (path, bytes, complete) in updates {
                    pane.update_directory_size(&path, bytes, complete);
                }
            }
            if finished || !cwd_matches {
                self.cancel_directory_size_scan(pane_id);
            }
        }
    }

    /// 接收檔案系統 watcher 事件，去重後刷新所有顯示受影響目錄的 panel。
    ///
    /// 參數：無；監看目錄來自目前 `panes`，事件來自 [`FilesystemWatcher`] channel。
    /// 回傳：`()`；watcher 或單一目錄刷新失敗只寫入狀態列，不會結束主事件迴圈。
    pub(crate) fn poll_filesystem_watcher(&mut self) {
        let directories = self
            .panes
            .values()
            .map(|pane| pane.cwd.clone())
            .collect::<BTreeSet<_>>();
        let (changed, watch_errors) = match self.filesystem_watcher.as_mut() {
            Some(watcher) => {
                let errors = watcher.sync_directories(directories);
                (watcher.changed_directories(), errors)
            }
            None => return,
        };

        if !watch_errors.is_empty() {
            self.status = format!("filesystem watcher failed: {}", watch_errors.join(" | "));
        }
        if !changed.is_empty() {
            self.pending_watched_directories.extend(changed);
            let debounce = if self.file_job_receivers.is_empty() {
                self.config.watcher.debounce
            } else {
                Duration::from_millis(400)
            };
            // 第一個事件設定 deadline，後續同批事件只合併路徑而不無限延後刷新。
            self.filesystem_refresh_deadline
                .get_or_insert_with(|| Instant::now() + debounce);
        }

        if !self
            .filesystem_refresh_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return;
        }

        self.filesystem_refresh_deadline = None;
        let changed = std::mem::take(&mut self.pending_watched_directories);
        if let Err(error) = self.reload_watched_directories(&changed) {
            self.status = format!("automatic directory refresh failed: {error}");
        }
    }

    /// 重新載入目前工作目錄出現在 watcher 變更集合中的所有 panel。
    ///
    /// 參數：`directories: &BTreeSet<PathBuf>`，已經過 debounce 的異動目錄集合。
    /// 回傳：`io::Result<()>`；任一受影響 panel 無法重新讀取時回傳原始 I/O 錯誤。
    pub(crate) fn reload_watched_directories(
        &mut self,
        directories: &BTreeSet<PathBuf>,
    ) -> io::Result<()> {
        let affected_panes = self
            .panes
            .iter()
            .filter(|(_, pane)| directories.contains(&pane.cwd))
            .map(|(pane_id, _)| *pane_id)
            .collect::<Vec<_>>();
        for pane_id in &affected_panes {
            let pane = self.panes.get_mut(pane_id).expect("pane id came from map");
            pane.reload()?;
        }
        for pane_id in &affected_panes {
            if let Some(pane) = self.panes.get(pane_id) {
                self.directory_entry_cache
                    .insert(pane.cwd.clone(), pane.entries.clone());
            }
        }
        for pane_id in &affected_panes {
            if self.file_job_receivers.is_empty()
                && self
                    .panes
                    .get(pane_id)
                    .is_some_and(|pane| matches!(pane.active_detail_kind(), SortDetailKind::Size))
            {
                self.start_directory_size_scan(*pane_id);
            }
        }
        for dir in directories {
            if !self.panes.values().any(|pane| &pane.cwd == dir) {
                self.directory_entry_cache.remove(dir);
            }
        }
        if !directories.is_empty() {
            self.full_redraw_requested = true;
        }
        Ok(())
    }

    /// 非阻塞接收 UNC `goto` 的背景載入結果，完成後才替換指定 panel。
    ///
    /// 參數：無；資料來自 `network_goto_rx`。
    /// 回傳：`()`；尚未完成時立即返回，取消後晚到的結果也不會被套用。
    pub(crate) fn poll_network_goto(&mut self) {
        let Some(receiver) = self.network_goto_rx.take() else {
            return;
        };

        let event = match receiver.try_recv() {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => {
                self.network_goto_rx = Some(receiver);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(task_id) = self.active_network_goto_task_id.take() {
                    self.finish_task(
                        task_id,
                        TaskState::Failed,
                        String::from("network goto worker disconnected"),
                    );
                }
                self.status = String::from("network goto failed: worker disconnected");
                return;
            }
        };

        if self.active_network_goto_task_id != Some(event.task_id) {
            return;
        }
        self.active_network_goto_task_id = None;

        match event.result {
            Ok(pane) if self.panes.contains_key(&event.pane_id) => {
                let cwd = pane.cwd.clone();
                self.panes.insert(event.pane_id, pane);
                self.zoxide_tracker.track(&cwd);
                self.finish_task(
                    event.task_id,
                    TaskState::Done,
                    format!("opened {}", event.target.display()),
                );
                self.full_redraw_requested = true;
                self.status = format!("jumped to path: {}", event.target.display());
            }
            Ok(_) => {
                self.finish_task(
                    event.task_id,
                    TaskState::Cancelled,
                    String::from("panel no longer exists"),
                );
                self.status = String::from("network goto cancelled: panel no longer exists");
            }
            Err(error) => {
                self.finish_task(event.task_id, TaskState::Failed, error.to_string());
                self.status = format!("path jump failed: {} ({error})", event.target.display());
            }
        }
    }

    /// 非阻塞接收大型 paste/compress/extract 工作，並在主執行緒更新 UI 與 Undo。
    ///
    /// 參數：無；接收端保存於 `file_job_receivers`。
    /// 回傳：`()`；每一輪只檢查現有 task，不等待尚未完成的 worker。
    pub(crate) fn poll_file_jobs(&mut self) {
        let task_ids = self.file_job_receivers.keys().copied().collect::<Vec<_>>();
        for task_id in task_ids {
            let Some(receiver) = self.file_job_receivers.remove(&task_id) else {
                continue;
            };
            let mut completed = false;
            let mut refresh_target = None;
            loop {
                match receiver.try_recv() {
                    Ok(FileJobEvent::DestinationVisible { target_dir }) => {
                        refresh_target = Some(target_dir);
                    }
                    Ok(FileJobEvent::Progress {
                        task_id,
                        completed_bytes,
                        total_bytes,
                    }) => {
                        self.update_task_progress(task_id, completed_bytes, total_bytes);
                    }
                    Ok(event) => {
                        self.apply_file_job_event(event);
                        completed = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        completed = true;
                        if self
                            .task_log
                            .iter()
                            .any(|task| task.id == task_id && task.state == TaskState::Running)
                        {
                            self.finish_task(
                                task_id,
                                TaskState::Failed,
                                String::from("file worker disconnected"),
                            );
                            self.status =
                                format!("file task {task_id} failed: worker disconnected");
                        }
                        break;
                    }
                }
            }
            if let Some(target_dir) = refresh_target {
                if let Err(error) = self.reload_panes_in_tree(&target_dir) {
                    self.status =
                        format!("background paste started; destination refresh failed: {error}");
                }
                self.full_redraw_requested = true;
            }
            if !completed {
                self.file_job_receivers.insert(task_id, receiver);
            } else {
                self.active_file_job_busy_paths.remove(&task_id);
                if self
                    .task_log
                    .iter()
                    .any(|task| task.id == task_id && task.state == TaskState::Running)
                {
                    self.finish_task(
                        task_id,
                        TaskState::Failed,
                        String::from("file worker disconnected"),
                    );
                    self.status = format!("file task {task_id} failed: worker disconnected");
                }
            }
        }
    }

    /// 套用單一背景檔案工作的完成結果。
    ///
    /// 參數：`event: FileJobEvent`，worker 回傳的 paste、compress、extract 或 delete 結果。
    /// 回傳：`()`；I/O 錯誤會完整寫入 task 與狀態列，不會中止主事件迴圈。
    pub(crate) fn apply_file_job_event(&mut self, event: FileJobEvent) {
        match event {
            FileJobEvent::DestinationVisible { .. } => {
                // 目標可見事件已在 `poll_file_jobs` 即時處理，不屬於完成事件。
            }
            FileJobEvent::Progress { .. } => {
                // Progress 已在 `poll_file_jobs` 合併處理，完成事件才會進入這個函數。
            }
            FileJobEvent::Paste {
                task_id,
                clipboard,
                overwrite,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                let operation = clipboard.operation;
                self.record_file_operation(operation, result.history_items);
                if let Some(failure) = result.failure {
                    let status = paste_failure_status(
                        &failure.display_name,
                        &failure.planned_target,
                        &failure.error,
                    );
                    self.finish_task(task_id, TaskState::Failed, status.clone());
                    self.status = status;
                } else {
                    if operation == ClipboardOperation::Cut
                        && self.clipboard.as_ref() == Some(&clipboard)
                    {
                        self.clipboard = None;
                    }
                    let status = paste_success_status(operation, overwrite, result.pasted_count);
                    self.finish_task(task_id, TaskState::Done, status.clone());
                    self.status = status;
                }
                if let Err(error) = self.reload_all_panes() {
                    self.status = format!("{}; refresh failed: {error}", self.status);
                }
                self.full_redraw_requested = true;
            }
            FileJobEvent::Compress {
                task_id,
                pane_id,
                entry_count,
                first_name,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok(archive_path) => {
                        if let Err(error) = self.reload_all_panes() {
                            self.status = format!("compress completed; refresh failed: {error}");
                            self.finish_task(task_id, TaskState::Failed, self.status.clone());
                            return;
                        }
                        if let Some(pane) = self.panes.get_mut(&pane_id) {
                            pane.select_path(&archive_path);
                        }
                        let archive_name = archive_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("archive.zip");
                        self.status = if entry_count == 1 {
                            format!("compressed {first_name} -> {archive_name}")
                        } else {
                            format!("compressed {entry_count} items -> {archive_name}")
                        };
                        self.finish_task(task_id, TaskState::Done, self.status.clone());
                        self.full_redraw_requested = true;
                    }
                    Err(error) => {
                        self.status = format!("compress failed: {error}");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                }
            }
            FileJobEvent::Extract {
                task_id,
                pane_id,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok((extracted, skipped)) if !extracted.is_empty() => {
                        if let Err(error) = self.reload_all_panes() {
                            self.status = format!("extract completed; refresh failed: {error}");
                            self.finish_task(task_id, TaskState::Failed, self.status.clone());
                            return;
                        }
                        if let Some(first) = extracted.first()
                            && let Some(pane) = self.panes.get_mut(&pane_id)
                        {
                            pane.select_path(&first.output_path);
                        }
                        self.status = extraction_status_label(&extracted, skipped);
                        self.finish_task(task_id, TaskState::Done, self.status.clone());
                        self.full_redraw_requested = true;
                    }
                    Ok((_, skipped)) => {
                        self.status = format!("no supported archives selected (skipped {skipped})");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                    Err(error) => {
                        self.status = format!("extract failed: {error}");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                }
            }
            FileJobEvent::Delete {
                task_id,
                target_name,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok(deleted_names) => {
                        let status = if deleted_names.len() == 1 {
                            format!("deleted permanently {}", deleted_names[0])
                        } else {
                            format!("deleted permanently {} items", deleted_names.len())
                        };
                        self.finish_task(task_id, TaskState::Done, status.clone());
                        self.status = status;
                    }
                    Err(error) => {
                        let status = format!("failed to delete {target_name}: {error}");
                        self.finish_task(task_id, TaskState::Failed, status.clone());
                        self.status = status;
                    }
                }
                if let Err(error) = self.reload_all_panes() {
                    self.status = format!("{}; refresh failed: {error}", self.status);
                }
                self.full_redraw_requested = true;
            }
        }
    }
}
