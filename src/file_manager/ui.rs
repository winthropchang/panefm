use chrono::{DateTime, Local};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::{
    config::AppConfig,
    theme::{Theme, ThemePreset},
};

use super::{
    pane::{PaneState, SortDetailKind},
    search::GlobalSearchEntry,
};

/// 描述 inline 編輯器目前需要顯示的內容、標題與游標位置。
///
/// 這個結構只負責把 `App` 的輸入狀態轉交給 UI，
/// 讓繪圖函數可以知道目前文字內容、游標在哪裡、處於哪一種模式，
/// 還有應該顯示哪一種標題。
#[derive(Clone, Copy)]
pub(crate) struct InlineEditorState<'a> {
    pub(crate) buffer: &'a str,
    pub(crate) cursor: usize,
    pub(crate) title: &'a str,
}

/// 描述 inline 選單目前需要顯示的標題、選項與游標位置。
#[derive(Clone, Copy)]
pub(crate) struct InlinePickerState<'a> {
    pub(crate) title: &'a str,
    pub(crate) options: &'a [String],
    pub(crate) selected: usize,
}

/// 描述目前 pane 是否要把主列表暫時切換成 global search 的結果畫面。
#[derive(Clone, Copy)]
pub(crate) struct SearchListState<'a> {
    pub(crate) results: &'a [GlobalSearchEntry],
    pub(crate) selected: usize,
    pub(crate) loading: bool,
}

/// 描述目前 pane 的列表區是否被某種特殊模式接管。
#[derive(Clone, Copy)]
pub(crate) enum PaneListState<'a> {
    Search(SearchListState<'a>),
    Trash {
        lines: &'a [TrashPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
    },
    Help {
        lines: &'a [HelpPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
    },
    RegexRename {
        lines: &'a [RegexRenamePanelLine],
        selected: usize,
    },
}

/// 描述 trash 面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrashPanelLine {
    pub(crate) name: String,
    pub(crate) original_path: String,
    pub(crate) deleted_at: String,
    pub(crate) marked: bool,
}

/// 描述說明面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HelpPanelLine {
    pub(crate) command: String,
    pub(crate) shortcut: String,
    pub(crate) description: String,
}

/// 描述書籤列表彈窗中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BookmarkPanelLine {
    pub(crate) key: String,
    pub(crate) path: String,
}

/// 描述 regex 批次改名預覽面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegexRenamePanelLine {
    pub(crate) original_name: String,
    pub(crate) new_name: String,
    pub(crate) status: String,
}

