use chrono::{DateTime, Local};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use std::path::Path;

use crate::{
    config::AppConfig,
    theme::{Theme, ThemePreset},
};

use super::{
    app::TrashConfirmAction,
    pane::{PaneState, SortDetailKind},
    search::GlobalSearchEntry,
};

/// 描述底部快捷鍵面板中的單一項目。
#[derive(Clone, Copy)]
struct ShortcutPanelItem<'a> {
    shortcut: &'a str,
    label: &'a str,
}

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
    pub(crate) preview_query: Option<&'a str>,
    pub(crate) preview_scroll: Option<usize>,
    pub(crate) preview_current_match: Option<usize>,
}

/// 描述目前 pane 的列表區是否被某種特殊模式接管。
#[derive(Clone, Copy)]
pub(crate) enum PaneListState<'a> {
    Search(SearchListState<'a>),
    Tasks {
        lines: &'a [TaskPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
    },
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

/// 描述 task 面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskPanelLine {
    pub(crate) state: String,
    pub(crate) time: String,
    pub(crate) title: String,
    pub(crate) detail: String,
}

/// 描述書籤列表彈窗中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BookmarkPanelLine {
    pub(crate) key: String,
    pub(crate) path: String,
}

/// 描述 zoxide 目錄列表彈窗中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ZoxidePanelLine {
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
    _config: &AppConfig,
    editor_state: Option<InlineEditorState<'_>>,
    picker_state: Option<InlinePickerState<'_>>,
    list_find_buffer: Option<&str>,
    list_find_editing: bool,
) -> Option<(u16, u16)> {
    let visual_mode_active = visual_range.is_some();
    let mark_column_active = visual_mode_active || pane.marked_count() > 0;
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
        Some(PaneListState::Tasks { .. }) => "  [tasks]",
        Some(PaneListState::Trash { .. }) => "  [trash d/D u/U]",
        Some(PaneListState::Help { .. }) => "  [help]",
        Some(PaneListState::RegexRename { .. }) => "  [rename-regex]",
        None => "",
    };
    let title = format_pane_title(
        pane_id,
        pane.cwd.as_path(),
        filter_suffix,
        &mark_suffix,
        panel_suffix,
        &pane.title_mode_label(),
        area.width.saturating_sub(3) as usize,
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    if preview_focused {
        let preview_viewport_height = area.height.saturating_sub(2).max(1) as usize;
        let (preview_title, preview_lines) = match panel_state {
            Some(PaneListState::Search(search_state))
                if !search_state.loading
                    && !search_state.results.is_empty()
                    && search_state.preview_query.is_some() =>
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
                    .map(|entry| {
                        let mut title = format!("Preview: {}", entry.name);
                        title.push_str("  [preview]");
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
                pane.set_preview_viewport_height(preview_viewport_height);
                (
                    default_preview_title,
                    pane.preview_lines(preview_viewport_height, theme),
                )
            }
        };
        let preview_content_width = area.width.saturating_sub(2) as usize;
        let preview_lines =
            pad_preview_lines_for_render(preview_lines, preview_content_width, theme);
        let preview = Paragraph::new(preview_lines).block(
            Block::default()
                .title(preview_title)
                .borders(Borders::ALL)
                .border_style(border_style),
        );
        frame.render_widget(preview, area);
        return None;
    }

    let content_width = area.width.saturating_sub(4) as usize;
    let list_viewport_height = area.height.saturating_sub(2).max(1) as usize;
    pane.set_list_viewport_height(list_viewport_height);
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
            PaneListState::Tasks { lines, .. } if lines.is_empty() => {
                vec![ListItem::new(Line::from("No tasks yet"))]
            }
            PaneListState::Tasks { lines, .. } => lines
                .iter()
                .map(|line| {
                    ListItem::new(Line::from(format!(
                        "{:<10} {:<8} {:<24} {}",
                        truncate_text(&line.state, 10),
                        truncate_text(&line.time, 8),
                        truncate_text(&line.title, 24),
                        line.detail
                    )))
                })
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
        let detail_kind = pane.active_detail_kind();
        let find_match_position = pane.list_find_match_position();
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
                        pane.list_find_query(),
                        find_match_position.filter(|_| entry.0 == pane.selected),
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
            PaneListState::RegexRename { lines, selected } if !lines.is_empty() => {
                list_state.select(Some(selected.min(lines.len().saturating_sub(1))));
            }
            _ => {}
        }
        frame.render_stateful_widget(list, area, &mut list_state);
    } else {
        frame.render_stateful_widget(list, area, &mut pane.list_state);
    }

    let mut editor_cursor = None;
    if let Some(state) = editor_state {
        editor_cursor = render_inline_editor(frame, area, pane, theme, state);
    }
    if let Some(state) = picker_state {
        render_inline_picker(frame, area, pane, theme, state);
    }

    let panel_cursor = match panel_state {
        Some(PaneListState::Trash {
            search,
            editing: true,
            ..
        }) => Some(render_top_right_input(
            frame,
            area,
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
            area,
            theme,
            "Help Search",
            search,
        )),
        Some(PaneListState::Tasks {
            search,
            editing: true,
            ..
        }) => Some(render_top_right_input(
            frame,
            area,
            theme,
            "Task Search",
            search,
        )),
        _ if list_find_editing => Some(render_top_right_input(
            frame,
            area,
            theme,
            "Find next",
            list_find_buffer.unwrap_or_default(),
        )),
        _ => None,
    };

    editor_cursor.or(panel_cursor)
}

