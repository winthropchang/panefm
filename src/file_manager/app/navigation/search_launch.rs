use std::io;
use std::time::Instant;

use super::super::*;

impl App {
    /// 打開 filter 輸入框，並以目前焦點 pane 作為過濾目標。
    pub(crate) fn open_filter_input(&mut self, mode: FilterMode) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let filter = FilterState {
            pane_id: self.focused_pane,
            buffer: String::new(),
            editing: true,
            mode,
        };
        self.apply_filter_buffer(&filter);
        self.status = format_filter_status(&filter);
        self.filter = Some(filter);
    }

    /// 打開 global search 面板，遞迴建立目前目錄下的搜尋候選資料集。
    pub(crate) fn open_global_search(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = GlobalSearchState {
            pane_id: self.focused_pane,
            root_dir: pane.cwd.clone(),
            mode: SearchMode::Path,
            buffer: String::new(),
            editing: true,
            loading: false,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        };
        self.status = global_search_status(search.mode, "", 0, true, false, false);
        self.global_search = Some(search);
        self.cancel_global_search_worker();
        Ok(())
    }

    /// 打開內容搜尋面板，遞迴搜尋目前目錄下所有檔案內容。
    pub(crate) fn open_content_search(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = GlobalSearchState {
            pane_id: self.focused_pane,
            root_dir: pane.cwd.clone(),
            mode: SearchMode::Content,
            buffer: String::new(),
            editing: true,
            loading: false,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        };
        self.status = global_search_status(search.mode, "", 0, true, false, false);
        self.global_search = Some(search);
        self.cancel_global_search_worker();
        Ok(())
    }

    /// 切換目前焦點 panel 自己的 preview mode。
    ///
    /// 參數：
    /// - `self: &mut App`，包含 panel 集合與目前焦點的應用程式狀態。
    ///
    /// 回傳：`()`；只切換 `focused_pane` 對應的 `PaneState::preview_active`，其他
    /// panel 已開啟的 preview 會保持原狀。若焦點 panel 已不存在，會在狀態列顯示錯誤。
    pub(crate) fn open_preview_focus(&mut self) {
        let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return;
        };
        let preview_open = pane.toggle_preview_open();
        self.pending_g = false;
        self.pending_y = false;
        self.status = if preview_open {
            String::from("preview focused (press 'h' to return to list, 'Tab' to close)")
        } else {
            String::from("normal mode")
        };
    }

    /// 切換目前焦點 panel 的 VCS Diff 預覽模式。
    ///
    /// - 若 preview 未開啟：打開 preview 並切換至 diff 模式。
    /// - 若 preview 已開啟且處於 diff 模式：切回全文預覽模式。
    /// - 若 preview 已開啟但處於全文模式：切換至 diff 模式。
    pub(crate) fn toggle_preview_diff_mode(&mut self) {
        let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return;
        };
        let is_diff = pane.toggle_preview_diff_mode();
        self.pending_g = false;
        self.pending_y = false;
        self.status = if is_diff {
            String::from("preview diff mode (press 'Ctrl+d' for full content, 'Tab' to close)")
        } else {
            String::from("preview full content (press 'Ctrl+d' for diff)")
        };
    }

    /// 打開 preview search 輸入框，並清空上一次的搜尋字串。
    pub(crate) fn open_preview_search_input(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = PreviewSearchState {
            pane_id: self.focused_pane,
            buffer: String::new(),
            editing: true,
        };
        self.apply_preview_search_buffer(&search);
        self.status =
            preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
        self.preview_search = Some(search);
    }

    /// 打開目前 pane 的列表內 find-next 輸入框，並沿用目前已存在的查詢字串。
    pub(crate) fn open_list_find_input(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = ListFindState {
            pane_id: self.focused_pane,
            buffer: String::new(),
        };
        self.apply_list_find_buffer(&search);
        self.status = list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
        self.list_find = Some(search);
    }

    /// 使用 `fzf` 遞迴掃描目前 pane 的目錄樹，快速挑選任意深度的目標。
    pub(crate) fn open_fzf_jump(&mut self) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("jump failed: panel not found");
            return;
        };

        if !pane.cwd.is_dir() {
            self.status = String::from("jump failed: current root is not a directory");
            return;
        }

        let root_dir = pane.cwd.clone();
        let task_id = self.push_task(
            self.focused_pane,
            "jump",
            format!("fzf jump in {}", root_dir.display()),
            String::from("waiting for fzf"),
            vec![root_dir.display().to_string()],
            None,
        );
        self.pending_fzf_jump = Some(FzfJumpRequest {
            pane_id: self.focused_pane,
            root_dir,
            show_hidden: true,
            follow_links: self.config.search.fzf_follow_links,
            task_id,
        });
        self.status = String::from("jump: fzf loading");
    }

    /// 套用 `fzf` 選取結果；取消時保留原本列表狀態。
    pub(crate) fn apply_fzf_jump_selection(
        &mut self,
        request: FzfJumpRequest,
        selected_line: Option<&str>,
    ) {
        let Some(line) = selected_line.map(str::trim).filter(|line| !line.is_empty()) else {
            self.finish_task(
                request.task_id,
                TaskState::Cancelled,
                String::from("fzf cancelled"),
            );
            self.status = String::from("jump cancelled");
            return;
        };

        if !self.panes.contains_key(&request.pane_id) {
            self.finish_task(
                request.task_id,
                TaskState::Failed,
                String::from("panel no longer exists"),
            );
            self.status = String::from("jump failed: panel no longer exists");
            return;
        }

        let target_path = jump_selection_to_path(&request.root_dir, line);
        let go_to_path_started_at = Instant::now();
        match self.go_to_path_and_track(request.pane_id, &target_path) {
            Ok(()) => {
                debug_timing_message(&format!("jump target path: {}", target_path.display()));
                debug_timing_log("jump go_to_path", go_to_path_started_at);
                self.finish_task(request.task_id, TaskState::Done, format!("opened {line}"));
                self.status = format!("jumped: {line}");
            }
            Err(error) => {
                debug_timing_log("jump go_to_path (failed)", go_to_path_started_at);
                self.finish_task(request.task_id, TaskState::Failed, error.to_string());
                self.status = format!("jump failed for {line}: {error}");
            }
        }
    }

    /// 切換目前焦點 pane 的隱藏檔顯示狀態。
    pub(crate) fn toggle_hidden_files(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        pane.toggle_hidden();
        self.status = if pane.show_hidden {
            String::from("showing hidden files")
        } else {
            String::from("hiding hidden files")
        };
        Ok(())
    }
}
