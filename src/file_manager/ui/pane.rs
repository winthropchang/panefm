use std::path::Path;

use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::{config::AppConfig, file_manager::pane::PaneState, theme::Theme};

use super::{
    entry::render_entry_line,
    input::{render_inline_editor, render_inline_picker, render_top_right_input},
    text::{
        compact_path_for_title, join_title_parts, normalize_title_status_segments,
        task_panel_display_lines, title_separator_width, truncate_text,
        truncate_text_to_display_width,
    },
    types::{InlineEditorState, InlinePickerState, PaneListState, SearchListState},
};

/// 繪製單一 pane 的檔案列表與預覽區。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，這個 pane 在畫面上可使用的矩形範圍。
/// - `pane_id: usize`，目前 pane 的識別值。
/// - `pane: &mut PaneState`，要被渲染的 pane 狀態。
/// - `focused: bool`，這個 pane 是否具有焦點。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `editor_state: Option<InlineEditorState<'_>>`，若目前有 inline 輸入框，這裡會帶入標題、內容、游標與模式。
///
/// 回傳：`Option<(u16, u16)>`。
/// - `Some((x, y))` 代表 rename 輸入游標應顯示的位置。
/// - `None` 代表目前不需要顯示 rename 游標。
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_pane(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    pane_id: usize,
    pane: &mut PaneState,
    focused: bool,
    preview_focused: bool,
    visual_range: Option<(usize, usize)>,
    panel_state: Option<PaneListState<'_>>,
    theme: Theme,
    config: &AppConfig,
    editor_state: Option<InlineEditorState<'_>>,
    picker_state: Option<InlinePickerState<'_>>,
    list_find_buffer: Option<&str>,
    list_find_editing: bool,
    text_input_cursor: usize,
    active_job_badges: &std::collections::HashMap<std::path::PathBuf, String>,
    easymotion_labels: Option<&[(char, usize)]>,
    update_badge: Option<(&str, bool)>,
) -> Option<(u16, u16)> {
    let visual_mode_active = visual_range.is_some();
    let mark_column_active = visual_mode_active || pane.marked_count() > 0;

    let filter_suffix = if pane.has_active_filter() {
        "  [filter]"
    } else {
        ""
    };
    let mark_suffix = if pane.marked_count() > 0 {
        format!("  [mark: {}]", pane.marked_count())
    } else {
        String::new()
    };
    let panel_suffix = match panel_state {
        Some(PaneListState::Search(_)) => "  [search]",
        Some(PaneListState::Tasks { .. }) => "  [tasks]",
        Some(PaneListState::Trash { .. }) => "  [trash d/D u/U]",
        Some(PaneListState::Help {
            custom_title: Some(_),
            ..
        }) => "  [cheatsheet ?]",
        Some(PaneListState::Help { .. }) => "  [help ~/F1]",
        Some(PaneListState::Tools { .. }) => "  [dependencies Esc]",
        Some(PaneListState::RegexRename { .. }) => "  [rename-regex]",
        None => "",
    };

    let is_side_by_side = pane.is_preview_open() && panel_state.is_none() && area.width >= 60;
    let (list_area, preview_area) = if is_side_by_side {
        let list_w = (area.width * 38 / 100).max(22);
        let prev_w = area.width.saturating_sub(list_w);
        (
            Rect {
                x: area.x,
                y: area.y,
                width: list_w,
                height: area.height,
            },
            Some(Rect {
                x: area.x + list_w,
                y: area.y,
                width: prev_w,
                height: area.height,
            }),
        )
    } else {
        (area, None)
    };

    if preview_area.is_none() && (preview_focused || pane.is_preview_focused()) {
        let preview_viewport_height = area.height.saturating_sub(2).max(1) as usize;
        let preview_content_width = area.width.saturating_sub(2).max(1) as usize;
        pane.set_preview_viewport_size(preview_content_width, preview_viewport_height);

        let (preview_title, preview_lines) = match panel_state {
            Some(PaneListState::Search(search_state))
                if !search_state.results.is_empty() && search_state.preview_query.is_some() =>
            {
                let selected = search_state
                    .selected
                    .min(search_state.results.len().saturating_sub(1));
                let entry = &search_state.results[selected];
                let preview = PaneState::search_preview_for_entry(
                    entry,
                    preview_viewport_height,
                    search_state.preview_query.unwrap_or_default(),
                    search_state.preview_scroll,
                    search_state.preview_current_match,
                    true,
                    theme,
                );
                (preview.title, preview.lines)
            }
            _ => {
                let default_preview_title = pane
                    .selected_entry()
                    .map(|entry| pane.preview_title_for_entry(entry))
                    .unwrap_or_else(|| "Preview".to_string());
                (
                    default_preview_title,
                    pane.preview_lines(preview_viewport_height, theme),
                )
            }
        };
        let preview_lines =
            pad_preview_lines_for_render(preview_lines, preview_content_width, theme);
        let preview_border_style = if focused {
            theme.focused_border_style()
        } else {
            theme.muted_style()
        };
        let preview = Paragraph::new(preview_lines).block(
            Block::default()
                .title(preview_title)
                .borders(Borders::ALL)
                .border_style(preview_border_style),
        );
        frame.render_widget(preview, area);
        return None;
    }

    if let Some(prev_rect) = preview_area {
        let preview_viewport_height = prev_rect.height.saturating_sub(2).max(1) as usize;
        let preview_content_width = prev_rect.width.saturating_sub(2).max(1) as usize;
        pane.set_preview_viewport_size(preview_content_width, preview_viewport_height);

        let default_preview_title = pane
            .selected_entry()
            .map(|entry| pane.preview_title_for_entry(entry))
            .unwrap_or_else(|| "Preview".to_string());
        let preview_lines = pane.preview_lines(preview_viewport_height, theme);
        let preview_lines =
            pad_preview_lines_for_render(preview_lines, preview_content_width, theme);
        let preview_border_style = if focused && pane.is_preview_focused() {
            theme.focused_border_style()
        } else {
            theme.muted_style()
        };
        let preview = Paragraph::new(preview_lines).block(
            Block::default()
                .title(default_preview_title)
                .borders(Borders::ALL)
                .border_style(preview_border_style),
        );
        frame.render_widget(preview, prev_rect);
    }

    // 在清單模式下，若 Pane 處於焦點且非特殊面板，背景預熱當前選取項目與相鄰項目
    if focused && panel_state.is_none() {
        pane.prefetch_current_and_adjacent_previews(true);
    }

    let list_focused = focused && !pane.is_preview_focused();
    let vcs_suffix = if config.ui.vcs.enabled {
        pane.vcs_header_label()
            .map(|l| format!("[{l}]"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let (badge, path_text, suffix) = format_pane_title_parts(
        pane_id,
        pane.cwd.as_path(),
        filter_suffix,
        &mark_suffix,
        panel_suffix,
        &vcs_suffix,
        &pane.title_mode_label(),
        list_area.width.saturating_sub(3) as usize,
    );
    let title = render_pane_title_line(&badge, &path_text, &suffix, list_focused, theme);
    let list_border_style = if list_focused {
        theme.focused_border_style()
    } else {
        theme.muted_style()
    };
    let mut block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(list_border_style);

    if let Some((latest_ver, is_updating)) = update_badge
        && focused
        && let Some(badge_line) = render_update_badge(latest_ver, is_updating, list_area.width)
    {
        block = block.title(badge_line.alignment(Alignment::Right));
    }

    let content_width = list_area.width.saturating_sub(4) as usize;
    let list_viewport_height = list_area.height.saturating_sub(2).max(1) as usize;
    pane.set_list_viewport_height(list_viewport_height);

    let mut normal_list_window_start = None;
    let items: Vec<ListItem<'static>> = if let Some(panel_state) = panel_state {
        match panel_state {
            PaneListState::Search(search_state) => {
                if let Some(message) = search_empty_message(&search_state) {
                    vec![ListItem::new(Line::from(message))]
                } else {
                    search_state
                        .results
                        .iter()
                        .map(|entry| ListItem::new(Line::from(entry.relative_path.clone())))
                        .collect()
                }
            }
            PaneListState::Tasks { lines: [], .. } => {
                vec![ListItem::new(Line::from("No tasks yet"))]
            }
            PaneListState::Tasks { lines, .. } => lines
                .iter()
                .map(|line| ListItem::new(task_panel_display_lines(line, content_width)))
                .collect(),
            PaneListState::Trash { lines: [], .. } => {
                vec![ListItem::new(Line::from("Trash is empty"))]
            }
            PaneListState::Trash { lines, .. } => lines
                .iter()
                .map(|line| {
                    let name_w = if content_width < 60 { 14 } else { 20 };
                    let date_w = if content_width < 60 { 12 } else { 16 };
                    let name_str = truncate_text(&line.name, name_w);
                    let date_str = truncate_text(&line.deleted_at, date_w);
                    let mark = if line.marked { "*" } else { " " };
                    let prefix = format!("{} {:<name_w$}  {:<date_w$}  ", mark, name_str, date_str);
                    let prefix_w = UnicodeWidthStr::width(prefix.as_str());
                    let path_max_w = content_width.saturating_sub(prefix_w);
                    let path_str = truncate_text_to_display_width(&line.original_path, path_max_w);
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("{} ", mark)),
                        Span::styled(format!("{:<name_w$}  ", name_str), theme.accent_style()),
                        Span::styled(format!("{:<date_w$}  ", date_str), theme.muted_style()),
                        Span::raw(path_str),
                    ]))
                })
                .collect(),
            PaneListState::Help { lines: [], .. } => {
                vec![ListItem::new(Line::from("沒有符合搜尋條件的功能"))]
            }
            PaneListState::Help { lines, .. } => {
                let cmd_w = if content_width < 50 {
                    11
                } else if content_width < 80 {
                    15
                } else {
                    18
                };
                let shortcut_w = if content_width < 50 {
                    6
                } else if content_width < 80 {
                    10
                } else {
                    14
                };
                lines
                    .iter()
                    .map(|line| {
                        let cmd_str = truncate_text(&line.command, cmd_w);
                        let shortcut_str = truncate_text(&line.shortcut, shortcut_w);
                        let prefix =
                            format!("{:<cmd_w$}  {:<shortcut_w$}  ", cmd_str, shortcut_str);
                        let prefix_w = UnicodeWidthStr::width(prefix.as_str());
                        let desc_max_w = content_width.saturating_sub(prefix_w);
                        let desc_str =
                            truncate_text_to_display_width(&line.description, desc_max_w);
                        ListItem::new(Line::from(vec![
                            Span::styled(
                                format!("{:<cmd_w$}  ", cmd_str),
                                theme.accent_style().add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("{:<shortcut_w$}  ", shortcut_str),
                                theme.muted_style(),
                            ),
                            Span::raw(desc_str),
                        ]))
                    })
                    .collect()
            }
            PaneListState::Tools { statuses, .. } => statuses
                .iter()
                .map(|tool| {
                    let state = if tool.installed {
                        "已安裝"
                    } else {
                        "未安裝"
                    };
                    ListItem::new(Line::from(format!("{:<10} {state}", tool.name)))
                })
                .collect(),
            PaneListState::RegexRename { lines: [], .. } => {
                vec![ListItem::new(Line::from("沒有可預覽的改名項目"))]
            }
            PaneListState::RegexRename { lines, .. } => lines
                .iter()
                .map(|line| {
                    ListItem::new(Line::from(vec![
                        Span::raw(format!(
                            "{:<22} -> {:<22}  ",
                            truncate_text(&line.original_name, 22),
                            truncate_text(&line.new_name, 22)
                        )),
                        Span::styled(
                            line.status.clone(),
                            regex_rename_status_style(theme, &line.status),
                        ),
                    ]))
                })
                .collect(),
        }
    } else {
        let visible_len = pane.visible_indices.len();
        let detail_kind = pane.active_detail_kind();
        let find_match_position = pane.list_find_match_position();
        if visible_len == 0 {
            vec![ListItem::new(Line::from("empty directory"))]
        } else {
            let (view_start, view_end) = visible_list_window_range(
                visible_len,
                pane.selected,
                list_viewport_height,
                pane.list_state.offset(),
            );
            normal_list_window_start = Some(view_start);
            pane.visible_indices[view_start..view_end]
                .iter()
                .filter_map(|entry_index| pane.entries.get(*entry_index))
                .enumerate()
                .map(|(index, entry)| {
                    let visible_index = view_start + index;
                    let active_job_badge = active_job_badges.get(&entry.path).map(|s| s.as_str());
                    let jump_label = easymotion_labels.and_then(|labels| {
                        labels
                            .iter()
                            .find(|(_, idx)| *idx == visible_index)
                            .map(|(ch, _)| *ch)
                    });
                    let vcs_status = if config.ui.vcs.enabled {
                        pane.vcs_status_for_path(&entry.path)
                    } else {
                        None
                    };
                    ListItem::new(render_entry_line(
                        entry,
                        pane.is_marked(entry),
                        mark_column_active,
                        visual_range
                            .map(|(start, end)| {
                                let range_start = start.min(end);
                                let range_end = start.max(end);
                                visible_index >= range_start && visible_index <= range_end
                            })
                            .unwrap_or(false),
                        detail_kind,
                        content_width,
                        theme,
                        config.ui.icons.enabled,
                        config.ui.icons.style,
                        pane.list_find_query(),
                        find_match_position.filter(|_| visible_index == pane.selected),
                        active_job_badge,
                        jump_label,
                        vcs_status,
                    ))
                })
                .collect()
        }
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    if let Some(panel_state) = panel_state {
        let mut list_state = ListState::default();
        match panel_state {
            PaneListState::Search(search_state) if !search_state.results.is_empty() => {
                list_state.select(search_list_selected_index(&search_state));
            }
            PaneListState::Trash {
                lines, selected, ..
            } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            PaneListState::Tasks {
                lines, selected, ..
            } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            PaneListState::Help {
                lines, selected, ..
            } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            PaneListState::Tools { statuses, selected } if !statuses.is_empty() => {
                list_state.select(Some(selected.min(statuses.len().saturating_sub(1))));
            }
            PaneListState::RegexRename { lines, selected } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            _ => {}
        }
        frame.render_stateful_widget(list, list_area, &mut list_state);
    } else if let Some(window_start) = normal_list_window_start {
        let mut viewport_state = ListState::default();
        viewport_state.select(Some(pane.selected.saturating_sub(window_start)));
        frame.render_stateful_widget(list, list_area, &mut viewport_state);
        pane.list_state.select(Some(pane.selected));
        *pane.list_state.offset_mut() = window_start;
    } else {
        frame.render_stateful_widget(list, list_area, &mut pane.list_state);
    }

    let mut editor_cursor = None;
    if let Some(state) = editor_state {
        editor_cursor = render_inline_editor(frame, list_area, pane, theme, state);
    }
    if let Some(state) = picker_state {
        render_inline_picker(frame, list_area, pane, theme, state);
    }

    let panel_cursor = match panel_state {
        Some(PaneListState::Trash {
            search,
            editing: true,
            cursor,
            ..
        }) => Some(render_top_right_input(
            frame,
            list_area,
            theme,
            "Trash Search",
            search,
            cursor,
        )),
        Some(PaneListState::Help {
            search,
            editing: true,
            cursor,
            custom_title,
            ..
        }) => Some(render_top_right_input(
            frame,
            list_area,
            theme,
            if custom_title.is_some() {
                "Cheatsheet Search"
            } else {
                "Help Search"
            },
            search,
            cursor,
        )),
        Some(PaneListState::Tasks {
            search,
            editing: true,
            cursor,
            ..
        }) => Some(render_top_right_input(
            frame,
            list_area,
            theme,
            "Task Search",
            search,
            cursor,
        )),
        _ if list_find_editing => Some(render_top_right_input(
            frame,
            list_area,
            theme,
            "Find next",
            list_find_buffer.unwrap_or_default(),
            text_input_cursor,
        )),
        _ => None,
    };

    if focused && pane.is_preview_focused() {
        None
    } else {
        editor_cursor.or(panel_cursor)
    }
}