/// 描述 command palette 中單一條命令補全候選。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandSuggestionLine {
    pub(crate) command: String,
    pub(crate) display_command: String,
    pub(crate) description: String,
}

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
) -> Option<(u16, u16)> {
    let visual_mode_active = visual_range.is_some();
    let mark_column_active = visual_mode_active || pane.marked_count() > 0;
    let preview_height = config
        .ui
        .preview
        .height
        .min(area.height.saturating_sub(4))
        .max(4);
    let list_height = config
        .ui
        .preview
        .focus_list_height
        .min(area.height.saturating_sub(4))
        .max(3);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if preview_focused {
            [Constraint::Length(list_height), Constraint::Min(6)]
        } else {
            [Constraint::Min(3), Constraint::Length(preview_height)]
        })
        .split(area);

    let border_style = if focused {
        theme.focused_border_style()
    } else {
        theme.muted_style()
    };

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
        Some(PaneListState::Trash { .. }) => "  [trash]",
        Some(PaneListState::Help { .. }) => "  [help]",
        Some(PaneListState::RegexRename { .. }) => "  [rename-regex]",
        None => "",
    };
    let title = format!(
        " pane {}  {}{}{}{}  [sort: {}]",
        pane_id,
        pane.cwd.display(),
        filter_suffix,
        mark_suffix,
        panel_suffix,
        pane.sort_mode.label()
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let content_width = chunks[0].width.saturating_sub(4) as usize;
    let items: Vec<ListItem<'static>> = if let Some(panel_state) = panel_state {
        match panel_state {
            PaneListState::Search(search_state) if search_state.loading => {
                vec![ListItem::new(Line::from("Loading search results..."))]
            }
            PaneListState::Search(search_state) if search_state.results.is_empty() => {
                vec![ListItem::new(Line::from("No matches"))]
            }
            PaneListState::Search(search_state) => search_state
                .results
                .iter()
                .map(|entry| ListItem::new(Line::from(entry.relative_path.clone())))
                .collect(),
            PaneListState::Trash { lines, .. } if lines.is_empty() => {
                vec![ListItem::new(Line::from("Trash is empty"))]
            }
            PaneListState::Trash { lines, .. } => lines
                .iter()
                .map(|line| {
                    ListItem::new(Line::from(format!(
                        "{} {:<20}  {:<16}  {}",
                        if line.marked { "*" } else { " " },
                        truncate_text(&line.name, 19),
                        line.deleted_at,
                        line.original_path
                    )))
                })
                .collect(),
            PaneListState::Help { lines, .. } if lines.is_empty() => {
                vec![ListItem::new(Line::from("沒有符合搜尋條件的功能"))]
            }
            PaneListState::Help { lines, .. } => lines
                .iter()
                .map(|line| {
                    ListItem::new(Line::from(format!(
                        "{:<18}  {:<16}  {}",
                        truncate_text(&line.command, 18),
                        truncate_text(&line.shortcut, 16),
                        line.description
                    )))
                })
                .collect(),
            PaneListState::RegexRename { lines, .. } if lines.is_empty() => {
                vec![ListItem::new(Line::from("沒有可預覽的改名項目"))]
            }
            PaneListState::RegexRename { lines, .. } => lines
                .iter()
                .map(|line| {
                    ListItem::new(Line::from(format!(
                        "{:<22} -> {:<22}  {}",
                        truncate_text(&line.original_name, 22),
                        truncate_text(&line.new_name, 22),
                        line.status
                    )))
                })
                .collect(),
        }
    } else {
        let visible_entries = pane.visible_entries();
        let detail_kind = pane.sort_mode.detail_kind();
        if visible_entries.is_empty() {
            vec![ListItem::new(Line::from("empty directory"))]
        } else {
            visible_entries
                .into_iter()
                .enumerate()
                .map(|entry| {
                    ListItem::new(render_entry_line(
                        entry.1,
                        pane.is_marked(entry.1),
                        mark_column_active,
                        visual_range
                            .map(|(start, end)| {
                                let range_start = start.min(end);
                                let range_end = start.max(end);
                                entry.0 >= range_start && entry.0 <= range_end
                            })
                            .unwrap_or(false),
                        detail_kind,
                        content_width,
                        theme,
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
            PaneListState::Search(search_state)
                if !search_state.loading && !search_state.results.is_empty() =>
            {
                list_state.select(Some(
                    search_state
                        .selected
                        .min(search_state.results.len().saturating_sub(1)),
                ));
            }
            PaneListState::Trash {
                lines, selected, ..
            } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            PaneListState::Help {
                lines, selected, ..
            } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            PaneListState::RegexRename { lines, selected } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            _ => {}
        }
        frame.render_stateful_widget(list, chunks[0], &mut list_state);
    } else {
        frame.render_stateful_widget(list, chunks[0], &mut pane.list_state);
    }

    let mut editor_cursor = None;
    if let Some(state) = editor_state {
        editor_cursor = render_inline_editor(frame, chunks[0], pane, theme, state);
    }
    if let Some(state) = picker_state {
        render_inline_picker(frame, chunks[0], pane, theme, state);
    }

    let panel_cursor = match panel_state {
        Some(PaneListState::Trash {
            search,
            editing: true,
            ..
        }) => Some(render_top_right_input(
            frame,
            chunks[0],
            theme,
            "Trash Search",
            search,
        )),
        Some(PaneListState::Help {
            search,
            editing: true,
            ..
        }) => Some(render_top_right_input(
            frame,
            chunks[0],
            theme,
            "Help Search",
            search,
        )),
        _ => None,
    };

    let preview_title = pane
        .selected_entry()
        .map(|entry| {
            let mut title = format!("Preview: {}", entry.name);
            if preview_focused {
                title.push_str("  [preview]");
            }
            if let Some(query) = pane.preview_search_query() {
                title.push_str(&format!("  [/{}]", query));
            }
            if pane.has_preview_scroll() {
                title.push_str("  ^");
            }
            if pane.preview_has_more_below() {
                title.push_str("  v");
            }
            title
        })
        .unwrap_or_else(|| "Preview".to_string());
    let preview_viewport_height = chunks[1].height.saturating_sub(2).max(1) as usize;
    pane.set_preview_viewport_height(preview_viewport_height);
    let preview = Paragraph::new(pane.preview_lines(preview_viewport_height)).block(
        Block::default()
            .title(preview_title)
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(border_style),
    );
    frame.render_widget(preview, chunks[1]);

    editor_cursor.or(panel_cursor)
}

/// 在列表區域中繪製 inline 輸入視窗，供 rename / create 這類功能重用。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `list_area: Rect`，檔案列表所在的畫面區域。
/// - `pane: &PaneState`，目前 pane 狀態。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `state: InlineEditorState<'_>`，目前正在編輯的標題、內容、游標與模式。
///
/// 回傳：`Option<(u16, u16)>`。
/// - `Some((x, y))` 代表輸入游標應該出現的位置。
/// - `None` 代表目前沒有足夠空間繪製 rename 區塊。
fn render_inline_editor(
    frame: &mut ratatui::Frame<'_>,
    list_area: Rect,
    pane: &PaneState,
    theme: Theme,
    state: InlineEditorState<'_>,
) -> Option<(u16, u16)> {
    let inner = Block::default().borders(Borders::ALL).inner(list_area);
    let selected_row = if pane.entries.is_empty() {
        inner.y
    } else {
        inner.y.saturating_add(pane.selected as u16)
    };
    let box_y = selected_row.saturating_add(1);

    if box_y.saturating_add(2) >= inner.y.saturating_add(inner.height) {
        return None;
    }

    let input_area = Rect {
        x: inner.x,
        y: box_y,
        width: inner.width.saturating_sub(1),
        height: 3,
    };

    frame.render_widget(Clear, input_area);
    let input_block = Block::default()
        .title(Line::from(Span::styled(
            state.title,
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let input_inner = input_block.inner(input_area);
    frame.render_widget(
        Paragraph::new(state.buffer.to_string()).block(input_block),
        input_area,
    );

    Some((
        input_inner
            .x
            .saturating_add(state.cursor.min(state.buffer.chars().count()) as u16),
        input_inner.y,
    ))
}

/// 在列表區域中繪製 inline 小型選單，供 `Open with` 這類操作重用。
fn render_inline_picker(
    frame: &mut ratatui::Frame<'_>,
    list_area: Rect,
    pane: &PaneState,
    theme: Theme,
    state: InlinePickerState<'_>,
) {
    let inner = Block::default().borders(Borders::ALL).inner(list_area);
    let selected_row = if pane.entries.is_empty() {
        inner.y
    } else {
        inner.y.saturating_add(pane.selected as u16)
    };
    let box_y = selected_row.saturating_add(1);
    let height = state.options.len().min(6) as u16 + 2;

    if box_y.saturating_add(height) >= inner.y.saturating_add(inner.height) {
        return;
    }

    let picker_area = Rect {
        x: inner.x,
        y: box_y,
        width: inner.width.saturating_sub(1),
        height,
    };

    frame.render_widget(Clear, picker_area);
    let picker_block = Block::default()
        .title(Line::from(Span::styled(
            state.title,
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let picker_inner = picker_block.inner(picker_area);
    frame.render_widget(picker_block, picker_area);

    let items = state
        .options
        .iter()
        .map(|option| ListItem::new(Line::from(option.clone())))
        .collect::<Vec<_>>();
    let mut list_state = ListState::default();
    if !state.options.is_empty() {
        list_state.select(Some(
            state.selected.min(state.options.len().saturating_sub(1)),
        ));
    }

    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(theme.selected_item_style())
            .highlight_symbol("▶ "),
        picker_inner,
        &mut list_state,
    );
}

/// 在指定區域中計算一個置中的 popup 矩形。
///
/// 參數：
/// - `area: Rect`，整體可用畫面範圍。
/// - `width_percent: u16`，popup 寬度占整體寬度的百分比。
/// - `height: u16`，popup 的固定高度列數。
///
/// 回傳：`Rect`，可直接拿來繪製 popup 的置中區域。
pub(crate) fn centered_rect(area: Rect, width_percent: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(height),
            Constraint::Min(1),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

/// 在畫面右上方繪製小型輸入框，供 filter 與 preview search 這類短文字輸入重用。
fn render_top_right_input(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    title: &str,
    buffer: &str,
) -> (u16, u16) {
    let width = area.width.min(32).max(18);
    let input_area = Rect {
        x: area.x + area.width.saturating_sub(width + 1),
        y: area.y + 1,
        width,
        height: 3,
    };

    frame.render_widget(Clear, input_area);
    let input_block = Block::default()
        .title(Line::from(Span::styled(
            title,
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let input_inner = input_block.inner(input_area);
    frame.render_widget(
        Paragraph::new(buffer.to_string()).block(input_block),
        input_area,
    );

    (
        input_inner.x.saturating_add(buffer.chars().count() as u16),
        input_inner.y,
    )
}

/// 在畫面右上方繪製 filter 輸入框，並回傳游標應該停留的位置。
pub(crate) fn render_filter_input(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    buffer: &str,
) -> (u16, u16) {
    render_top_right_input(frame, area, theme, " Filter ", buffer)
}

/// 在畫面右上方繪製 preview search 輸入框，並回傳游標應該停留的位置。
pub(crate) fn render_preview_search_input(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    buffer: &str,
) -> (u16, u16) {
    render_top_right_input(frame, area, theme, " Preview Search ", buffer)
}

/// 在目前 pane 上方疊出 global search 輸入框，只顯示查詢文字。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前畫面物件。
/// - `area: Rect`，目前 pane 的可用區域。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `buffer: &str`，搜尋框中的查詢文字。
/// - `editing: bool`，是否仍處於輸入模式。
///
/// 回傳：`(u16, u16)`，global search 輸入游標應停留的位置。
pub(crate) fn render_global_search_panel(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    buffer: &str,
    editing: bool,
) -> (u16, u16) {
    let width = area.width.min(40).max(24);
    let panel_area = Rect {
        x: area.x + area.width.saturating_sub(width + 1),
        y: area.y + 1,
        width,
        height: 3,
    };

    frame.render_widget(Clear, panel_area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            if editing {
                " Global Search (insert) "
            } else {
                " Global Search (normal) "
            },
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let input_inner = block.inner(panel_area);
    frame.render_widget(Paragraph::new(buffer.to_string()).block(block), panel_area);

    (
        input_inner
            .x
            .saturating_add(buffer.chars().count().min(input_inner.width as usize) as u16),
        input_inner.y,
    )
}

/// 在畫面底部繪製排序選單，模仿 mature-reference 的快捷鍵提示面板。
pub(crate) fn render_sort_picker(frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
    let panel_height = 7;
    let panel_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(panel_height),
        width: area.width,
        height: panel_height,
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(",m", theme.accent_style()),
            Span::raw(" -> modified  "),
            Span::styled(",M", theme.accent_style()),
            Span::raw(" -> modified (reverse)  "),
            Span::styled(",b", theme.accent_style()),
            Span::raw(" -> birth  "),
            Span::styled(",B", theme.accent_style()),
            Span::raw(" -> birth (reverse)"),
        ]),
        Line::from(vec![
            Span::styled(",a", theme.accent_style()),
            Span::raw(" -> alphabetical  "),
            Span::styled(",A", theme.accent_style()),
            Span::raw(" -> alphabetical (reverse)  "),
            Span::styled(",n", theme.accent_style()),
            Span::raw(" -> natural  "),
            Span::styled(",N", theme.accent_style()),
            Span::raw(" -> natural (reverse)"),
        ]),
        Line::from(vec![
            Span::styled(",e", theme.accent_style()),
            Span::raw(" -> extension  "),
            Span::styled(",E", theme.accent_style()),
            Span::raw(" -> extension (reverse)  "),
            Span::styled(",s", theme.accent_style()),
            Span::raw(" -> size  "),
            Span::styled(",S", theme.accent_style()),
            Span::raw(" -> size (reverse)"),
        ]),
        Line::from(vec![
            Span::styled(",r", theme.accent_style()),
            Span::raw(" -> random  "),
            Span::styled("Esc", theme.accent_style()),
            Span::raw(" -> cancel"),
        ]),
    ];

    frame.render_widget(Clear, panel_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Sort ",
                    theme.accent_style().add_modifier(Modifier::BOLD),
                )))
                .borders(Borders::TOP),
        ),
        panel_area,
    );
}

/// 在畫面中央繪製書籤列表彈窗，供 `:bookmark list` 使用。
pub(crate) fn render_bookmark_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    lines: &[BookmarkPanelLine],
    selected: usize,
) {
    let popup_height = (lines.len().min(10) as u16).saturating_add(4).max(6);
    let popup_area = centered_rect(area, 68, popup_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Bookmarks ",
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let items = if lines.is_empty() {
        vec![ListItem::new(Line::from("沒有書籤，按 m{key} 新增"))]
    } else {
        lines
            .iter()
            .map(|line| {
                ListItem::new(Line::from(format!(
                    "{:<4} {}",
                    truncate_text(&line.key, 4),
                    line.path
                )))
            })
            .collect::<Vec<_>>()
    };

    let mut list_state = ListState::default();
    if !lines.is_empty() {
        list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
    }

    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(theme.selected_item_style())
            .highlight_symbol("▶ "),
        inner,
        &mut list_state,
    );
}

/// 在指定 pane 區域中央繪製命令輸入視窗，並回傳游標位置。
pub(crate) fn render_command_palette(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    buffer: &str,
    suggestions: &[CommandSuggestionLine],
    selected: usize,
) -> (u16, u16) {
    let popup_height = (suggestions.len().min(6) as u16).saturating_add(3).max(3);
    let popup_area = centered_rect(area, 70, popup_height);
    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Command ",
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);
    frame.render_widget(Paragraph::new(format!(":{}", buffer)), chunks[0]);

    if !suggestions.is_empty() && chunks.len() > 1 {
        let items = suggestions
            .iter()
            .map(|line| {
                ListItem::new(Line::from(format!(
                    "{:<22}  {}",
                    truncate_text(&line.display_command, 22),
                    line.description
                )))
            })
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        list_state.select(Some(selected.min(suggestions.len().saturating_sub(1))));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(theme.selected_item_style())
                .highlight_symbol("▶ "),
            chunks[1],
            &mut list_state,
        );
    }

    (
        inner
            .x
            .saturating_add(
                buffer
                    .chars()
                    .count()
                    .min(inner.width.saturating_sub(1) as usize) as u16,
            )
            .saturating_add(1),
        inner.y,
    )
}

/// 將過長文字裁切成指定寬度，避免面板欄位爆掉。
fn truncate_text(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars
        .into_iter()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        + "…"
}

/// 根據目前排序模式，產生單一列表列的顯示內容。
fn render_entry_line(
    entry: &super::entry::FileEntry,
    marked: bool,
    mark_column_active: bool,
    visual_selected: bool,
    detail_kind: SortDetailKind,
    width: usize,
    theme: Theme,
) -> Line<'static> {
    let marker = if mark_column_active {
        if marked || visual_selected {
            "[*] "
        } else {
            "    "
        }
    } else {
        ""
    };
    let name = format!("{marker}{}", entry.display_name());
    let detail = format_sort_detail(entry, detail_kind);
    if detail.is_empty() || width < 8 {
        return Line::from(name);
    }

    let name_len = name.chars().count();
    let detail_len = detail.chars().count();
    let spacer_len = width.saturating_sub(name_len + detail_len).max(1);

    Line::from(vec![
        Span::raw(name),
        Span::raw(" ".repeat(spacer_len)),
        Span::styled(detail, theme.muted_style()),
    ])
}

/// 依照目前排序依據，決定右側欄位要顯示的文字。
fn format_sort_detail(entry: &super::entry::FileEntry, detail_kind: SortDetailKind) -> String {
    match detail_kind {
        SortDetailKind::None => String::new(),
        SortDetailKind::Size => {
            if entry.is_dir {
                String::from("dir")
            } else {
                format_size_short(entry.size)
            }
        }
        SortDetailKind::Modified => format_system_time(entry.modified),
        SortDetailKind::Created => format_system_time(entry.created),
        SortDetailKind::Extension => {
            if entry.is_dir {
                String::from("dir")
            } else {
                entry
                    .path
                    .extension()
                    .map(|value| value.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }
        }
    }
}

/// 把 `SystemTime` 轉成比較容易閱讀的本地時間字串。
fn format_system_time(value: std::time::SystemTime) -> String {
    let datetime: DateTime<Local> = value.into();
    datetime.format("%m/%d %H:%M").to_string()
}

/// 把位元組大小轉成較容易閱讀的短格式。
fn format_size_short(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let size = size as f64;
    if size >= GB {
        format_compact_size(size / GB, "G")
    } else if size >= MB {
        format_compact_size(size / MB, "mb")
    } else if size >= KB {
        format_compact_size(size / KB, "kb")
    } else {
        format!("{}b", size as u64)
    }
}

/// 將大小數值格式化成最多一位小數的緊湊字串。
fn format_compact_size(value: f64, suffix: &str) -> String {
    if value >= 10.0 || value.fract() == 0.0 {
        format!("{:.0}{suffix}", value)
    } else {
        format!("{value:.1}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::format_size_short;

    #[test]
    /// 驗證大小格式會轉成人類較容易閱讀的單位顯示。
    fn format_size_short_uses_compact_units() {
        assert_eq!(format_size_short(512), "512b");
        assert_eq!(format_size_short(2_048), "2kb");
        assert_eq!(format_size_short(1_572_864), "1.5mb");
        assert_eq!(format_size_short(3_221_225_472), "3G");
    }
}

/// 繪製刪除確認視窗。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，整體可用畫面範圍。
/// - `target_name: &str`，要顯示的刪除目標名稱。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `config: &AppConfig`，控制 popup 尺寸的應用程式設定。
///
/// 回傳：`()`
pub(crate) fn render_confirm_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    target_name: &str,
    theme: Theme,
    config: &AppConfig,
) {
    let dialog_area = centered_rect(
        area,
        config.ui.dialogs.confirm.width_percent,
        config.ui.dialogs.confirm.height,
    );
    frame.render_widget(Clear, dialog_area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("Move {target_name} to trash?")),
            Line::from("Press y to confirm, n or Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Confirm Trash ",
                    theme.danger_title_style(),
                )))
                .borders(Borders::ALL),
        ),
        dialog_area,
    );
}

/// 繪製主題選擇視窗。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，整體可用畫面範圍。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `selected: usize`，主題選單目前選取的索引位置。
/// - `config: &AppConfig`，控制 popup 尺寸的應用程式設定。
///
/// 回傳：`()`
pub(crate) fn render_theme_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    selected: usize,
    config: &AppConfig,
) {
    let dialog_area = centered_rect(
        area,
        config.ui.dialogs.theme_picker.width_percent,
        config.ui.dialogs.theme_picker.height,
    );
    frame.render_widget(Clear, dialog_area);

    let items: Vec<ListItem<'static>> = ThemePreset::ALL
        .iter()
        .map(|preset| ListItem::new(Line::from(preset.name().to_string())))
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(selected));

    let list = List::new(items)
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Theme Picker ",
                    theme.accent_style().add_modifier(Modifier::BOLD),
                )))
                .borders(Borders::ALL),
        )
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, dialog_area, &mut list_state);
}
