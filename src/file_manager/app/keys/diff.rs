use super::*;

impl App {
    pub(crate) fn handle_diff_matrix_action_key(
        &mut self,
        key: KeyEvent,
        mut state: DiffMatrixState,
    ) -> Result<bool> {
        if state.search_active {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    state.search_active = false;
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                KeyCode::Backspace => {
                    state.search_query.pop();
                    state.refresh_filtered_indices();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                KeyCode::Char(c) => {
                    state.search_query.push(c);
                    state.refresh_filtered_indices();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ => {
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
            }
        } else if state.loading {
            match key.code {
                KeyCode::Esc => {
                    if let Some(cancelled) = self.diff_job_cancelled.take() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    self.diff_job_rx = None;
                    self.status = String::from("diff matrix closed");
                }
                _ if key_matches_plain_letter(&key, 'q') => {
                    if let Some(cancelled) = self.diff_job_cancelled.take() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    self.diff_job_rx = None;
                    self.status = String::from("diff matrix closed");
                }
                _ => {
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
            }
        } else {
            match key.code {
                KeyCode::Esc => {
                    if let Some(cancelled) = self.diff_job_cancelled.take() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    self.diff_job_rx = None;
                    self.status = String::from("diff matrix closed");
                }
                _ if key_matches_plain_letter(&key, 'q') => {
                    if let Some(cancelled) = self.diff_job_cancelled.take() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    self.diff_job_rx = None;
                    self.status = String::from("diff matrix closed");
                }
                _ if key_matches_plain_letter(&key, 'j') || key.code == KeyCode::Down => {
                    state.move_down();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key_matches_plain_letter(&key, 'k') || key.code == KeyCode::Up => {
                    state.move_up();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key_matches_plain_letter(&key, 'g') || key.code == KeyCode::Home => {
                    state.move_to_top();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key_matches_letter_any_case(&key, 'G') || key.code == KeyCode::End => {
                    state.move_to_bottom();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key_matches_plain_letter(&key, 'f') => {
                    state.cycle_filter_mode();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key.code == KeyCode::Char('/') => {
                    state.search_active = true;
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key_matches_plain_letter(&key, 'i') => {
                    state.git_ignore = !state.git_ignore;
                    if let Some(cancelled) = self.diff_job_cancelled.take() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    let cancelled = Arc::new(AtomicBool::new(false));
                    self.diff_job_cancelled = Some(cancelled.clone());
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.diff_job_rx = Some(rx);
                    spawn_background_diff(
                        state.panel_roots.clone(),
                        state.git_ignore,
                        state.include_hidden,
                        cancelled,
                        tx,
                    );
                    state.loading = true;
                    state.rows.clear();
                    state.filtered_indices.clear();
                    self.status = format!(
                        "diff matrix: .gitignore rules {}",
                        if state.git_ignore {
                            "enabled"
                        } else {
                            "disabled (scanning target/build dirs)"
                        }
                    );
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key.code == KeyCode::Char('.') => {
                    state.include_hidden = !state.include_hidden;
                    if let Some(cancelled) = self.diff_job_cancelled.take() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    let cancelled = Arc::new(AtomicBool::new(false));
                    self.diff_job_cancelled = Some(cancelled.clone());
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.diff_job_rx = Some(rx);
                    spawn_background_diff(
                        state.panel_roots.clone(),
                        state.git_ignore,
                        state.include_hidden,
                        cancelled,
                        tx,
                    );
                    state.loading = true;
                    state.rows.clear();
                    state.filtered_indices.clear();
                    self.status = format!(
                        "diff matrix: hidden files {}",
                        if state.include_hidden {
                            "included"
                        } else {
                            "excluded"
                        }
                    );
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ if key_matches_plain_letter(&key, 'r') => {
                    if let Some(cancelled) = self.diff_job_cancelled.take() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    let cancelled = Arc::new(AtomicBool::new(false));
                    self.diff_job_cancelled = Some(cancelled.clone());
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.diff_job_rx = Some(rx);
                    spawn_background_diff(
                        state.panel_roots.clone(),
                        state.git_ignore,
                        state.include_hidden,
                        cancelled,
                        tx,
                    );
                    state.loading = true;
                    state.rows.clear();
                    state.filtered_indices.clear();
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                KeyCode::Enter => {
                    if let Some(row) = state.selected_row() {
                        if let Some(launch) = launch_content_diff_spec(&state.panel_roots, row) {
                            let detail = format!("{} {}", launch.program, launch.args.join(" "));
                            let sources = state
                                .panel_roots
                                .iter()
                                .map(|p| p.join(&row.relative_path).display().to_string())
                                .collect();
                            let task_id = self.push_task(
                                self.focused_pane,
                                "diff",
                                format!("diff: {}", row.relative_path.display()),
                                detail,
                                sources,
                                None,
                            );
                            self.pending_launch = Some(QueuedLaunch { task_id, launch });
                            self.status =
                                format!("opening diff for {}", row.relative_path.display());
                        } else if row.is_dir {
                            self.status = format!("{} is a directory", row.relative_path.display());
                        }
                    }
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
                _ => {
                    self.pending_action = Some(PendingAction::DiffMatrix(state));
                }
            }
        }
        if let Some(action) = self.pending_action.as_ref() {
            self.status = self.status_for_pending_action(action)?;
        }

        Ok(true)
    }
}