/// 回傳搜尋列表在尚未收到任何結果時應顯示的提示文字。
pub(crate) fn search_empty_message(state: &SearchListState<'_>) -> Option<&'static str> {
    if !state.results.is_empty() {
        None
    } else if state.loading {
        Some("Loading search results...")
    } else {
        Some("No matches")
    }
}

/// 計算 global search 列表目前應該反白的項目索引。
pub(crate) fn search_list_selected_index(search_state: &SearchListState<'_>) -> Option<usize> {
    (!search_state.results.is_empty()).then(|| {
        search_state
            .selected
            .min(search_state.results.len().saturating_sub(1))
    })
}

/// 計算大型一般列表本幀真正需要建立 widget 的 viewport 範圍。
pub(crate) fn visible_list_window_range(
    total: usize,
    selected: usize,
    viewport_height: usize,
    previous_start: usize,
) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }

    let height = viewport_height.max(1).min(total);
    let selected = selected.min(total - 1);
    let max_start = total.saturating_sub(height);
    let mut start = previous_start.min(max_start);
    if selected < start {
        start = selected;
    } else if selected >= start + height {
        start = selected + 1 - height;
    }
    let end = (start + height).min(total);
    (start, end)
}

/// 根據 regex 批次改名預覽狀態套用主題語意色。
pub(crate) fn regex_rename_status_style(theme: Theme, status: &str) -> Style {
    match status {
        "ready" => theme.success_style(),
        "unchanged" => theme.muted_style(),
        "conflict" | "invalid" => theme.danger_style(),
        _ => Style::default(),
    }
}

