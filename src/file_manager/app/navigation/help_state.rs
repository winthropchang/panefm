//! Help 互動狀態快照/回復與 pending action 狀態格式化。

use std::io;
use std::mem;

use super::super::*;
use crate::theme::ThemePreset;

impl App {
    /// 以目前正在操作的上下文為返回點，打開全局 help 字典。
    pub(crate) fn open_help_from_current(&mut self) {
        self.help_return = self.capture_help_return_state();
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
            custom_title: None,
            custom_entries: None,
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 以目前正在操作的上下文為基礎，打開專屬的快捷鍵 Cheatsheet。
    pub(crate) fn open_cheatsheet_from_current(&mut self) {
        let kind = self.active_context_help_kind();
        let (title, entries) = context_cheatsheet_entries(kind);
        let count = entries.len();
        self.help_return = self.capture_help_return_state();
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
            custom_title: Some(title.clone()),
            custom_entries: Some(entries),
        });
        self.status = format!("{title} ({count} keys) (?/Esc/q to return)");
    }

    /// 擷取目前互動狀態，供 help 面板關閉後回復。
    pub(crate) fn capture_help_return_state(&mut self) -> Option<HelpReturnState> {
        if let Some(action) = self.pending_action.take() {
            return Some(HelpReturnState::Pending(action));
        }
        if let Some(filter) = self.filter.take() {
            return Some(HelpReturnState::Filter(filter));
        }
        if let Some(search) = self.preview_search.take() {
            return Some(HelpReturnState::PreviewSearch(search));
        }
        if let Some(search) = self.list_find.take() {
            return Some(HelpReturnState::ListFind(search));
        }
        if let Some(search) = self.global_search.take() {
            self.cancel_global_search();
            return Some(HelpReturnState::GlobalSearch(search));
        }
        if let Some(selection) = self.visual_selection.take() {
            return Some(HelpReturnState::VisualSelection(selection));
        }
        if self.command_mode {
            self.command_mode = false;
            self.command_completion_cycle = None;
            return Some(HelpReturnState::CommandMode(mem::take(
                &mut self.command_buffer,
            )));
        }
        if let Some(prompt) = self.pending_bookmark.take() {
            return Some(HelpReturnState::PendingBookmark(prompt));
        }
        if let Some(pane) = self.panes.get_mut(&self.focused_pane)
            && pane.is_preview_active()
        {
            pane.set_preview_active(false);
            return Some(HelpReturnState::PreviewFocus(self.focused_pane));
        }
        None
    }

    /// 從 help 面板回到先前的互動上下文。
    pub(crate) fn restore_help_return_state(&mut self, preserve_status: bool) -> io::Result<()> {
        let previous_status = self.status.clone();
        let Some(state) = self.help_return.take() else {
            self.status = String::from("normal mode");
            return Ok(());
        };

        match state {
            HelpReturnState::Pending(action) => {
                self.status = self.status_for_pending_action(&action)?;
                self.pending_action = Some(action);
            }
            HelpReturnState::Filter(filter) => {
                self.status = if filter.editing {
                    if filter.buffer.is_empty() {
                        String::from("filter: all")
                    } else {
                        format!("filter: {}", filter.buffer)
                    }
                } else if filter.buffer.is_empty() {
                    String::from("filter active")
                } else {
                    format!("filter locked: {}", filter.buffer)
                };
                self.filter = Some(filter);
            }
            HelpReturnState::PreviewSearch(search) => {
                self.status =
                    preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
                self.preview_search = Some(search);
            }
            HelpReturnState::ListFind(search) => {
                self.status =
                    list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
                self.list_find = Some(search);
            }
            HelpReturnState::GlobalSearch(search) => {
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            HelpReturnState::VisualSelection(selection) => {
                self.visual_selection = Some(selection);
                self.status = self.visual_status_label();
            }
            HelpReturnState::CommandMode(buffer) => {
                self.open_prefilled_command(buffer);
            }
            HelpReturnState::PendingBookmark(prompt) => {
                self.pending_bookmark = Some(prompt);
                self.status = match prompt {
                    BookmarkPrompt::Jump => String::from("bookmark: press a key to jump"),
                };
            }
            HelpReturnState::PreviewFocus(pane_id) => {
                if let Some(pane) = self.panes.get_mut(&pane_id) {
                    pane.set_preview_active(true);
                    self.status = String::from("preview mode");
                } else {
                    self.status = String::from("panel no longer exists");
                }
            }
        }

        if preserve_status {
            self.status = previous_status;
        }

        Ok(())
    }

    /// 依 pending action 類型回傳適合顯示的狀態文字。
    pub(crate) fn status_for_pending_action(&self, action: &PendingAction) -> io::Result<String> {
        Ok(match action {
            PendingAction::ConfirmDelete {
                target_name,
                permanent,
                ..
            } => {
                if *permanent {
                    format!("confirm delete {target_name}: y/n")
                } else {
                    format!("confirm trash {target_name}: y/n")
                }
            }
            PendingAction::ConfirmPasteOverwrite {
                target_name,
                entry_count,
                ..
            } => paste_overwrite_confirm_status(target_name, *entry_count),
            PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                ..
            } => trash_confirm_status(action, target_name, *entry_count),
            PendingAction::GoPicker { .. } => String::from("go: choose g/t/d/k/l from the panel"),
            PendingAction::ThemeCommandPicker { .. } => {
                String::from("theme/trash: choose l/n/t/u from the panel")
            }
            PendingAction::SortPicker { .. } => String::from("sort: choose a key from the panel"),
            PendingAction::WindowPicker { .. } => {
                String::from("panel: choose h/j/k/l/r/=/c/o/t/d from the panel")
            }
            PendingAction::WindowResize { .. } => String::from(
                "[RESIZE] h/l: width (±4) | j/k: height (±2) | =: equal | Esc/Enter: done",
            ),
            PendingAction::LineModePicker { .. } => {
                String::from("move / linemode: choose a key from the panel")
            }
            PendingAction::YankPicker { .. } => String::from(
                "yank: choose a key from the panel (y: clipboard, p: panel, 1..9: pane id)",
            ),
            PendingAction::ThemePicker { selected, .. } => {
                format!("theme picker: {}", ThemePreset::ALL[*selected].name())
            }
            PendingAction::TaskPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            } => {
                let filtered =
                    filtered_task_entries(&self.tasks_for_pane(*pane_id), &search.buffer);
                if let Some(anchor) = visual_anchor {
                    self.task_visual_status_label(*anchor, *selected, marked_ids.len())
                } else {
                    task_panel_status(
                        &search.buffer,
                        filtered.len(),
                        *selected,
                        search.editing,
                        marked_ids.len(),
                    )
                }
            }
            PendingAction::BookmarkPicker { .. } => {
                String::from("bookmark: choose a/g/d/D from the panel")
            }
            PendingAction::TrashPanel {
                selected,
                search,
                marked_ids,
                ..
            } => {
                let visible = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
                trash_panel_status(
                    &search.buffer,
                    visible,
                    *selected,
                    search.editing,
                    marked_ids.len(),
                )
            }
            PendingAction::HelpPanel { search, .. } => help_panel_status(
                &search.buffer,
                help_entries(&search.buffer).len(),
                search.editing,
            ),
            PendingAction::ToolPanel { .. } => String::from("dependencies: j/k move, Esc close"),
            PendingAction::EasyMotion { target_char, .. } => {
                if let Some(c) = target_char {
                    format!("-- EASYMOTION [{c}] -- (press label to jump, Esc to cancel)")
                } else {
                    String::from("-- EASYMOTION -- (type target char, Esc to cancel)")
                }
            }
            PendingAction::BookmarkList {
                selected,
                mode,
                search,
                ..
            } => {
                let filtered =
                    filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer);
                bookmark_list_status(
                    &search.buffer,
                    filtered.len(),
                    *selected,
                    *mode,
                    search.editing,
                )
            }
            PendingAction::ZoxideList {
                entries,
                selected,
                search,
                ..
            } => {
                let filtered = filtered_zoxide_entries(entries, &search.buffer);
                zoxide_list_status(&search.buffer, filtered.len(), *selected, search.editing)
            }
            PendingAction::CopyPicker { target, .. } => {
                format!("copy to clipboard: {}", target.display_name)
            }
            PendingAction::OpenPicker { target, .. } => {
                format!("open with: {}", target.display_name)
            }
            PendingAction::Rename { mode, .. } => match mode {
                RenameMode::Insert => String::from("rename: insert"),
                RenameMode::Normal => String::from("rename: normal"),
            },
            PendingAction::CreateEntry { mode, .. } => match mode {
                RenameMode::Insert => create_status_label("insert"),
                RenameMode::Normal => create_status_label("normal"),
            },
            PendingAction::RegexRename {
                previews,
                pattern,
                replacement,
                ..
            } => regex_rename_status(pattern, replacement, previews),
            PendingAction::DiffMatrix(state) => {
                format!(
                    "diff matrix: {} items (filter: {})",
                    state.filtered_indices.len(),
                    state.filter_mode.label()
                )
            }
        })
    }
}