/// 組合 pane 標題列文字，讓 pane 編號固定顯示在最前面，方便搭配數字切換。
fn format_pane_title(
    pane_id: usize,
    cwd: &Path,
    filter_suffix: &str,
    mark_suffix: &str,
    panel_suffix: &str,
    mode_label: &str,
    max_width: usize,
) -> String {
    let prefix = format!("panel #{pane_id}");
    let full_path = cwd.display().to_string();
    let status_suffix =
        normalize_title_status_segments(&[filter_suffix, mark_suffix, panel_suffix]);
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
            return full_title;
        }

        let separator_width = title_separator_width(true, !suffix.is_empty());
        let fixed_width = prefix.chars().count() + suffix.chars().count() + separator_width;
        let path_width = max_width.saturating_sub(fixed_width).max(1);
        let compact_path = compact_path_for_title(&full_path, path_width);
        let compact_title = join_title_parts(&prefix, &compact_path, &suffix);
        if compact_title.chars().count() <= max_width {
            return compact_title;
        }
    }

    let fallback_path_width = max_width
        .saturating_sub(prefix.chars().count())
        .saturating_sub(1)
        .max(1);
    join_title_parts(
        &prefix,
        &compact_path_for_title(&full_path, fallback_path_width),
        "",
    )
}

/// 將多個標題狀態片段去掉前後空白後重新用單一空格組合，避免出現多餘空隙。
fn normalize_title_status_segments(segments: &[&str]) -> String {
    segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 將 pane 標題的 prefix / path / suffix 以最少必要空格拼接，避免浪費可用寬度。
fn join_title_parts(prefix: &str, path: &str, suffix: &str) -> String {
    let mut parts = vec![prefix.to_string()];
    if !path.is_empty() {
        parts.push(path.to_string());
    }
    if !suffix.is_empty() {
        parts.push(suffix.to_string());
    }
    parts.join(" ")
}

/// 計算 prefix / path / suffix 三段之間實際需要的空格數，供路徑可用寬度估算使用。
fn title_separator_width(has_path: bool, has_suffix: bool) -> usize {
    let mut spaces = 0;
    if has_path {
        spaces += 1;
    }
    if has_suffix {
        spaces += 1;
    }
    spaces
}

/// 專門為 pane 標題壓縮過長路徑，優先保留最後幾層目錄名稱與檔名尾端。
fn compact_path_for_title(path: &str, max_chars: usize) -> String {
    if path.chars().count() <= max_chars {
        return path.to_string();
    }
    if max_chars <= 1 {
        return String::from("…");
    }

    let separator = if path.contains('\\') && !path.contains('/') {
        '\\'
    } else {
        '/'
    };
    let separator_text = separator.to_string();

    let (path_prefix, remainder) = if path.starts_with('/') {
        (String::from("/"), &path[1..])
    } else if path.len() >= 3
        && path.as_bytes().get(1) == Some(&b':')
        && matches!(path.as_bytes().get(2), Some(b'/') | Some(b'\\'))
    {
        (path[..3].to_string(), &path[3..])
    } else {
        (String::new(), path)
    };

    let parts = remainder
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return truncate_text_end_preserving_tail(path, max_chars);
    }

    let last_part = parts.last().copied().unwrap_or(path);
    let last_part_only = compact_last_segment_only(last_part, max_chars);

    if parts.len() == 1 {
        return last_part_only;
    }

    let mut best = if path_prefix.is_empty() {
        last_part_only.clone()
    } else {
        let rooted = format!("{path_prefix}{last_part}");
        if rooted.chars().count() <= max_chars {
            rooted
        } else {
            last_part_only.clone()
        }
    };

    for start in (0..parts.len()).rev() {
        let tail = parts[start..].join(&separator_text);
        let candidate = if start == parts.len() - 1 {
            format!("…{tail}")
        } else {
            format!("…{separator}{tail}")
        };
        if candidate.chars().count() <= max_chars
            && candidate.chars().count() >= best.chars().count()
        {
            best = candidate;
        }
    }

    best
}