/// 組合 pane 標題列文字三元素：(膠囊徽章字串, 壓縮或完整路徑, 狀態後綴)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn format_pane_title_parts(
    pane_id: usize,
    cwd: &Path,
    filter_suffix: &str,
    mark_suffix: &str,
    panel_suffix: &str,
    vcs_suffix: &str,
    mode_label: &str,
    max_width: usize,
) -> (String, String, String) {
    let prefix = format!(" {pane_id} ");
    let full_path = cwd.display().to_string();
    let status_suffix =
        normalize_title_status_segments(&[filter_suffix, mark_suffix, panel_suffix, vcs_suffix]);
    let suffix_candidates = [
        if status_suffix.is_empty() {
            format!("[{mode_label}]")
        } else {
            format!("{status_suffix} [{mode_label}]")
        },
        if status_suffix.is_empty() {
            format!("[{mode_label}]")
        } else {
            format!("{status_suffix} [{mode_label}]")
        },
        status_suffix.clone(),
        String::new(),
    ];

    for suffix in suffix_candidates {
        let full_title = join_title_parts(&prefix, &full_path, &suffix);
        if full_title.chars().count() <= max_width {
            return (prefix, full_path, suffix);
        }

        let separator_width = title_separator_width(true, !suffix.is_empty());
        let fixed_width = prefix.chars().count() + suffix.chars().count() + separator_width;
        let path_width = max_width.saturating_sub(fixed_width).max(1);
        let compact_path = compact_path_for_title(&full_path, path_width);
        let compact_title = join_title_parts(&prefix, &compact_path, &suffix);
        if compact_title.chars().count() <= max_width {
            return (prefix, compact_path, suffix);
        }
    }

    let fallback_path_width = max_width
        .saturating_sub(prefix.chars().count())
        .saturating_sub(1)
        .max(1);
    (
        prefix,
        compact_path_for_title(&full_path, fallback_path_width),
        String::new(),
    )
}

