use std::{collections::HashMap, path::PathBuf};

use ratatui::layout::Rect;

use crate::file_manager::{
    copy::copy_picker_options,
    tools::ToolStatus,
    ui::{
        InlineEditorState, InlinePickerState, PaneListState, SearchListState, render_pane,
        visible_list_window_range,
    },
};

use super::{
    help::{filter_custom_help_entries, help_panel_lines},
    line_editor::create_editor_title,
    rename::regex_rename_panel_lines,
    render::filtered_global_search_entries,
    state::{App, PendingAction, RenameMode, SearchMode},
    tasks::{filtered_task_entries, task_panel_lines},
    trash::{trash_panel_lines, trash_panel_overlay_state_from_pending_action},
};

pub(crate) fn render_single_pane(
    app: &mut App,
    frame: &mut ratatui::Frame<'_>,
    pane_id: usize,
    rect: Rect,
    tool_statuses: &[ToolStatus],
) -> Option<(u16, u16)> {
    let trash_overlay_state =
        trash_panel_overlay_state_from_pending_action(&app.pending_action, pane_id);
    let trash_lines =
        if let Some((selected, search, marked_ids, visual_anchor)) = trash_overlay_state.as_ref() {
            Some(
                trash_panel_lines(
                    &app.trash_store,
                    &search.buffer,
                    marked_ids,
                    visual_anchor.map(|anchor| (anchor, *selected)),
                )
                .unwrap_or_default(),
            )
        } else {
            None
        };
    let task_records = if matches!(
        &app.pending_action,
        Some(PendingAction::TaskPanel {
            pane_id: action_pane_id,
            ..
        }) if *action_pane_id == pane_id
    ) {
        Some(app.tasks_for_pane(pane_id))
    } else {
        None
    };
    let task_lines = if let (
        Some(records),
        Some(PendingAction::TaskPanel {
            pane_id: action_pane_id,
            search,
            marked_ids,
            visual_anchor,
            selected,
        }),
    ) = (task_records.as_ref(), app.pending_action.as_ref())
    {
        if *action_pane_id == pane_id {
            let filtered = filtered_task_entries(records, &search.buffer);
            let mut effective_marked = marked_ids.clone();
            if let Some(anchor) = visual_anchor {
                let start = (*anchor).min(*selected);
                let end = (*anchor).max(*selected);
                for task in filtered
                    .iter()
                    .skip(start)
                    .take(end.saturating_sub(start) + 1)
                {
                    if !effective_marked.contains(&task.id) {
                        effective_marked.push(task.id);
                    }
                }
            }
            Some(task_panel_lines(&filtered, &effective_marked))
        } else {
            None
        }
    } else {
        None
    };
    let help_lines = if let Some(PendingAction::HelpPanel {
        pane_id: action_pane_id,
        search,
        custom_entries,
        ..
    }) = &app.pending_action
    {
        if *action_pane_id == pane_id {
            Some(if let Some(custom) = custom_entries {
                filter_custom_help_entries(custom, &search.buffer)
                    .into_iter()
                    .map(|e| e.line)
                    .collect()
            } else {
                help_panel_lines(&search.buffer)
            })
        } else {
            None
        }
    } else {
        None
    };
    let regex_rename_lines = if let Some(PendingAction::RegexRename {
        pane_id: action_pane_id,
        previews,
        ..
    }) = &app.pending_action
    {
        if *action_pane_id == pane_id {
            Some(regex_rename_panel_lines(previews))
        } else {
            None
        }
    } else {
        None
    };
    let global_search_results = app
        .global_search
        .as_ref()
        .filter(|search| search.pane_id == pane_id)
        .map(|search| filtered_global_search_entries(&search.results, &search.filter.buffer));
    let active_job_badges = if app.active_file_job_busy_paths.is_empty() {
        HashMap::new()
    } else {
        app.panes
            .get(&pane_id)
            .map(|pane| {
                visible_job_badge_paths(pane, rect.height.saturating_sub(2) as usize)
                    .into_iter()
                    .filter_map(|path| {
                        app.active_job_badge_for_path(&path)
                            .map(|badge| (path, badge))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default()
    };

    if let Some(pane) = app.panes.get_mut(&pane_id) {
        let rename_buffer = match &app.pending_action {
            Some(PendingAction::Rename {
                pane_id: rename_pane_id,
                buffer,
                cursor,
                mode,
                ..
            }) if *rename_pane_id == pane_id => Some(InlineEditorState {
                buffer: buffer.as_str(),
                cursor: *cursor,
                title: match mode {
                    RenameMode::Insert => " Rename (insert): ",
                    RenameMode::Normal => " Rename (normal): ",
                },
            }),
            Some(PendingAction::CreateEntry {
                pane_id: create_pane_id,
                buffer,
                cursor,
                mode,
            }) if *create_pane_id == pane_id => Some(InlineEditorState {
                buffer: buffer.as_str(),
                cursor: *cursor,
                title: create_editor_title(*mode),
            }),
            _ => None,
        };
        let picker_options = match &app.pending_action {
            Some(PendingAction::CopyPicker {
                pane_id: copy_pane_id,
                ..
            }) if *copy_pane_id == pane_id => Some(
                copy_picker_options()
                    .into_iter()
                    .map(|option| format!("{} -> {}", option.shortcut, option.label))
                    .collect::<Vec<_>>(),
            ),
            Some(PendingAction::OpenPicker {
                pane_id: open_pane_id,
                options,
                ..
            }) if *open_pane_id == pane_id => Some(
                options
                    .iter()
                    .map(|option| option.label.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        let picker_state = match &app.pending_action {
            Some(PendingAction::CopyPicker {
                pane_id: copy_pane_id,
                selected,
                ..
            }) if *copy_pane_id == pane_id => {
                picker_options.as_ref().map(|options| InlinePickerState {
                    title: " Copy: ",
                    options,
                    selected: *selected,
                })
            }
            Some(PendingAction::OpenPicker {
                pane_id: open_pane_id,
                selected,
                ..
            }) if *open_pane_id == pane_id => {
                picker_options.as_ref().map(|options| InlinePickerState {
                    title: " Open with: ",
                    options,
                    selected: *selected,
                })
            }
            _ => None,
        };
        let panel_state = if let Some(search) = app.global_search.as_ref() {
            (search.pane_id == pane_id && (search.loading || search.searched)).then_some(
                PaneListState::Search(SearchListState {
                    results: global_search_results.as_deref().unwrap_or(&[]),
                    selected: search.selected,
                    loading: search.loading && app.config.search.show_loading,
                    preview_query: matches!(search.mode, SearchMode::Content)
                        .then_some(search.buffer.as_str()),
                    preview_scroll: search.preview_scroll,
                    preview_current_match: search.preview_current_match,
                }),
            )
        } else if let Some((selected, search, ..)) = trash_overlay_state.as_ref() {
            Some(PaneListState::Trash {
                lines: trash_lines.as_deref().unwrap_or(&[]),
                selected: *selected,
                search: &search.buffer,
                editing: search.editing,
                cursor: app.text_input_cursor,
            })
        } else if let Some(PendingAction::TaskPanel {
            pane_id: action_pane_id,
            selected,
            search,
            ..
        }) = &app.pending_action
        {
            if *action_pane_id == pane_id {
                Some(PaneListState::Tasks {
                    lines: task_lines.as_deref().unwrap_or(&[]),
                    selected: *selected,
                    search: &search.buffer,
                    editing: search.editing,
                    cursor: app.text_input_cursor,
                })
            } else {
                None
            }
        } else if let Some(PendingAction::HelpPanel {
            pane_id: action_pane_id,
            selected,
            search,
            custom_title,
            ..
        }) = &app.pending_action
        {
            if *action_pane_id == pane_id {
                Some(PaneListState::Help {
                    lines: help_lines.as_deref().unwrap_or(&[]),
                    selected: *selected,
                    search: &search.buffer,
                    editing: search.editing,
                    cursor: app.text_input_cursor,
                    custom_title: custom_title.as_deref(),
                })
            } else {
                None
            }
        } else if let Some(PendingAction::ToolPanel {
            pane_id: action_pane_id,
            selected,
        }) = &app.pending_action
        {
            if *action_pane_id == pane_id {
                Some(PaneListState::Tools {
                    statuses: tool_statuses,
                    selected: *selected,
                })
            } else {
                None
            }
        } else if let Some(PendingAction::RegexRename {
            pane_id: action_pane_id,
            selected,
            ..
        }) = &app.pending_action
        {
            if *action_pane_id == pane_id {
                Some(PaneListState::RegexRename {
                    lines: regex_rename_lines.as_deref().unwrap_or(&[]),
                    selected: *selected,
                })
            } else {
                None
            }
        } else {
            None
        };
        let preview_active = pane.is_preview_active();
        let easymotion_labels = match &app.pending_action {
            Some(PendingAction::EasyMotion {
                pane_id: action_pane_id,
                labels,
                ..
            }) if *action_pane_id == pane_id && !labels.is_empty() => Some(labels.as_slice()),
            _ => None,
        };
        let update_badge = if pane_id == app.focused_pane {
            if app.in_app_updating {
                Some(("...", true))
            } else {
                app.update_badge_info
                    .as_ref()
                    .map(|info| (info.latest_version.as_str(), false))
            }
        } else {
            None
        };
        render_pane(
            frame,
            rect,
            pane_id,
            pane,
            pane_id == app.focused_pane,
            preview_active,
            app.visual_selection.as_ref().and_then(|selection| {
                (selection.pane_id == pane_id).then_some((selection.anchor, selection.current))
            }),
            panel_state,
            app.theme,
            &app.config,
            rename_buffer,
            picker_state,
            app.list_find
                .as_ref()
                .filter(|search| search.pane_id == pane_id)
                .map(|search| search.buffer.as_str()),
            app.list_find
                .as_ref()
                .is_some_and(|search| search.pane_id == pane_id),
            app.text_input_cursor,
            &active_job_badges,
            easymotion_labels,
            update_badge,
        )
    } else {
        None
    }
}

/// 收集目前 viewport 中需要查詢背景工作標籤的檔案路徑。
pub(crate) fn visible_job_badge_paths(
    pane: &crate::file_manager::pane::PaneState,
    viewport_height: usize,
) -> Vec<PathBuf> {
    let (start, end) = visible_list_window_range(
        pane.visible_indices.len(),
        pane.selected,
        viewport_height.max(1),
        pane.list_state.offset(),
    );
    pane.visible_indices[start..end]
        .iter()
        .filter_map(|entry_index| pane.entries.get(*entry_index))
        .map(|entry| entry.path.clone())
        .collect()
}