/// 優先保留字串尾端，只在前方放上 `…` 表示前面內容被省略。
fn truncate_text_end_preserving_tail(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return String::from("…");
    }
    let tail = chars
        .iter()
        .skip(chars.len().saturating_sub(max_chars - 1))
        .collect::<String>();
    format!("…{tail}")
}

/// 在放不下整層目錄時，直接退化成 `…最後目錄尾端` 的顯示形式。
fn compact_last_segment_only(last_part: &str, max_chars: usize) -> String {
    if last_part.chars().count() <= max_chars {
        last_part.to_string()
    } else if max_chars <= 1 {
        String::from("…")
    } else {
        truncate_text_end_preserving_tail(last_part, max_chars)
    }
}

/// 將 preview 行內容依照可見寬度補齊，讓目前命中列的背景可以延伸到整行右側。
fn pad_preview_lines_for_render(
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
    title: &str,
    buffer: &str,
    _editing: bool,
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
            title,
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
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Sort ",
        &[
            ShortcutPanelItem {
                shortcut: "m",
                label: "modified",
            },
            ShortcutPanelItem {
                shortcut: "M",
                label: "modified (reverse)",
            },
            ShortcutPanelItem {
                shortcut: "b",
                label: "birth",
            },
            ShortcutPanelItem {
                shortcut: "B",
                label: "birth (reverse)",
            },
            ShortcutPanelItem {
                shortcut: "a",
                label: "alphabetical",
            },
            ShortcutPanelItem {
                shortcut: "A",
                label: "alphabetical (reverse)",
            },
            ShortcutPanelItem {
                shortcut: "n",
                label: "natural",
            },
            ShortcutPanelItem {
                shortcut: "N",
                label: "natural (reverse)",
            },
            ShortcutPanelItem {
                shortcut: "e",
                label: "extension",
            },
            ShortcutPanelItem {
                shortcut: "E",
                label: "extension (reverse)",
            },
            ShortcutPanelItem {
                shortcut: "s",
                label: "size",
            },
            ShortcutPanelItem {
                shortcut: "S",
                label: "size (reverse)",
            },
            ShortcutPanelItem {
                shortcut: "r",
                label: "random",
            },
            ShortcutPanelItem {
                shortcut: "Esc",
                label: "cancel",
            },
        ],
    );
}

/// 在畫面底部繪製 linemode 快捷鍵面板，供 `m` 使用。
pub(crate) fn render_linemode_picker(frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " LineMode ",
        &[
            ShortcutPanelItem {
                shortcut: "s",
                label: "linemode size",
            },
            ShortcutPanelItem {
                shortcut: "p",
                label: "linemode permissions",
            },
            ShortcutPanelItem {
                shortcut: "b",
                label: "linemode btime",
            },
            ShortcutPanelItem {
                shortcut: "t",
                label: "linemode mtime",
            },
            ShortcutPanelItem {
                shortcut: "n",
                label: "linemode none",
            },
            ShortcutPanelItem {
                shortcut: "Esc",
                label: "cancel",
            },
        ],
    );
}

/// 在畫面中央繪製書籤列表彈窗，供 `:bookmark list` 使用。
pub(crate) fn render_bookmark_action_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Bookmark ",
        &[
            ShortcutPanelItem {
                shortcut: "a",
                label: "add bookmark (auto key)",
            },
            ShortcutPanelItem {
                shortcut: "g",
                label: "jump from list",
            },
            ShortcutPanelItem {
                shortcut: "d",
                label: "delete one bookmark",
            },
            ShortcutPanelItem {
                shortcut: "D",
                label: "delete all bookmarks",
            },
            ShortcutPanelItem {
                shortcut: "Esc",
                label: "cancel",
            },
        ],
    );
}