/// 組合 pane 標題列文字，讓 pane 編號以膠囊標記固定顯示在最前面，方便搭配數字切換。
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn format_pane_title(
    pane_id: usize,
    cwd: &Path,
    filter_suffix: &str,
    mark_suffix: &str,
    panel_suffix: &str,
    vcs_suffix: &str,
    mode_label: &str,
    max_width: usize,
) -> String {
    let (prefix, path, suffix) = format_pane_title_parts(
        pane_id,
        cwd,
        filter_suffix,
        mark_suffix,
        panel_suffix,
        vcs_suffix,
        mode_label,
        max_width,
    );
    join_title_parts(&prefix, &path, &suffix)
}

/// 建立帶有獨立樣式之 Pane 標題列 Line 物件（支援方案 A 實心膠囊徽章與路徑分段上色）。
pub(crate) fn render_pane_title_line(
    badge: &str,
    path: &str,
    suffix: &str,
    focused: bool,
    theme: Theme,
) -> Line<'static> {
    let badge_style = theme.pane_badge_style(focused);
    let path_style = if focused {
        theme.focused_border_style()
    } else {
        theme.muted_style()
    };
    let suffix_style = theme.muted_style();

    let mut spans = vec![Span::styled(badge.to_string(), badge_style)];
    if !path.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(path.to_string(), path_style));
    }
    if !suffix.is_empty() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(suffix.to_string(), suffix_style));
    }

    Line::from(spans)
}

