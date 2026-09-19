use std::collections::BTreeMap;

use ratatui::layout::Rect;

use super::{
    pickers::{
        bookmark_panel_lines, bookmark_picker_copy, filtered_bookmark_entries,
        filtered_zoxide_entries, zoxide_panel_lines,
    },
    state::{App, PendingAction},
    trash::trash_confirm_panel_id,
};
use crate::file_manager::ui::{
    render_bookmark_action_picker, render_bookmark_picker, render_confirm_dialog,
    render_diff_matrix, render_go_picker, render_linemode_picker, render_paste_overwrite_dialog,
    render_sort_picker, render_theme_command_picker, render_theme_picker,
    render_trash_confirm_dialog, render_window_picker, render_window_resize_picker,
    render_yank_picker, render_zoxide_picker,
};

pub(crate) fn render_pending_action_overlay(
    app: &mut App,
    frame: &mut ratatui::Frame<'_>,
    pane_rects: &BTreeMap<usize, Rect>,
    cursor_position: &mut Option<(u16, u16)>,
) {
    match &mut app.pending_action {
        Some(PendingAction::ConfirmDelete {
            target_name,
            permanent,
            warning_message,
            ..
        }) => {
            render_confirm_dialog(
                frame,
                frame.area(),
                target_name,
                *permanent,
                warning_message.as_deref(),
                app.theme,
                &app.config,
            );
        }
        Some(PendingAction::ConfirmPasteOverwrite {
            target_name,
            entry_count,
            ..
        }) => {
            render_paste_overwrite_dialog(
                frame,
                frame.area(),
                target_name,
                *entry_count,
                app.theme,
                &app.config,
            );
        }
        Some(PendingAction::ConfirmTrashAction {
            action,
            target_name,
            entry_count,
            ..
        }) => {
            let confirm_area = trash_confirm_panel_id(action)
                .and_then(|pane_id| pane_rects.get(&pane_id).copied())
                .unwrap_or(frame.area());
            render_trash_confirm_dialog(
                frame,
                confirm_area,
                action,
                target_name,
                *entry_count,
                app.theme,
                &app.config,
            );
        }
        Some(PendingAction::GoPicker { .. }) => {
            render_go_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::ThemeCommandPicker { .. }) => {
            render_theme_command_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::SortPicker { .. }) => {
            render_sort_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::WindowPicker { .. }) => {
            render_window_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::WindowResize { .. }) => {
            render_window_resize_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::LineModePicker { .. }) => {
            render_linemode_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::YankPicker { .. }) => {
            render_yank_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::BookmarkPicker { .. }) => {
            render_bookmark_action_picker(frame, frame.area(), app.theme);
        }
        Some(PendingAction::ThemePicker { selected, .. }) => {
            render_theme_picker(frame, frame.area(), app.theme, *selected, &app.config);
        }
        Some(PendingAction::BookmarkList {
            pane_id,
            selected,
            mode,
            search,
        }) => {
            let filtered = filtered_bookmark_entries(app.bookmark_store.list(), &search.buffer);
            let lines = bookmark_panel_lines(filtered);
            if let Some(area) = pane_rects.get(pane_id) {
                let (title, empty_message) = bookmark_picker_copy(*mode);
                let bookmark_cursor = render_bookmark_picker(
                    frame,
                    *area,
                    app.theme,
                    &lines,
                    *selected,
                    title,
                    empty_message,
                    &search.buffer,
                    search.editing,
                    app.text_input_cursor,
                );
                if search.editing && cursor_position.is_none() {
                    *cursor_position = bookmark_cursor;
                }
            }
        }
        Some(PendingAction::ZoxideList {
            pane_id,
            selected,
            entries,
            search,
        }) => {
            let filtered = filtered_zoxide_entries(entries, &search.buffer);
            let lines = zoxide_panel_lines(filtered);
            if let Some(area) = pane_rects.get(pane_id) {
                let zoxide_cursor = render_zoxide_picker(
                    frame,
                    *area,
                    app.theme,
                    &lines,
                    *selected,
                    &search.buffer,
                    search.editing,
                    app.text_input_cursor,
                );
                if search.editing && cursor_position.is_none() {
                    *cursor_position = zoxide_cursor;
                }
            }
        }
        Some(PendingAction::DiffMatrix(state)) => {
            render_diff_matrix(frame, frame.area(), state, app.theme);
        }
        Some(PendingAction::TrashPanel { .. })
        | Some(PendingAction::TaskPanel { .. })
        | Some(PendingAction::HelpPanel { .. })
        | Some(PendingAction::ToolPanel { .. })
        | Some(PendingAction::CopyPicker { .. })
        | Some(PendingAction::OpenPicker { .. })
        | Some(PendingAction::RegexRename { .. })
        | Some(PendingAction::EasyMotion { .. }) => {}
        Some(PendingAction::Rename { .. }) | Some(PendingAction::CreateEntry { .. }) => {}
        None => {}
    }
}