/// 在畫面底部繪製 `t` 系列 trash 快捷鍵面板。
pub(crate) fn render_trash_action_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Trash ",
        &[
            ShortcutPanelItem {
                shortcut: "t",
                label: "open trash list",
            },
            ShortcutPanelItem {
                shortcut: "u",
                label: "undo latest trash",
            },
            ShortcutPanelItem {
                shortcut: "t / Esc",
                label: "cancel",
            },
        ],
    );
}

/// 在畫面底部繪製 `g` 系列命令面板，供 `gg` 與 `gt` 這類兩段式操作共用。
pub(crate) fn render_go_picker(frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Go ",
        &[
            ShortcutPanelItem {
                shortcut: "g",
                label: "jump top",
            },
            ShortcutPanelItem {
                shortcut: "t",
                label: "goto path",
            },
            ShortcutPanelItem {
                shortcut: "d",
                label: "documents",
            },
            ShortcutPanelItem {
                shortcut: "k",
                label: "desktop",
            },
            ShortcutPanelItem {
                shortcut: "Esc",
                label: "cancel",
            },
        ],
    );
}

/// 在畫面底部繪製 panel 操作快捷鍵面板，供 `w` 使用。
pub(crate) fn render_window_picker(frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Panel ",
        &[
            ShortcutPanelItem {
                shortcut: "h",
                label: "split left",
            },
            ShortcutPanelItem {
                shortcut: "j",
                label: "split down",
            },
            ShortcutPanelItem {
                shortcut: "k",
                label: "split up",
            },
            ShortcutPanelItem {
                shortcut: "l",
                label: "split right",
            },
            ShortcutPanelItem {
                shortcut: "c",
                label: "close panel",
            },
            ShortcutPanelItem {
                shortcut: "o",
                label: "keep only current panel",
            },
            ShortcutPanelItem {
                shortcut: "Esc",
                label: "cancel",
            },
        ],
    );
}

/// 將底部快捷鍵面板項目依目前寬度自動分欄，避免小 pane 時擠成難讀的一長行。
fn render_shortcut_grid_panel(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    title: &str,
    items: &[ShortcutPanelItem<'_>],
) {
    let lines = shortcut_panel_lines(items, area.width.saturating_sub(2) as usize, theme);
    let desired_height = (lines.len() as u16).saturating_add(2).max(4);
    let panel_height = desired_height.min(area.height.max(1));
    let panel_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(panel_height),
        width: area.width,
        height: panel_height,
    };

    frame.render_widget(Clear, panel_area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(Line::from(Span::styled(
                    title,
                    theme.accent_style().add_modifier(Modifier::BOLD),
                )))
                .borders(Borders::TOP),
        ),
        panel_area,
    );
}