/// 建立頂部更新提示膠囊徽章（高對比黃底紅字）。
pub(crate) fn render_update_badge(
    latest_version: &str,
    is_updating: bool,
    available_width: u16,
) -> Option<Line<'static>> {
    let badge_style = Style::default()
        .fg(ratatui::style::Color::Rgb(180, 0, 0))
        .bg(ratatui::style::Color::Rgb(255, 220, 0))
        .add_modifier(Modifier::BOLD);

    if is_updating {
        if available_width >= 20 {
            return Some(Line::from(vec![Span::styled(
                " [ ⏳ 升級中... ] ".to_string(),
                badge_style,
            )]));
        }
        return None;
    }

    if available_width >= 55 {
        Some(Line::from(vec![Span::styled(
            format!(" [ 🚀 新版 v{latest_version} 可用！按 :update 升級 ] "),
            badge_style,
        )]))
    } else if available_width >= 35 {
        Some(Line::from(vec![Span::styled(
            format!(" [ 🚀 v{latest_version} :update ] "),
            badge_style,
        )]))
    } else if available_width >= 20 {
        Some(Line::from(vec![Span::styled(
            format!(" [ 🚀 v{latest_version} ] "),
            badge_style,
        )]))
    } else {
        None
    }
}

/// 將 preview 行內容依照可見寬度補齊，讓目前命中列的背景可以延伸到整行右側。
pub(crate) fn pad_preview_lines_for_render(
    mut lines: Vec<Line<'static>>,
    content_width: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    for line in &mut lines {
        let is_current_line = line
            .spans
            .iter()
            .any(|span| span.style.bg == Some(theme.preview_current_line_bg));
        if !is_current_line {
            continue;
        }

        let current_width = line.to_string().chars().count();
        if current_width >= content_width {
            continue;
        }

        let padding = " ".repeat(content_width - current_width);
        line.spans
            .push(Span::styled(padding, theme.preview_current_line_style()));
    }
    lines
}
