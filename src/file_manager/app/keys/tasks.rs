use super::*;

impl App {
    pub(crate) fn handle_task_panel_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        mut selected: usize,
        mut search: PanelSearchState,
        mut marked_ids: Vec<usize>,
        mut visual_anchor: Option<usize>,
    ) -> Result<bool> {
        let tasks = self.tasks_for_pane(pane_id);
        let filtered_tasks = filtered_task_entries(&tasks, &search.buffer);
        let len = filtered_tasks.len();
        if search.editing {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    search.editing = false;
                }
                _ if key_matches_plain_letter(&key, 'q') => {
                    search.editing = false;
                }
                _ => {}
            }
            let next_len = filtered_task_entries(&tasks, &search.buffer).len();
            let status = task_panel_status(
                &search.buffer,
                next_len,
                selected,
                search.editing,
                marked_ids.len(),
            );
            self.pending_action = Some(PendingAction::TaskPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            });
            self.status = status;
        } else {
            if self.capture_pending_count_digit(&key) {
                self.pending_action = Some(PendingAction::TaskPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                return Ok(true);
            }
            if key_matches_shifted_letter(&key, 'G') {
                if let Some(count) = self.take_pending_count() {
                    if len > 0 {
                        selected = count.saturating_sub(1).min(len.saturating_sub(1));
                    }
                } else if len > 0 {
                    selected = len - 1;
                }
                self.pending_g = false;
                let status = if let Some(anchor) = visual_anchor {
                    self.task_visual_status_label(anchor, selected, marked_ids.len())
                } else {
                    task_panel_status(&search.buffer, len, selected, false, marked_ids.len())
                };
                self.pending_action = Some(PendingAction::TaskPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                self.status = status;
                return Ok(true);
            }
            if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V') {
                self.pending_g = false;
                if let Some(anchor) = visual_anchor.take() {
                    let added = self.commit_task_visual_selection(
                        &filtered_tasks,
                        &mut marked_ids,
                        anchor,
                        selected,
                    );
                    self.status = if added == 0 {
                        format!("tasks: kept {} marked items", marked_ids.len())
                    } else {
                        format!("tasks: marked {} items", marked_ids.len())
                    };
                } else if len > 0 {
                    visual_anchor = Some(selected);
                    self.status =
                        self.task_visual_status_label(selected, selected, marked_ids.len());
                }
                self.pending_action = Some(PendingAction::TaskPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                return Ok(true);
            }
            if key.code == KeyCode::Char(' ') && key.modifiers.is_empty() {
                self.pending_g = false;
                if let Some(task) = filtered_tasks.get(selected) {
                    if let Some(pos) = marked_ids.iter().position(|id| *id == task.id) {
                        marked_ids.remove(pos);
                        self.status = format!(
                            "tasks: unmarked task #{}, {} marked",
                            task.id,
                            marked_ids.len()
                        );
                    } else {
                        marked_ids.push(task.id);
                        self.status = format!(
                            "tasks: marked task #{}, {} marked",
                            task.id,
                            marked_ids.len()
                        );
                    }
                }
                self.pending_action = Some(PendingAction::TaskPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                return Ok(true);
            }
            if key_matches_plain_letter(&key, 'a') {
                self.pending_g = false;
                if marked_ids.len() == len && len > 0 {
                    marked_ids.clear();
                    self.status = String::from("tasks: cleared all marks");
                } else {
                    marked_ids = filtered_tasks.iter().map(|t| t.id).collect();
                    self.status = format!("tasks: marked all {} tasks", marked_ids.len());
                }
                self.pending_action = Some(PendingAction::TaskPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                return Ok(true);
            }
            if key_matches_plain_letter(&key, 'd') {
                self.clear_pending_count();
                self.pending_g = false;
                if !marked_ids.is_empty() {
                    let to_delete = std::mem::take(&mut marked_ids);
                    visual_anchor = None;
                    let deleted = self.delete_tasks_by_ids(&to_delete);
                    let next_tasks = self.tasks_for_pane(pane_id);
                    let next_len = filtered_task_entries(&next_tasks, &search.buffer).len();
                    selected = selected.min(next_len.saturating_sub(1));
                    self.status = format!("tasks: deleted {deleted} tasks");
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    return Ok(true);
                } else if let Some(task) = filtered_tasks.get(selected) {
                    let target_id = task.id;
                    let target_kind = task.kind.clone();
                    let _deleted = self.delete_tasks_by_ids(&[target_id]);
                    let next_tasks = self.tasks_for_pane(pane_id);
                    let next_len = filtered_task_entries(&next_tasks, &search.buffer).len();
                    selected = selected.min(next_len.saturating_sub(1));
                    self.status = format!("tasks: deleted task #{target_id} [{target_kind}]");
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    return Ok(true);
                } else {
                    self.status = String::from("tasks: empty");
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected: 0,
                        search,
                        marked_ids,
                        visual_anchor: None,
                    });
                    return Ok(true);
                }
            }
            if key_matches_shifted_letter(&key, 'D') {
                self.clear_pending_count();
                self.pending_g = false;
                marked_ids.clear();
                visual_anchor = None;
                let deleted = self.delete_all_tasks_for_pane(pane_id);
                selected = 0;
                self.status = if deleted == 0 {
                    String::from("tasks: empty")
                } else {
                    format!("tasks: cleared {deleted} tasks")
                };
                self.pending_action = Some(PendingAction::TaskPanel {
                    pane_id,
                    selected,
                    search,
                    marked_ids,
                    visual_anchor,
                });
                return Ok(true);
            }
            match key.code {
                KeyCode::Down => {
                    if len > 0 {
                        selected = (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                _ if key_matches_plain_letter(&key, 'j') => {
                    if len > 0 {
                        selected = (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(self.take_count_or_one());
                    self.pending_g = false;
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    selected = selected.saturating_sub(self.take_count_or_one());
                    self.pending_g = false;
                }
                _ if key_matches_ctrl_letter(&key, 'd') => {
                    if len > 0 {
                        selected =
                            (selected + self.take_panel_page_step()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                _ if key_matches_ctrl_letter(&key, 'u') => {
                    selected = selected.saturating_sub(self.take_panel_page_step());
                    self.pending_g = false;
                }
                _ if key_matches_shifted_letter(&key, 'J') => {
                    if len > 0 {
                        selected =
                            (selected + self.take_large_move_step()).min(len.saturating_sub(1));
                    }
                    self.pending_g = false;
                }
                _ if key_matches_shifted_letter(&key, 'K') => {
                    selected = selected.saturating_sub(self.take_large_move_step());
                    self.pending_g = false;
                }
                _ if key_matches_plain_letter(&key, 'f') => {
                    self.clear_pending_count();
                    search.editing = true;
                    self.text_input_mode = RenameMode::Insert;
                    self.text_input_cursor = search.buffer.chars().count();
                    self.pending_g = false;
                }
                _ if key_matches_plain_letter(&key, 'x') || key_matches_plain_letter(&key, 'c') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(task) = filtered_tasks.get(selected) {
                        self.cancel_task_by_id(task.id);
                        let next_tasks = self.tasks_for_pane(pane_id);
                        let next_len = filtered_task_entries(&next_tasks, &search.buffer).len();
                        selected = selected.min(next_len.saturating_sub(1));
                        let status = task_panel_status(
                            &search.buffer,
                            next_len,
                            selected,
                            false,
                            marked_ids.len(),
                        );
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        if self.status.is_empty() {
                            self.status = status;
                        }
                        return Ok(true);
                    }
                }
                _ if key_matches_shifted_letter(&key, 'X')
                    || key_matches_shifted_letter(&key, 'C') =>
                {
                    self.clear_pending_count();
                    self.pending_g = false;
                    let cancelled = self.cancel_running_tasks_for_pane(pane_id);
                    self.status = if cancelled == 0 {
                        String::from("no cancellable running tasks")
                    } else {
                        format!("cancelled {cancelled} tasks")
                    };
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'g') => {
                    if self.pending_g {
                        if let Some(count) = self.take_pending_count() {
                            selected = count.saturating_sub(1).min(len.saturating_sub(1));
                        } else {
                            selected = 0;
                        }
                        self.pending_g = false;
                    } else {
                        self.pending_g = true;
                    }
                }
                KeyCode::Enter | KeyCode::Right => {
                    if let Some(task) = filtered_tasks.get(selected) {
                        self.status = format!("task {} [{}] {}", task.id, task.kind, task.detail);
                    } else {
                        self.status = String::from("tasks: empty");
                    }
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    if let Some(task) = filtered_tasks.get(selected) {
                        self.status = format!("task {} [{}] {}", task.id, task.kind, task.detail);
                    } else {
                        self.status = String::from("tasks: empty");
                    }
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if visual_anchor.take().is_some() {
                        self.status = String::from("task visual: cancelled");
                    } else if !marked_ids.is_empty() {
                        let cleared = marked_ids.len();
                        marked_ids.clear();
                        self.status = format!("tasks: cleared {cleared} marks");
                    } else {
                        self.status = String::from("normal mode");
                        return Ok(true);
                    }
                }
                _ if key_matches_plain_letter(&key, 't')
                    || key_matches_plain_letter(&key, 'q')
                    || key_matches_plain_letter(&key, 'h') =>
                {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
                }
                _ => {
                    self.clear_pending_count();
                    self.pending_g = false;
                }
            }
            let status = if let Some(anchor) = visual_anchor {
                self.task_visual_status_label(anchor, selected, marked_ids.len())
            } else {
                task_panel_status(&search.buffer, len, selected, false, marked_ids.len())
            };
            self.pending_action = Some(PendingAction::TaskPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            });
            if !matches!(key.code, KeyCode::Enter | KeyCode::Right)
                && !key_matches_plain_letter(&key, 'l')
            {
                self.status = status;
            }
        }

        Ok(true)
    }
}
