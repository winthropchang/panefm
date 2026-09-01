#![allow(unused_imports)]

use super::*;

impl App {
    /// 處理 command mode 中的按鍵編輯、候選切換與送出行為。
    pub(crate) fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
        let suggestions = self.command_suggestions();
        let path_completion_active =
            command_path_completion_context(self.current_pane_cwd(), &self.command_buffer)
                .is_some();
        if self.text_input_mode == RenameMode::Insert
            && path_completion_active
            && (key.code == KeyCode::Tab || key.code == KeyCode::BackTab)
            && !suggestions.is_empty()
        {
            self.apply_path_completion_tab_cycle(key.code == KeyCode::BackTab, &suggestions);
            return Ok(true);
        }
        if self.text_input_mode == RenameMode::Insert
            && !path_completion_active
            && (key.code == KeyCode::Tab || key.code == KeyCode::BackTab)
            && !suggestions.is_empty()
        {
            self.apply_command_completion_tab(key.code == KeyCode::BackTab, &suggestions);
            return Ok(true);
        }
        if self.text_input_mode == RenameMode::Insert
            && let Some(direction) = command_suggestion_navigation(&key)
        {
            if !suggestions.is_empty() {
                match direction {
                    SuggestionNavigation::Next => {
                        self.command_suggestion_selected =
                            (self.command_suggestion_selected + 1) % suggestions.len();
                    }
                    SuggestionNavigation::Previous => {
                        self.command_suggestion_selected =
                            (self.command_suggestion_selected + suggestions.len() - 1)
                                % suggestions.len();
                    }
                }
            }
            return Ok(true);
        }

        let mut buffer = std::mem::take(&mut self.command_buffer);
        let edit_result = self.edit_text_buffer(&mut buffer, &key);
        self.command_buffer = buffer;
        match edit_result {
            TextEditResult::Changed => {
                self.command_suggestion_selected = 0;
                self.command_completion_cycle = None;
                return Ok(true);
            }
            TextEditResult::Consumed => return Ok(true),
            TextEditResult::PassThrough => {}
        }