/// 依目前可用寬度，把快捷鍵項目排成多欄對齊的行。
fn shortcut_panel_lines(
    items: &[ShortcutPanelItem<'_>],
    available_width: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    if items.is_empty() {
        return vec![Line::from("")];
    }

    let cell_width = items
        .iter()
        .map(shortcut_panel_item_width)
        .max()
        .unwrap_or(1)
        .saturating_add(2);
    let columns = available_width.max(1) / cell_width.max(1);
    let columns = columns.max(1);

    items
        .chunks(columns)
        .map(|row| {
            let mut spans = Vec::new();
            for (index, item) in row.iter().enumerate() {
                spans.push(Span::styled(
                    item.shortcut.to_string(),
                    theme.accent_style(),
                ));
                spans.push(Span::raw(" -> "));
                spans.push(Span::raw(item.label.to_string()));

                if index + 1 < row.len() {
                    let used_width = shortcut_panel_item_width(item);
                    let padding = cell_width.saturating_sub(used_width).max(2);
                    spans.push(Span::raw(" ".repeat(padding)));
                }
            }
            Line::from(spans)
        })
        .collect()
}

/// 計算單一快捷鍵項目在面板中實際會佔用的字元寬度。
fn shortcut_panel_item_width(item: &ShortcutPanelItem<'_>) -> usize {
    item.shortcut.chars().count() + 4 + item.label.chars().count()
}

/// 在畫面中央繪製書籤列表彈窗，供 `:bookmark list` 使用。
pub(crate) fn render_bookmark_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    lines: &[BookmarkPanelLine],
    selected: usize,
    title: &str,
    empty_message: &str,
    search: &str,
    editing: bool,
) -> Option<(u16, u16)> {
    let popup_height = (lines.len().min(10) as u16).saturating_add(4).max(6);
    let popup_area = centered_rect(area, 68, popup_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            title,
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let items = if lines.is_empty() {
        vec![ListItem::new(Line::from(empty_message))]
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

    editing.then(|| render_top_right_input(frame, popup_area, theme, "Filter", search))
}

/// 在畫面中央繪製 zoxide 目錄列表彈窗，供 `Z` 與 `:zoxide` 共用。
pub(crate) fn render_zoxide_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    lines: &[ZoxidePanelLine],
    selected: usize,
    search: &str,
    editing: bool,
) -> Option<(u16, u16)> {
    let popup_height = (lines.len().min(10) as u16).saturating_add(4).max(6);
    let popup_area = centered_rect(area, 68, popup_height);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Zoxide ",
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.accent_style());
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let items = if lines.is_empty() {
        vec![ListItem::new(Line::from("zoxide 還沒有學到任何目錄"))]
    } else {
        lines
            .iter()
            .map(|line| ListItem::new(Line::from(line.path.clone())))
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

    editing.then(|| render_top_right_input(frame, popup_area, theme, "Filter", search))
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
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    frame.render_widget(Paragraph::new(format!(":{}", buffer)), chunks[0]);

    if !suggestions.is_empty() && chunks.len() > 1 {
        let items = suggestions
            .iter()
            .map(|line| {
                let text = if line.description.trim().is_empty() {
                    line.display_command.clone()
                } else {
                    format!(
                        "{:<22}  {}",
                        truncate_text(&line.display_command, 22),
                        line.description
                    )
                };
                ListItem::new(Line::from(text))
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
    list_find_query: Option<&str>,
    list_find_position: Option<(usize, usize)>,
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
    let name = entry.display_name();
    let badge = list_find_position.map(|(current, total)| format!("[{current}/{total}]"));
    let detail = format_sort_detail(entry, detail_kind);
    let badge_len = badge
        .as_ref()
        .map(|value| value.chars().count() + 1)
        .unwrap_or(0);
    let name_len = marker.chars().count() + name.chars().count() + badge_len;

    if detail.is_empty() || width < 8 {
        let mut spans = Vec::new();
        if !marker.is_empty() {
            spans.push(Span::raw(marker.to_string()));
        }
        spans.extend(highlight_name_spans(&name, list_find_query, theme));
        if let Some(badge) = badge {
            spans.push(Span::raw(" ".to_string()));
            spans.push(Span::styled(
                badge,
                theme
                    .accent_style()
                    .bg(theme.selection_bg)
                    .fg(theme.selection_fg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        return Line::from(spans);
    }

    let detail_len = detail.chars().count();
    let spacer_len = width.saturating_sub(name_len + detail_len).max(1);

    let mut spans = Vec::new();
    if !marker.is_empty() {
        spans.push(Span::raw(marker.to_string()));
    }
    spans.extend(highlight_name_spans(&name, list_find_query, theme));
    if let Some(badge) = badge {
        spans.push(Span::raw(" ".to_string()));
        spans.push(Span::styled(
            badge,
            theme
                .accent_style()
                .bg(theme.selection_bg)
                .fg(theme.selection_fg)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::raw(" ".repeat(spacer_len)));
    spans.push(Span::styled(detail, theme.muted_style()));

    Line::from(spans)
}

/// 依照目前的 list find 查詢，把檔名切成一般片段與高亮片段。
fn highlight_name_spans(name: &str, query: Option<&str>, theme: Theme) -> Vec<Span<'static>> {
    let Some(query) = query.filter(|value| !value.is_empty()) else {
        return vec![Span::raw(name.to_string())];
    };

    let lower_name = name.to_lowercase();
    let lower_query = query.to_lowercase();
    let mut spans = Vec::new();
    let mut search_start = 0usize;
    let mut byte_start = 0usize;

    while let Some(relative_match) = lower_name[search_start..].find(&lower_query) {
        let match_start = search_start + relative_match;
        let match_end = match_start + lower_query.len();

        if let Some(prefix) = name.get(byte_start..match_start)
            && !prefix.is_empty()
        {
            spans.push(Span::raw(prefix.to_string()));
        }
        if let Some(matched) = name.get(match_start..match_end) {
            spans.push(Span::styled(
                matched.to_string(),
                highlight_match_style(theme),
            ));
        }

        search_start = match_end;
        byte_start = match_end;
    }

    if let Some(suffix) = name.get(byte_start..)
        && !suffix.is_empty()
    {
        spans.push(Span::raw(suffix.to_string()));
    }

    if spans.is_empty() {
        vec![Span::raw(name.to_string())]
    } else {
        spans
    }
}

/// 回傳列表內 find-next 命中文字使用的高亮樣式。
fn highlight_match_style(theme: Theme) -> Style {
    theme
        .accent_style()
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

/// 依照目前排序依據，決定右側欄位要顯示的文字。
fn format_sort_detail(entry: &super::entry::FileEntry, detail_kind: SortDetailKind) -> String {
    match detail_kind {
        SortDetailKind::None => String::new(),
        SortDetailKind::Size => {
            if entry.is_dir {
                entry
                    .child_count
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| String::from("?"))
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
        SortDetailKind::Permissions => format_permissions_detail(entry),
    }
}

/// 依照目前平台與快取 metadata，產生適合列表右側顯示的權限字串。
fn format_permissions_detail(entry: &super::entry::FileEntry) -> String {
    if let Some(mode) = entry.unix_mode {
        return format_unix_permissions(entry.is_dir, mode);
    }

    let kind = if entry.is_dir { "dir" } else { "file" };
    let access = if entry.readonly {
        "readonly"
    } else {
        "writable"
    };
    format!("{kind} {access}")
}

/// 把 Unix 權限位元轉成類似 `drwxr-xr-x` 的緊湊字串。
fn format_unix_permissions(is_dir: bool, mode: u32) -> String {
    let mut result = String::with_capacity(10);
    result.push(if is_dir { 'd' } else { '-' });

    for shift in [6_u32, 3_u32, 0_u32] {
        result.push(if mode & (0o4 << shift) != 0 { 'r' } else { '-' });
        result.push(if mode & (0o2 << shift) != 0 { 'w' } else { '-' });
        result.push(if mode & (0o1 << shift) != 0 { 'x' } else { '-' });
    }

    result
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
    use super::{format_pane_title, format_permissions_detail, format_size_short};
    use std::path::Path;
    use std::time::SystemTime;

    use crate::file_manager::entry::FileEntry;

    #[test]
    /// 驗證大小格式會轉成人類較容易閱讀的單位顯示。
    fn format_size_short_uses_compact_units() {
        assert_eq!(format_size_short(512), "512b");
        assert_eq!(format_size_short(2_048), "2kb");
        assert_eq!(format_size_short(1_572_864), "1.5mb");
        assert_eq!(format_size_short(3_221_225_472), "3G");
    }

    #[test]
    /// 驗證 pane 標題會把固定 pane 編號顯示在最前面，方便對照快捷鍵切換。
    fn format_pane_title_keeps_stable_pane_id_prefix() {
        let title = format_pane_title(
            3,
            Path::new("/tmp/demo"),
            "  [filter]",
            "  [mark: 2]",
            "  [help]",
            "sort: natural",
            80,
        );

        assert_eq!(
            title,
            "panel #3 /tmp/demo [filter] [mark: 2] [help] [sort: natural]"
        );
    }

    #[test]
    /// 驗證 pane 寬度足夠時，標題會完整顯示，不會過早縮短路徑。
    fn format_pane_title_keeps_full_path_when_it_fits() {
        let title = format_pane_title(
            2,
            Path::new("/Users/otto/Documents/terminal-file-manager"),
            "",
            "",
            "",
            "sort: natural",
            120,
        );

        assert_eq!(
            title,
            "panel #2 /Users/otto/Documents/terminal-file-manager [sort: natural]"
        );
    }

    #[test]
    /// 驗證 pane 很窄時，標題仍會自動縮短，不會超出可用寬度。
    fn format_pane_title_compacts_long_path_for_narrow_panes() {
        let title = format_pane_title(
            12,
            Path::new("/Users/otto/GitHub/cocos-tutorial-happy-path/dev/library/very/deep/path"),
            "",
            "",
            "",
            "sort: natural",
            32,
        );

        assert!(title.chars().count() <= 32);
        assert!(title.starts_with("panel #12"));
        assert!(title.contains("natural"));
        assert!(title.contains("path"));
        assert!(!title.contains("happy-path/dev"));
        assert!(!title.contains("/…/…"));
    }

    #[test]
    /// 驗證路徑放不下時，會優先保留最後目錄名稱，而不是在前面留下多餘根路徑。
    fn format_pane_title_prefers_last_directory_tail() {
        let title = format_pane_title(
            4,
            Path::new("/Users/otto/Documents/terminal-file-manager"),
            "",
            "",
            "",
            "sort: natural",
            40,
        );

        assert!(title.contains("…"));
        assert!(title.contains("file-manager"));
        assert!(title.contains("[sort: natural]"));
        assert!(!title.contains("/…/…"));
    }

    #[test]
    /// 驗證 linemode 開啟後，pane 標題尾端會顯示目前啟用的 linemode。
    fn format_pane_title_supports_linemode_status() {
        let title = format_pane_title(5, Path::new("/tmp/demo"), "", "", "", "linemode: size", 80);

        assert_eq!(title, "panel #5 /tmp/demo [linemode: size]");
    }

    #[test]
    /// 驗證非 Unix 平台或缺少 mode bits 時，permissions 仍有可讀的 fallback 顯示。
    fn format_permissions_detail_falls_back_to_cross_platform_text() {
        let entry = FileEntry {
            name: String::from("notes.txt"),
            path: Path::new("/tmp/notes.txt").to_path_buf(),
            is_dir: false,
            size: 12,
            child_count: None,
            modified: SystemTime::UNIX_EPOCH,
            created: SystemTime::UNIX_EPOCH,
            readonly: true,
            unix_mode: None,
        };

        assert_eq!(format_permissions_detail(&entry), "file readonly");
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
    permanent: bool,
    theme: Theme,
    config: &AppConfig,
) {
    let dialog_area = centered_rect(
        area,
        config.ui.dialogs.confirm.width_percent,
        config.ui.dialogs.confirm.height,
    );
    frame.render_widget(Clear, dialog_area);
    let (title, question) = if permanent {
        (
            " Confirm Delete ",
            format!("Delete {target_name} permanently?"),
        )
    } else {
        (" Confirm Trash ", format!("Move {target_name} to trash?"))
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(question),
            Line::from("Press y to confirm, n or Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(Line::from(Span::styled(title, theme.danger_title_style())))
                .borders(Borders::ALL),
        ),
        dialog_area,
    );
}

/// 繪製 trash 專用的確認視窗，讓 restore/delete 都能顯示正確的說明。
pub(crate) fn render_trash_confirm_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    action: &TrashConfirmAction,
    target_name: &str,
    entry_count: usize,
    theme: Theme,
    config: &AppConfig,
) {
    let dialog_area = centered_rect(
        area,
        config.ui.dialogs.confirm.width_percent,
        config.ui.dialogs.confirm.height,
    );
    frame.render_widget(Clear, dialog_area);

    let (title, verb) = match action {
        TrashConfirmAction::RestoreFromPanel { .. } => (" Confirm Restore ", "Restore"),
        TrashConfirmAction::DeleteFromPanel { .. } => (" Confirm Delete ", "Delete"),
    };
    let question = if entry_count <= 1 {
        format!("{verb} {target_name}?")
    } else {
        format!("{verb} {target_name} ({entry_count} items)?")
    };

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(question),
            Line::from("Press y to confirm, n or Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(Line::from(Span::styled(title, theme.danger_title_style())))
                .borders(Borders::ALL),
        ),
        dialog_area,
    );
}

/// 繪製貼上覆蓋確認視窗。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，整體可用畫面範圍。
/// - `target_name: &str`，這次會被覆蓋的目標名稱摘要。
/// - `entry_count: usize`，這次整批貼上的項目數量。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `config: &AppConfig`，控制 popup 尺寸的應用程式設定。
///
/// 回傳：`()`
pub(crate) fn render_paste_overwrite_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    target_name: &str,
    entry_count: usize,
    theme: Theme,
    config: &AppConfig,
) {
    let dialog_area = centered_rect(
        area,
        config.ui.dialogs.confirm.width_percent,
        config.ui.dialogs.confirm.height,
    );
    frame.render_widget(Clear, dialog_area);
    let question = if entry_count <= 1 {
        format!("Overwrite existing item {target_name}?")
    } else {
        format!("Overwrite existing items {target_name}?")
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(question),
            Line::from("Press y or Enter to overwrite, n or Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Confirm Paste Overwrite ",
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
