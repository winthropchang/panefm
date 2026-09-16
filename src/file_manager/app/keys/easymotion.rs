use super::*;

impl App {
    pub(crate) fn handle_easymotion_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        target_char: Option<char>,
        labels: Vec<(char, usize)>,
    ) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.status = String::from("easymotion cancelled");
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty()
                    || key.modifiers == KeyModifiers::NONE
                    || key.modifiers == KeyModifiers::SHIFT)
                    && target_char.is_none() =>
            {
                let Some(pane) = self.panes.get_mut(&pane_id) else {
                    return Ok(true);
                };
                let visible_total = pane.visible_indices.len();
                let viewport_height = pane.list_viewport_height;
                let (view_start, view_end) = crate::file_manager::ui::visible_list_window_range(
                    visible_total,
                    pane.selected,
                    viewport_height,
                    pane.list_state.offset(),
                );
                let lower_c = c.to_ascii_lowercase();
                let mut matches = Vec::new();
                for visible_idx in view_start..view_end {
                    if let Some(&entry_idx) = pane.visible_indices.get(visible_idx)
                        && let Some(entry) = pane.entries.get(entry_idx)
                    {
                        let name_lower = entry.name.to_lowercase();
                        let name_trimmed_lower = entry.name.trim_start_matches('.').to_lowercase();
                        if name_lower.starts_with(lower_c)
                            || name_trimmed_lower.starts_with(lower_c)
                        {
                            matches.push(visible_idx);
                        }
                    }
                }

                if matches.is_empty() {
                    self.status = format!("easymotion: no items starting with '{}'", c);
                } else if matches.len() == 1 {
                    pane.move_to_visible_index(matches[0]);
                    let entry_name = pane
                        .selected_entry()
                        .map(|e| e.name.clone())
                        .unwrap_or_default();
                    self.status = format!("jumped to {}", entry_name);
                } else {
                    let mut new_labels = Vec::new();
                    for (i, &visible_idx) in matches.iter().enumerate() {
                        if let Some(&key_char) =
                            crate::file_manager::preview::EASYMOTION_KEYS.get(i)
                        {
                            new_labels.push((key_char, visible_idx));
                        }
                    }
                    self.status =
                        format!("-- EASYMOTION [{c}] -- (press label to jump, Esc to cancel)");
                    self.pending_action = Some(PendingAction::EasyMotion {
                        pane_id,
                        target_char: Some(c),
                        labels: new_labels,
                    });
                }
            }
            KeyCode::Char(c)
                if (key.modifiers.is_empty()
                    || key.modifiers == KeyModifiers::NONE
                    || key.modifiers == KeyModifiers::SHIFT)
                    && target_char.is_some() =>
            {
                let lower_c = c.to_ascii_lowercase();
                if let Some(&(_, target_visible_idx)) = labels
                    .iter()
                    .find(|(ch, _)| ch.to_ascii_lowercase() == lower_c)
                {
                    if let Some(pane) = self.panes.get_mut(&pane_id) {
                        pane.move_to_visible_index(target_visible_idx);
                        let entry_name = pane
                            .selected_entry()
                            .map(|e| e.name.clone())
                            .unwrap_or_default();
                        self.status = format!("jumped to {}", entry_name);
                    }
                } else {
                    self.status = format!("easymotion: no match for '{}'", c);
                }
            }
            _ => {
                self.status = String::from("easymotion cancelled");
            }
        }
        Ok(true)
    }

    pub(crate) fn handle_tool_panel_action_key(
        &mut self,
        key: KeyEvent,
        pane_id: usize,
        mut selected: usize,
    ) -> Result<bool> {
        let tools = external_tool_statuses();
        let len = tools.len();
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.status = String::from("dependency panel closed");
            }
            _ if key_matches_plain_letter(&key, 'j') || key.code == KeyCode::Down => {
                selected = if len == 0 {
                    0
                } else {
                    (selected + 1).min(len - 1)
                };
                self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
            }
            _ if key_matches_plain_letter(&key, 'k') || key.code == KeyCode::Up => {
                selected = selected.saturating_sub(1);
                self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
            }
            _ => {
                self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
            }
        }
        Ok(true)
    }
}