        match key.code {
            KeyCode::Esc => {
                self.command_mode = false;
                self.command_buffer.clear();
                self.command_suggestion_selected = 0;
                self.command_completion_cycle = None;
                self.status = String::from("normal mode");
            }
            KeyCode::Enter => {
                let selected_suggestion = suggestions
                    .get(
                        self.command_suggestion_selected
                            .min(suggestions.len().saturating_sub(1)),
                    )
                    .map(|entry| entry.command.trim_start_matches(':').to_string());
                let current = self.command_buffer.trim();
                let has_arguments = current.contains(char::is_whitespace);
                let path_like_input = looks_like_navigation_path(current);
                if let Some(suggestion) = selected_suggestion
                    && !suggestion.is_empty()
                    && suggestion != current
                    && !has_arguments
                    && !path_like_input
                {
                    self.command_buffer = suggestion;
                    self.text_input_cursor = self.command_buffer.chars().count();
                } else {
                    let command = std::mem::take(&mut self.command_buffer);
                    self.command_mode = false;
                    self.command_suggestion_selected = 0;
                    self.command_completion_cycle = None;
                    self.execute_command(command.trim())?;
                }
            }
            _ => {}
        }
        Ok(true)
    }

    /// 根據目前焦點 pane 與 command buffer，取得 command palette 應顯示的候選。
    pub(crate) fn command_suggestions(&self) -> Vec<CommandSuggestionLine> {
        self.command_completion_cycle.clone().map_or_else(
            || command_suggestions_for_buffer(self.current_pane_cwd(), &self.command_buffer),
            |cycle| cycle.suggestions,
        )
    }

    /// 取得目前焦點 pane 的工作目錄，供 command palette 的路徑補全使用。
    pub(crate) fn current_pane_cwd(&self) -> Option<&Path> {
        self.panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.as_path())
    }

    /// 在路徑補全模式下處理 `Tab` / `Shift+Tab`，提供共同前綴補齊與候選輪詢。
    pub(crate) fn apply_path_completion_tab_cycle(
        &mut self,
        reverse: bool,
        suggestions: &[CommandSuggestionLine],
    ) {
        if suggestions.is_empty() {
            return;
        }

        if suggestions.len() == 1 {
            self.command_buffer = suggestions[0].command.clone();
            self.text_input_cursor = self.command_buffer.chars().count();
            self.command_suggestion_selected = 0;
            self.command_completion_cycle = None;
            return;
        }

        let current = self.command_buffer.clone();
        let common_prefix = longest_common_prefix(
            &suggestions
                .iter()
                .map(|line| line.command.as_str())
                .collect::<Vec<_>>(),
        );
        if common_prefix.chars().count() > current.chars().count() {
            self.command_buffer = common_prefix;
            self.text_input_cursor = self.command_buffer.chars().count();
            self.command_suggestion_selected = 0;
            self.command_completion_cycle = None;
            return;
        }

        let len = suggestions.len();
        let selected = match &self.command_completion_cycle {
            Some(cycle) if cycle.suggestions == suggestions => {
                if reverse {
                    (self.command_suggestion_selected + len - 1) % len
                } else {
                    (self.command_suggestion_selected + 1) % len
                }
            }
            _ => {
                if reverse {
                    len - 1
                } else {
                    0
                }
            }
        };

        self.command_suggestion_selected = selected;
        self.command_buffer = suggestions[selected].command.clone();
        self.text_input_cursor = self.command_buffer.chars().count();
        self.command_completion_cycle = Some(CommandCompletionCycle {
            suggestions: suggestions.to_vec(),
        });
    }

    /// 在一般 command 提示模式下處理 `Tab` / `Shift+Tab`，直接採用最接近的候選。
    ///
    /// `Tab` 會使用目前選取的提示；若使用者尚未手動切換，預設採用排序後的第一筆。
    /// `Shift+Tab` 則會先往上一筆，再套用該提示，方便快速反向挑選。
    pub(crate) fn apply_command_completion_tab(
        &mut self,
        reverse: bool,
        suggestions: &[CommandSuggestionLine],
    ) {
        if suggestions.is_empty() {
            return;
        }

        let len = suggestions.len();
        let selected = if reverse {
            (self.command_suggestion_selected + len - 1) % len
        } else {
            self.command_suggestion_selected.min(len - 1)
        };

        self.command_suggestion_selected = selected;
        self.command_buffer = suggestions[selected]
            .command
            .trim_start_matches(':')
            .to_string();
        self.text_input_cursor = self.command_buffer.chars().count();
        self.command_completion_cycle = None;
    }

    /// 執行 command mode 送出的命令字串。
    pub(crate) fn execute_command(&mut self, command: &str) -> Result<()> {
        match command {
            "q" => self.status = String::from("use q in normal mode to quit"),
            "goto" => self.status = String::from("usage: goto <path>"),
            "rename" => self.start_rename(),
            "create" => self.start_create_entry(),
            "copy" => self.copy_selected(),
            "mark-toggle" => self.toggle_mark_selected_in_focused_pane()?,
            "cancel-copy" => self.clear_clipboard(ClipboardOperation::Copy),
            "cancel-cut" => self.clear_clipboard(ClipboardOperation::Cut),
            "cut" => self.cut_selected(),
            "paste" => self.paste_into_focused_pane()?,
            "paste!" => self.paste_into_focused_pane_with_overwrite()?,
            "undo" => self.undo_latest_file_operation()?,
            "terminal" => self.open_terminal_in_active_panel()?,
            "compress" => self.compress_selected_entries()?,
            "extract" => self.extract_selected_archives()?,
            "jump" => self.open_fzf_jump(),
            "open" => self.open_selected_with_default()?,
            "open-picker" => self.open_selected_with_picker()?,
            "copy-picker" => self.open_copy_picker()?,
            "mark-all" | "select-all" => self.mark_all_in_focused_pane()?,
            "mark-invert" | "invert-selection" => self.invert_marks_in_focused_pane()?,
            "split" => self.split_current(SplitDirection::Horizontal)?,
            "vsplit" => self.split_current(SplitDirection::Vertical)?,
            "split-up" => {
                self.split_current_at(SplitDirection::Horizontal, SplitPlacement::Before)?
            }
            "split-left" => {
                self.split_current_at(SplitDirection::Vertical, SplitPlacement::Before)?
            }
            "vim" => {
                let Some(target) = self.selected_open_target() else {
                    self.status = String::from("nothing selected to open");
                    return Ok(());
                };
                self.queue_open_action(target, OpenAction::Vim)?;
            }
            "reveal" => {
                let Some(target) = self.selected_open_target() else {
                    self.status = String::from("nothing selected to reveal");
                    return Ok(());
                };
                self.queue_open_action(target, OpenAction::Reveal)?;
            }
            "unmark" | "unmark-all" => self.clear_marks_in_focused_pane()?,
            "preview" => self.open_preview_focus(),
            "preview-search" => self.open_preview_search_input(),
            "search" => self.open_global_search()?,
            "filter" => self.open_filter_input(FilterMode::Normal),
            "filter-fuzzy" | "filter fuzzy" | "fuzzy" => self.open_filter_input(FilterMode::Fuzzy),
            "linemode" => self.open_linemode_picker(),
            "bookmark" => self.open_bookmark_picker(),
            "bookmark add" => self.add_bookmark_with_auto_key(self.focused_pane)?,
            "bookmark jump" => self.open_bookmark_list(),
            "bookmark delete" => {
                self.open_bookmark_list_with_mode(self.focused_pane, BookmarkListMode::Delete);
            }
            "bookmark clear" => self.delete_all_bookmarks()?,
            "trash" => self.open_trash_panel()?,
            "tasks" => self.open_task_panel(),
            "status" => self.open_tool_panel(),
            "help" => self.open_help_panel(),
            "bookmark list" => self.open_bookmark_list(),
            "zoxide" => self.open_zoxide_list(),
            "trash undo" => self.restore_latest_from_trash()?,
            "delete" => self.start_delete_confirmation(false),
            "delete!" | "delete-permanently" => self.start_delete_confirmation(true),
            "theme list" => self.open_theme_picker(),
            "theme next" => self.cycle_theme(),
            "panel" | "pane" => {
                self.status = format!(
                    "usage: panel <panel-id>. available: {}",
                    self.available_pane_ids_label()
                );
            }
            "close" => self.close_current_pane(),
            "only" => self.only_current_pane(),
            "diff" | "df" | "d" => self.open_diff_matrix(None)?,
            "rename-regex" | "reg" => {
                self.status = String::from("usage: rename-regex <pattern> <replace>");
            }
            "" => self.status = String::from("normal mode"),
            other => {
                if let Some(args) = other
                    .strip_prefix("diff ")
                    .or_else(|| other.strip_prefix("df "))
                    .or_else(|| other.strip_prefix("d "))
                {
                    let ids = args
                        .split_whitespace()
                        .filter_map(|s| s.parse::<usize>().ok())
                        .collect::<Vec<_>>();
                    self.open_diff_matrix(Some(ids))?;
                } else if let Some(name) = other.strip_prefix("theme ") {
                    self.set_theme_by_name(name.trim());
                } else if let Some(path) = other.strip_prefix("goto ") {
                    self.change_directory_from_command(path.trim())?;
                } else if let Some(name) = other.strip_prefix("create ") {
                    self.create_entry_from_command(name)?;
                } else if let Some(args) = other
                    .strip_prefix("rename-regex ")
                    .or_else(|| other.strip_prefix("reg "))
                {
                    self.start_regex_rename_from_command(args)?;
                } else if let Some(args) = other.strip_prefix("move-panel ") {
                    self.move_selected_to_pane_id(args.trim())?;
                } else if let Some(args) = other.strip_prefix("panel ") {
                    self.focus_pane_by_id_argument(args.trim());
                } else if let Some(args) = other.strip_prefix("pane ") {
                    self.focus_pane_by_id_argument(args.trim());
                } else if let Some(path) = other.strip_prefix("move ") {
                    self.move_selected_to_path(path.trim())?;
                } else if let Some(args) = other.strip_prefix("linemode ") {
                    self.apply_line_mode_from_command(args.trim())?;
                } else if let Some(args) = other.strip_prefix("bookmark jump ") {
                    self.jump_to_bookmark_from_command(args.trim())?;
                } else if let Some(args) = other.strip_prefix("bookmark delete ") {
                    let Some(key) = parse_bookmark_argument(args) else {
                        self.status = String::from("usage: bookmark delete <key>");
                        return Ok(());
                    };
                    self.delete_bookmark(key)?;
                } else if looks_like_navigation_path(other) {
                    self.change_directory_from_command(other)?;
                } else {
                    self.status = format!("unknown command: {other}");
                }
            }
        }
        Ok(())
    }
}
