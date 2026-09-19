//! 頂層待處理操作（PendingAction）按鍵分派與暫時面板互動處理。

use anyhow::Result;
use crossterm::event::KeyEvent;

use super::super::*;

impl App {
    /// 處理目前 pending action 所代表的暫時面板、選單或確認視窗。
    ///
    /// 參數：`key: KeyEvent`，要交給最上層暫時 UI 的按鍵。
    /// 回傳：`Result<bool>`，暫時 UI 一律消耗事件並回傳 `true`；檔案操作失敗則回傳
    /// error。函數開頭先 `take()` action，只有需要保留 UI 的分支才放回去，因此某
    /// 分支若未重設 `pending_action`，語意就是操作完成並關閉面板。
    pub(crate) fn handle_pending_action_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut action) = self.pending_action.take() else {
            return Ok(true);
        };

        // 所有具搜尋框的列表先共用文字編輯器。Normal 模式的 h/l 不可落入下面的
        // panel 導航，否則使用者只想移動輸入游標時會意外關閉或執行選項。
        let panel_edit_result = match &mut action {
            PendingAction::TrashPanel {
                selected, search, ..
            }
            | PendingAction::HelpPanel {
                selected, search, ..
            }
            | PendingAction::TaskPanel {
                selected, search, ..
            }
            | PendingAction::BookmarkList {
                selected, search, ..
            }
            | PendingAction::ZoxideList {
                selected, search, ..
            } if search.editing => {
                let result = self.edit_text_buffer(&mut search.buffer, &key);
                if matches!(result, TextEditResult::Changed) {
                    *selected = 0;
                }
                Some(result)
            }
            _ => None,
        };
        if panel_edit_result.is_some_and(|result| {
            matches!(result, TextEditResult::Changed | TextEditResult::Consumed)
        }) {
            self.status = self.status_for_pending_action(&action)?;
            self.pending_action = Some(action);
            return Ok(true);
        }

        match action {
            PendingAction::EasyMotion {
                pane_id,
                target_char,
                labels,
            } => self.handle_easymotion_action_key(key, pane_id, target_char, labels),
            PendingAction::ToolPanel { pane_id, selected } => {
                self.handle_tool_panel_action_key(key, pane_id, selected)
            }
            PendingAction::ConfirmDelete {
                pane_id,
                target_name,
                permanent,
                warning_message,
            } => self.handle_confirm_delete_action_key(
                key,
                pane_id,
                target_name,
                permanent,
                warning_message,
            ),
            PendingAction::ConfirmPasteOverwrite {
                pane_id,
                target_name,
                entry_count,
                operation,
            } => self.handle_confirm_paste_overwrite_action_key(
                key,
                pane_id,
                target_name,
                entry_count,
                operation,
            ),
            PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                marked_ids,
                visual_anchor,
            } => self.handle_confirm_trash_action_key(
                key,
                action,
                target_name,
                entry_count,
                marked_ids,
                visual_anchor,
            ),
            PendingAction::GoPicker { pane_id } => self.handle_go_picker_action_key(key, pane_id),
            PendingAction::ThemeCommandPicker { pane_id } => {
                self.handle_theme_command_picker_action_key(key, pane_id)
            }
            PendingAction::SortPicker { pane_id } => {
                self.handle_sort_picker_action_key(key, pane_id)
            }
            PendingAction::WindowPicker { pane_id } => {
                self.handle_window_picker_action_key(key, pane_id)
            }
            PendingAction::WindowResize { pane_id } => {
                self.handle_window_resize_action_key(key, pane_id)
            }
            PendingAction::LineModePicker { pane_id } => {
                self.handle_line_mode_picker_action_key(key, pane_id)
            }
            PendingAction::YankPicker { pane_id } => {
                self.handle_yank_picker_action_key(key, pane_id)
            }
            PendingAction::ThemePicker { selected, original } => {
                self.handle_theme_picker_action_key(key, selected, original)
            }
            PendingAction::TrashPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            } => self.handle_trash_panel_action_key(
                key,
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            ),
            PendingAction::HelpPanel {
                pane_id,
                selected,
                search,
                custom_title,
                custom_entries,
            } => self.handle_help_panel_action_key(
                key,
                pane_id,
                selected,
                search,
                custom_title,
                custom_entries,
            ),
            PendingAction::TaskPanel {
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            } => self.handle_task_panel_action_key(
                key,
                pane_id,
                selected,
                search,
                marked_ids,
                visual_anchor,
            ),
            PendingAction::BookmarkPicker { pane_id } => {
                self.handle_bookmark_picker_action_key(key, pane_id)
            }
            PendingAction::BookmarkList {
                pane_id,
                selected,
                mode,
                search,
            } => self.handle_bookmark_list_action_key(key, pane_id, selected, mode, search),
            PendingAction::ZoxideList {
                pane_id,
                selected,
                entries,
                search,
            } => self.handle_zoxide_list_action_key(key, pane_id, selected, entries, search),
            PendingAction::CopyPicker {
                pane_id,
                target,
                selected,
            } => self.handle_copy_picker_action_key(key, pane_id, target, selected),
            PendingAction::OpenPicker {
                pane_id,
                target,
                selected,
                options,
            } => self.handle_open_picker_action_key(key, pane_id, target, selected, options),
            PendingAction::Rename {
                pane_id,
                original_name,
                buffer,
                cursor,
                mode,
            } => self.handle_rename_action_key(key, pane_id, original_name, buffer, cursor, mode),
            PendingAction::CreateEntry {
                pane_id,
                buffer,
                cursor,
                mode,
            } => self.handle_create_entry_action_key(key, pane_id, buffer, cursor, mode),
            PendingAction::RegexRename {
                pane_id,
                pattern,
                replacement,
                selected,
                previews,
            } => self.handle_regex_rename_action_key(
                key,
                pane_id,
                pattern,
                replacement,
                selected,
                previews,
            ),
            PendingAction::DiffMatrix(state) => self.handle_diff_matrix_action_key(key, state),
        }
    }
}
