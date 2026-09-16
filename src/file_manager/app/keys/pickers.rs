use super::*;

impl App {
    pub(crate) fn handle_go_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 'g') => {
                if let Some(count) = self.take_pending_count() {
                    self.current_pane_mut()?
                        .move_to_visible_index(count.saturating_sub(1));
                    self.status = format!("jumped to item {count}");
                } else {
                    self.current_pane_mut()?.move_top();
                    self.status = String::from("jumped to top");
                }
                self.pending_g = false;
                self.pending_y = false;
            }
            _ if key_matches_plain_letter(&key, 't') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_prefilled_command("goto ");
            }
            _ if key_matches_plain_letter(&key, 'd') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.go_to_special_directory(GoSpecialDirectory::Documents)?;
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.go_to_special_directory(GoSpecialDirectory::Desktop)?;
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.go_to_special_directory(GoSpecialDirectory::Downloads)?;
            }
            KeyCode::Esc => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.status = String::from("normal mode");
            }
            _ => {
                self.pending_action = Some(PendingAction::GoPicker { pane_id });
                self.status = String::from("go: choose g/t/d/k/l from the panel");
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_theme_command_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 't') => {
                self.focused_pane = pane_id;
                self.open_trash_panel()?;
            }
            _ if key_matches_plain_letter(&key, 'u') => {
                self.focused_pane = pane_id;
                self.restore_latest_from_trash()?;
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.open_theme_picker();
            }
            _ if key_matches_plain_letter(&key, 'n') => {
                self.cycle_theme();
            }
            KeyCode::Esc => {
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.status = String::from("normal mode");
            }
            _ => {
                self.pending_action = Some(PendingAction::ThemeCommandPicker { pane_id });
                self.status = String::from("theme/trash: choose l/n/t/u from the panel");
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_sort_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            KeyCode::Char(',') => {
                self.status = String::from("sort cancelled");
            }
            _ if key_matches_shifted_letter(&key, 'M') => {
                self.apply_sort_mode(pane_id, SortMode::Modified { reverse: true })?
            }
            _ if key_matches_plain_letter(&key, 'm') => {
                self.apply_sort_mode(pane_id, SortMode::Modified { reverse: false })?
            }
            _ if key_matches_shifted_letter(&key, 'B') => {
                self.apply_sort_mode(pane_id, SortMode::Created { reverse: true })?
            }
            _ if key_matches_plain_letter(&key, 'b') => {
                self.apply_sort_mode(pane_id, SortMode::Created { reverse: false })?
            }
            _ if key_matches_shifted_letter(&key, 'A') => {
                self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: true })?
            }
            _ if key_matches_plain_letter(&key, 'a') => {
                self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: false })?
            }
            _ if key_matches_shifted_letter(&key, 'N') => {
                self.apply_sort_mode(pane_id, SortMode::Natural { reverse: true })?
            }
            _ if key_matches_plain_letter(&key, 'n') => {
                self.apply_sort_mode(pane_id, SortMode::Natural { reverse: false })?
            }
            _ if key_matches_shifted_letter(&key, 'E') => {
                self.apply_sort_mode(pane_id, SortMode::Extension { reverse: true })?
            }
            _ if key_matches_plain_letter(&key, 'e') => {
                self.apply_sort_mode(pane_id, SortMode::Extension { reverse: false })?
            }
            _ if key_matches_shifted_letter(&key, 'S') => {
                self.apply_sort_mode(pane_id, SortMode::Size { reverse: true })?
            }
            _ if key_matches_plain_letter(&key, 's') => {
                self.apply_sort_mode(pane_id, SortMode::Size { reverse: false })?
            }
            _ if key_matches_plain_letter(&key, 'r') => {
                self.apply_sort_mode(pane_id, SortMode::Random)?
            }
            KeyCode::Esc => {
                self.status = String::from("sort cancelled");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.status = String::from("sort cancelled");
            }
            _ => {
                self.pending_action = Some(PendingAction::SortPicker { pane_id });
                self.status = String::from("sort: choose a key from the panel");
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_window_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 'w') => {
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.split_current_at(SplitDirection::Vertical, SplitPlacement::Before)?;
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.split_current_at(SplitDirection::Horizontal, SplitPlacement::After)?;
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.split_current_at(SplitDirection::Horizontal, SplitPlacement::Before)?;
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.split_current_at(SplitDirection::Vertical, SplitPlacement::After)?;
            }
            _ if key_matches_plain_letter(&key, 'r') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.pending_action = Some(PendingAction::WindowResize { pane_id });
                self.status = String::from(
                    "[RESIZE] h/l: width (±4) | j/k: height (±2) | =: equal | Esc/Enter: done",
                );
            }
            KeyCode::Char('=') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.equalize_layout();
            }
            _ if key_matches_plain_letter(&key, 'c') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                if self.focused_pane == pane_id {
                    self.close_current_pane();
                } else {
                    self.status = String::from("panel focus changed");
                }
            }
            _ if key_matches_plain_letter(&key, 'o') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                if self.focused_pane == pane_id {
                    self.only_current_pane();
                } else {
                    self.status = String::from("panel focus changed");
                }
            }
            _ if key_matches_plain_letter(&key, 't') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                if self.focused_pane == pane_id {
                    self.open_terminal_in_active_panel()?;
                } else {
                    self.status = String::from("panel focus changed");
                }
            }
            _ if key_matches_plain_letter(&key, 's') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.split_current_at(SplitDirection::Horizontal, SplitPlacement::After)?;
            }
            _ if key_matches_plain_letter(&key, 'v') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.split_current_at(SplitDirection::Vertical, SplitPlacement::After)?;
            }
            _ if key_matches_shifted_letter(&key, 'W') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_prefilled_command("width ");
            }
            _ if key_matches_shifted_letter(&key, 'H') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_prefilled_command("height ");
            }
            _ if key_matches_plain_letter(&key, 'd') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_diff_matrix(None)?;
            }
            _ if key_matches_shifted_letter(&key, 'D') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_prefilled_command("diff ");
            }
            KeyCode::Char(ch @ '1'..='9') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.focus_pane_by_id_argument(&ch.to_string());
            }
            KeyCode::Esc => {
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') => {
                self.status = String::from("normal mode");
            }
            _ => {
                self.pending_action = Some(PendingAction::WindowPicker { pane_id });
                self.status =
                    String::from("panel: choose h/j/k/l/s/v/r/=/W/H/c/o/t/d/1..9 from the panel");
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_window_resize_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 'h')
                || key.code == KeyCode::Left
                || key.code == KeyCode::Char('<')
                || key.code == KeyCode::Char(',') =>
            {
                self.resize_focused_pane_width(-4);
                self.pending_action = Some(PendingAction::WindowResize { pane_id });
            }
            _ if key_matches_plain_letter(&key, 'l')
                || key.code == KeyCode::Right
                || key.code == KeyCode::Char('>')
                || key.code == KeyCode::Char('.') =>
            {
                self.resize_focused_pane_width(4);
                self.pending_action = Some(PendingAction::WindowResize { pane_id });
            }
            _ if key_matches_plain_letter(&key, 'k')
                || key.code == KeyCode::Up
                || key.code == KeyCode::Char('+') =>
            {
                self.resize_focused_pane_height(2);
                self.pending_action = Some(PendingAction::WindowResize { pane_id });
            }
            _ if key_matches_plain_letter(&key, 'j')
                || key.code == KeyCode::Down
                || key.code == KeyCode::Char('-') =>
            {
                self.resize_focused_pane_height(-2);
                self.pending_action = Some(PendingAction::WindowResize { pane_id });
            }
            KeyCode::Char('=') => {
                self.equalize_layout();
                self.pending_action = Some(PendingAction::WindowResize { pane_id });
            }
            KeyCode::Esc | KeyCode::Enter => {
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'w') => {
                self.status = String::from("normal mode");
            }
            _ => {
                self.pending_action = Some(PendingAction::WindowResize { pane_id });
                self.status = String::from(
                    "[RESIZE] h/l: width (±4) | j/k: height (±2) | =: equal | Esc/Enter: done",
                );
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_line_mode_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 'm') => {
                self.clear_pending_count();
                self.open_prefilled_command("move ");
            }
            _ if key_matches_plain_letter(&key, 'p') => {
                self.clear_pending_count();
                self.open_prefilled_command("move-panel ");
            }
            KeyCode::Char(digit @ '1'..='9') if key.modifiers.is_empty() => {
                self.clear_pending_count();
                let target_str = digit.to_string();
                self.move_selected_to_pane_id(&target_str)?;
            }
            _ if key_matches_plain_letter(&key, 's') => {
                self.apply_line_mode(pane_id, LineMode::Size)?;
            }
            _ if key_matches_plain_letter(&key, 'r') => {
                self.apply_line_mode(pane_id, LineMode::Permissions)?;
            }
            _ if key_matches_plain_letter(&key, 'b') => {
                self.apply_line_mode(pane_id, LineMode::Btime)?;
            }
            _ if key_matches_plain_letter(&key, 't') => {
                self.apply_line_mode(pane_id, LineMode::Mtime)?;
            }
            _ if key_matches_plain_letter(&key, 'n') => {
                self.apply_line_mode(pane_id, LineMode::None)?;
            }
            KeyCode::Esc => {
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.status = String::from("normal mode");
            }
            _ => {
                self.pending_action = Some(PendingAction::LineModePicker { pane_id });
                self.status = String::from("move / linemode: choose a key from the panel");
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_yank_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
    ) -> Result<bool> {
        match key.code {
            _ if key_matches_plain_letter(&key, 'y') => {
                self.clear_pending_count();
                self.copy_selected();
            }
            _ if key_matches_plain_letter(&key, 'p') => {
                self.clear_pending_count();
                self.open_prefilled_command("copy-panel ");
            }
            KeyCode::Char(digit @ '1'..='9') if key.modifiers.is_empty() => {
                self.clear_pending_count();
                let target_str = digit.to_string();
                self.copy_selected_to_pane_id(&target_str)?;
            }
            KeyCode::Esc => {
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.status = String::from("normal mode");
            }
            _ => {
                self.pending_action = Some(PendingAction::YankPicker { pane_id });
                self.status = String::from(
                    "yank: choose a key from the panel (y: clipboard, p: panel, 1..9: pane id)",
                );
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_theme_picker_action_key(
        &mut self,
        key: KeyEvent,
        mut selected: usize,
        original: ThemePreset,
    ) -> Result<bool> {
        match key.code {
            KeyCode::Down => {
                selected = (selected + 1) % ThemePreset::ALL.len();
                self.preview_theme_picker_selection(selected, original);
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                selected = (selected + 1) % ThemePreset::ALL.len();
                self.preview_theme_picker_selection(selected, original);
            }
            KeyCode::Up => {
                selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                self.preview_theme_picker_selection(selected, original);
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                self.preview_theme_picker_selection(selected, original);
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                selected = (selected + self.take_large_move_step())
                    .min(ThemePreset::ALL.len().saturating_sub(1));
                self.preview_theme_picker_selection(selected, original);
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                selected = selected.saturating_sub(self.take_large_move_step());
                self.preview_theme_picker_selection(selected, original);
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                selected = (selected + self.take_panel_page_step())
                    .min(ThemePreset::ALL.len().saturating_sub(1));
                self.preview_theme_picker_selection(selected, original);
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                selected = selected.saturating_sub(self.take_panel_page_step());
                self.preview_theme_picker_selection(selected, original);
            }
            KeyCode::Enter => self.apply_theme(ThemePreset::ALL[selected]),
            _ if key_matches_plain_letter(&key, 'l') => {
                self.apply_theme(ThemePreset::ALL[selected])
            }
            KeyCode::Esc => {
                self.theme = original.into();
                self.status = String::from("theme picker cancelled");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.theme = original.into();
                self.status = String::from("theme picker cancelled");
            }
            _ => {
                self.pending_action = Some(PendingAction::ThemePicker { selected, original });
                self.status = String::from("theme picker: use j/k preview, l apply, h cancel");
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_copy_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        target: OpenTarget,
        mut selected: usize,
    ) -> Result<bool> {
        let options = copy_picker_options();
        if self.capture_pending_count_digit(&key) {
            self.pending_action = Some(PendingAction::CopyPicker {
                pane_id,
                target: target.clone(),
                selected,
            });
            self.status = format!("copy to clipboard: {}", target.display_name);
            return Ok(true);
        }
        match key.code {
            _ if key_matches_plain_letter(&key, 'c') => {
                self.clear_pending_count();
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'u') => {
                self.clear_pending_count();
                self.copy_target_to_system_clipboard(target.clone(), CopyAction::FileUrl)?;
            }
            _ if key_matches_plain_letter(&key, 'd') => {
                self.clear_pending_count();
                self.copy_target_to_system_clipboard(target.clone(), CopyAction::DirectoryUrl)?;
            }
            _ if key_matches_plain_letter(&key, 'f') => {
                self.clear_pending_count();
                self.copy_target_to_system_clipboard(target.clone(), CopyAction::Filename)?;
            }
            _ if key_matches_plain_letter(&key, 'n') => {
                self.clear_pending_count();
                self.copy_target_to_system_clipboard(
                    target.clone(),
                    CopyAction::FilenameWithoutExtension,
                )?;
            }
            KeyCode::Down => {
                if !options.is_empty() {
                    selected =
                        (selected + self.take_count_or_one()).min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                if !options.is_empty() {
                    selected =
                        (selected + self.take_count_or_one()).min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(self.take_count_or_one());
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                selected = selected.saturating_sub(self.take_count_or_one());
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                if !options.is_empty() {
                    selected = (selected + self.take_panel_page_step())
                        .min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                selected = selected.saturating_sub(self.take_panel_page_step());
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                if !options.is_empty() {
                    selected = (selected + self.take_large_move_step())
                        .min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                selected = selected.saturating_sub(self.take_large_move_step());
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
            KeyCode::Enter => {
                self.clear_pending_count();
                if let Some(option) = options.get(selected) {
                    self.copy_target_to_system_clipboard(target.clone(), option.action)?;
                } else {
                    self.status = String::from("copy picker: no option selected");
                }
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                if let Some(option) = options.get(selected) {
                    self.copy_target_to_system_clipboard(target.clone(), option.action)?;
                } else {
                    self.status = String::from("copy picker: no option selected");
                }
            }
            KeyCode::Esc => {
                self.clear_pending_count();
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.status = String::from("normal mode");
            }
            _ => {
                self.clear_pending_count();
                self.pending_action = Some(PendingAction::CopyPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                });
                self.status = format!("copy to clipboard: {}", target.display_name);
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_open_picker_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        target: OpenTarget,
        mut selected: usize,
        options: Vec<OpenPickerOption>,
    ) -> Result<bool> {
        if self.capture_pending_count_digit(&key) {
            self.pending_action = Some(PendingAction::OpenPicker {
                pane_id,
                target: target.clone(),
                selected,
                options: options.clone(),
            });
            self.status = format!("open with: {}", target.display_name);
            return Ok(true);
        }
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.clear_pending_count();
                self.status = String::from("normal mode");
            }
            _ if key_matches_shifted_letter(&key, 'O') => {
                self.clear_pending_count();
                self.status = String::from("normal mode");
            }
            KeyCode::Down => {
                if !options.is_empty() {
                    selected =
                        (selected + self.take_count_or_one()).min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                if !options.is_empty() {
                    selected =
                        (selected + self.take_count_or_one()).min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            KeyCode::Up => {
                selected = selected.saturating_sub(self.take_count_or_one());
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                selected = selected.saturating_sub(self.take_count_or_one());
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                if !options.is_empty() {
                    selected = (selected + self.take_panel_page_step())
                        .min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                selected = selected.saturating_sub(self.take_panel_page_step());
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                if !options.is_empty() {
                    selected = (selected + self.take_large_move_step())
                        .min(options.len().saturating_sub(1));
                }
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                selected = selected.saturating_sub(self.take_large_move_step());
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
            KeyCode::Enter => {
                self.clear_pending_count();
                if let Some(option) = options.get(selected) {
                    self.queue_open_picker_action(target.clone(), option.action.clone())?;
                } else {
                    self.status = String::from("open with: no option selected");
                }
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                if let Some(option) = options.get(selected) {
                    self.queue_open_picker_action(target.clone(), option.action.clone())?;
                } else {
                    self.status = String::from("open with: no option selected");
                }
            }
            KeyCode::Esc => {
                self.clear_pending_count();
                self.status = String::from("normal mode");
            }
            _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.status = String::from("normal mode");
            }
            _ => {
                self.clear_pending_count();
                self.pending_action = Some(PendingAction::OpenPicker {
                    pane_id,
                    target: target.clone(),
                    selected,
                    options: options.clone(),
                });
                self.status = format!("open with: {}", target.display_name);
            }
        }
        Ok(true)
    }
}
