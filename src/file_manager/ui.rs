//! ratatui 畫面組裝與純顯示格式化函數。
//!
//! 本模組只根據 `App`/`PaneState` 的快照繪圖，不執行檔案操作或改變業務狀態。
//! panel 內 UI 應限制在傳入的 `Rect`，顏色一律取自 `Theme`，狹窄視窗則交由本層
//! 的截斷與動態對齊 helper 處理，避免各功能自行計算造成版面不一致。

use chrono::{DateTime, Local};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    config::{AppConfig, IconStyle},
    theme::{Theme, ThemePreset},
};

use super::{
    app::{RenameMode, TrashConfirmAction},
    diff::{DiffEntryState, DiffMatrixState, DiffStatus},
    pane::{PaneState, SortDetailKind},
    search::GlobalSearchEntry,
    tools::ToolStatus,
};

/// 描述底部快捷鍵面板中的單一項目。
#[derive(Clone, Copy)]
struct ShortcutPanelItem<'a> {
    shortcut: &'a str,
    label: &'a str,
}

/// 描述 command palette 繪製所需的狀態。
pub(crate) struct CommandPaletteState<'a> {
    pub(crate) buffer: &'a str,
    pub(crate) suggestions: &'a [CommandSuggestionLine],
    pub(crate) selected: usize,
    pub(crate) cursor: usize,
    pub(crate) mode: RenameMode,
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
        cursor: usize,
    },
    Trash {
        lines: &'a [TrashPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
        cursor: usize,
    },
    Help {
        lines: &'a [HelpPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
        cursor: usize,
        custom_title: Option<&'a str>,
    },
    Tools {
        statuses: &'a [ToolStatus],
        selected: usize,
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
    pub(crate) started_at: String,
    pub(crate) finished_at: String,
    pub(crate) progress: String,
    pub(crate) title: String,
    /// 任務來源位置；多選操作可包含多筆，渲染時會限制展開數量避免面板過長。
    pub(crate) source_locations: Vec<String>,
    /// 任務目的位置；刪除等沒有目的地的工作使用 `None`。
    pub(crate) destination_location: Option<String>,
    pub(crate) detail: String,
    pub(crate) marked: bool,
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
    pub(crate) shortcut: String,
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
        Some(PaneListState::Help {
            custom_title: Some(_),
            ..
        }) => "  [cheatsheet ?]",
        Some(PaneListState::Help { .. }) => "  [help ~/F1]",
        Some(PaneListState::Tools { .. }) => "  [dependencies Esc]",
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
    // 一般檔案列表只建立目前 viewport 內的 widget。大型目錄可能有數萬筆項目，
    // 若每一幀仍替畫面外項目配置空白 ListItem，單次 j/k 也會產生 O(n) 配置並卡住。
    // panel overlay 的資料量通常很小，維持原本完整列表即可。
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
        frame.render_stateful_widget(list, area, &mut list_state);
    } else {
        if let Some(window_start) = normal_list_window_start {
            // 傳給 ratatui 的 items 已是局部 viewport，因此 selected 也必須轉成局部索引。
            // PaneState 仍保存完整列表索引與 window 起點，鍵盤、find、mark 等邏輯不會
            // 因虛擬化而改變語意。
            let mut viewport_state = ListState::default();
            viewport_state.select(Some(pane.selected.saturating_sub(window_start)));
            frame.render_stateful_widget(list, area, &mut viewport_state);
            pane.list_state.select(Some(pane.selected));
            *pane.list_state.offset_mut() = window_start;
        } else {
            frame.render_stateful_widget(list, area, &mut pane.list_state);
        }
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
            cursor,
            ..
        }) => Some(render_top_right_input(
            frame,
            area,
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
            area,
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
            area,
            theme,
            "Task Search",
            search,
            cursor,
        )),
        _ if list_find_editing => Some(render_top_right_input(
            frame,
            area,
            theme,
            "Find next",
            list_find_buffer.unwrap_or_default(),
            text_input_cursor,
        )),
        _ => None,
    };

    editor_cursor.or(panel_cursor)
}

/// 回傳搜尋列表在尚未收到任何結果時應顯示的提示文字。
///
/// 參數：
/// - `state: &SearchListState`，目前搜尋列表的結果與載入狀態。
///
/// 回傳：`Some(&str)` 代表列表要顯示提示；`None` 代表已有結果，應直接顯示結果。
fn search_empty_message(state: &SearchListState<'_>) -> Option<&'static str> {
    if !state.results.is_empty() {
        None
    } else if state.loading {
        Some("Loading search results...")
    } else {
        Some("No matches")
    }
}

/// 計算 global search 列表目前應該反白的項目索引。
///
/// 參數：
/// - `search_state: &SearchListState<'_>`，包含串流結果與目前游標位置的搜尋列表資料。
///
/// 回傳：`Option<usize>`。
/// - 列表已有內容時回傳合法索引；即使背景工作仍在載入，也會立即顯示游標。
/// - 列表尚無內容時回傳 `None`。
fn search_list_selected_index(search_state: &SearchListState<'_>) -> Option<usize> {
    (!search_state.results.is_empty()).then(|| {
        search_state
            .selected
            .min(search_state.results.len().saturating_sub(1))
    })
}

/// 計算大型一般列表本幀真正需要建立 widget 的 viewport 範圍。
///
/// 參數：
/// - `total: usize`：filter 後列表的完整項目數。
/// - `selected: usize`：完整列表中的游標索引。
/// - `viewport_height: usize`：panel 目前可顯示的資料列數。
/// - `previous_start: usize`：上一幀 viewport 的起點，用來避免游標移動時畫面無故跳動。
///
/// 回傳：`(usize, usize)`，採 Rust range 的 `[start, end)` 格式。範圍一定包含合法的
/// `selected`，且長度不超過 viewport。渲染端只走訪這段資料，因此大型目錄按 j/k
/// 不再隨完整項目數增加配置成本。
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
///
/// 參數：
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `status: &str`，預覽列右側的狀態文字。
///
/// 回傳：`Style`，供狀態文字直接套用的顏色樣式。
fn regex_rename_status_style(theme: Theme, status: &str) -> Style {
    match status {
        "ready" => theme.success_style(),
        "unchanged" => theme.muted_style(),
        "conflict" | "invalid" => theme.danger_style(),
        _ => Style::default(),
    }
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

    let (path_prefix, remainder) = if let Some(stripped) = path.strip_prefix('/') {
        (String::from("/"), stripped)
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

/// 計算字串前 `cursor_char_count` 個字元在終端機中的實際顯示寬度（欄數）。
///
/// 中文或全形字元在終端機會佔用 2 欄寬度，若直接使用字元數或 byte 數計算游標的
/// 螢幕 X 座標，會導致游標落在錯誤字元上（例如中文檔名會使游標看起來停在中間）。
/// 這裡逐字累加 [`UnicodeWidthChar::width`]，確保終端機光標精確對齊插入點。
///
/// 參數：
/// - `text: &str`，輸入緩衝區的文字內容。
/// - `cursor_char_count: usize`，以 Unicode scalar (char) 為單位的游標索引。
///
/// 回傳：`usize`，游標在終端機畫面上對應的顯示欄數。
#[allow(dead_code)]
pub(crate) fn cursor_display_width(text: &str, cursor_char_count: usize) -> usize {
    text.chars()
        .take(cursor_char_count)
        .map(|c| c.width().unwrap_or(0))
        .sum()
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
        inner
            .y
            .saturating_add(pane.selected.saturating_sub(pane.list_state.offset()) as u16)
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
    let scrolled = compute_scrolled_input(
        state.buffer,
        state.cursor,
        input_inner.width as usize,
        None,
        theme,
    );
    frame.render_widget(
        Paragraph::new(Line::from(scrolled.spans)).block(input_block),
        input_area,
    );

    Some((
        input_inner.x.saturating_add(scrolled.cursor_col),
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
        inner
            .y
            .saturating_add(pane.selected.saturating_sub(pane.list_state.offset()) as u16)
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

/// 描述單行文字輸入框經過水平滑動視窗計算後的顯示內容與游標欄位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScrolledInputView {
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) cursor_col: u16,
}

/// 針對長文字輸入框計算水平滑動視窗（Scheme A: Viewport Scrolling）。
///
/// 當文字長度超過可用寬度時，自動依目前游標位置決定可見切片，
/// 並在被截斷的一側或兩側加上 `<` / `>` 溢位提示符號。
pub(crate) fn compute_scrolled_input(
    buffer: &str,
    cursor: usize,
    available_width: usize,
    prefix: Option<&str>,
    theme: Theme,
) -> ScrolledInputView {
    let prefix_str = prefix.unwrap_or("");
    let prefix_w = UnicodeWidthStr::width(prefix_str);
    if available_width <= prefix_w {
        return ScrolledInputView {
            spans: vec![Span::raw(prefix_str.to_string())],
            cursor_col: 0,
        };
    }

    let w = available_width - prefix_w;
    let chars: Vec<char> = buffer.chars().collect();
    let char_widths: Vec<usize> = chars
        .iter()
        .map(|c| UnicodeWidthChar::width(*c).unwrap_or(1).max(1))
        .collect();
    let total_w: usize = char_widths.iter().sum();
    let cursor = cursor.min(chars.len());
    let cursor_w: usize = char_widths[..cursor].iter().sum();

    // 寬度足夠顯示全部內容（包括字尾游標預留空間），無需滑動視窗
    if total_w < w {
        let mut spans = Vec::with_capacity(2);
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::raw(buffer.to_string()));
        return ScrolledInputView {
            spans,
            cursor_col: prefix_len_clamp(prefix_w + cursor_w, available_width) as u16,
        };
    }

    // 空間極小時的最小保護
    if w <= 2 {
        let mut spans = Vec::new();
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::styled(
            ">",
            theme.accent_style().add_modifier(Modifier::BOLD),
        ));
        return ScrolledInputView {
            spans,
            cursor_col: prefix_w as u16,
        };
    }

    // 判斷左溢位 (<) 與右溢位 (>)
    let indicator_style = theme.accent_style().add_modifier(Modifier::BOLD);

    // Case 1: 游標靠左側（起點固定為 0，只有右側溢位）
    // 右側預留 1 格給 `>`，可用寬度為 w - 1
    if cursor_w < w.saturating_sub(1) {
        let budget = w.saturating_sub(1);
        let mut accumulated = 0;
        let mut end_idx = 0;
        for (i, &cw) in char_widths.iter().enumerate() {
            if accumulated + cw > budget {
                break;
            }
            accumulated += cw;
            end_idx = i + 1;
        }
        end_idx = end_idx.max(cursor).min(chars.len());

        let visible_str: String = chars[0..end_idx].iter().collect();
        let mut spans = Vec::with_capacity(3);
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::raw(visible_str));
        spans.push(Span::styled(">", indicator_style));

        return ScrolledInputView {
            spans,
            cursor_col: prefix_len_clamp(prefix_w + cursor_w, available_width) as u16,
        };
    }

    // 剩餘寬度從游標到尾端
    let remaining_w: usize = char_widths[cursor..].iter().sum();

    // Case 2: 游標靠右側尾端（終點固定為 chars.len()，只有左側溢位）
    // 左側預留 1 格給 `<`；右側恆定預留 1 格給字尾游標（不顯示 `>`）。
    // 關鍵設計：此預算恆定為 w - 2，絕不隨 cursor == chars.len() 切換而改變。
    // 如此一來，游標在字尾與最後字元之間移動時，start_idx 與可見文字完全固定不動，
    // 字尾與邊界的間距完全保持恆定，徹底杜絕忽大忽小與震盪現象。
    if remaining_w <= 2 || cursor == chars.len() {
        let budget = w.saturating_sub(2);
        let mut accumulated = 0;
        let mut start_idx = chars.len();
        for (i, &cw) in char_widths.iter().enumerate().rev() {
            if accumulated + cw > budget {
                break;
            }
            accumulated += cw;
            start_idx = i;
        }
        start_idx = start_idx.min(cursor);

        let visible_str: String = chars[start_idx..chars.len()].iter().collect();
        let cursor_offset: usize = char_widths[start_idx..cursor].iter().sum();

        let mut spans = Vec::with_capacity(3);
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::styled("<", indicator_style));
        spans.push(Span::raw(visible_str));

        return ScrolledInputView {
            spans,
            cursor_col: prefix_len_clamp(prefix_w + 1 + cursor_offset, available_width) as u16,
        };
    }

    // Case 3: 游標在中間（兩側皆有溢位，左右各留 1 格給 `<` 與 `>`）
    let budget = w.saturating_sub(2);
    let margin_right = 3.min(budget / 3);
    let target_cursor_pos = budget.saturating_sub(margin_right).max(1);

    let mut accumulated = 0;
    let mut start_idx = cursor;
    for (i, &cw) in char_widths[..cursor].iter().enumerate().rev() {
        if accumulated + cw > target_cursor_pos {
            break;
        }
        accumulated += cw;
        start_idx = i;
    }

    let mut forward_acc = 0;
    let mut end_idx = start_idx;
    for (i, &cw) in char_widths[start_idx..].iter().enumerate() {
        if forward_acc + cw > budget {
            break;
        }
        forward_acc += cw;
        end_idx = start_idx + i + 1;
    }
    end_idx = end_idx.max(cursor).min(chars.len());

    let visible_str: String = chars[start_idx..end_idx].iter().collect();
    let cursor_offset: usize = char_widths[start_idx..cursor].iter().sum();

    let has_left = start_idx > 0;
    let has_right = end_idx < chars.len();

    let mut spans = Vec::with_capacity(4);
    if !prefix_str.is_empty() {
        spans.push(Span::raw(prefix_str.to_string()));
    }
    if has_left {
        spans.push(Span::styled("<", indicator_style));
    }
    spans.push(Span::raw(visible_str));
    if has_right {
        spans.push(Span::styled(">", indicator_style));
    }

    let left_pad = if has_left { 1 } else { 0 };
    ScrolledInputView {
        spans,
        cursor_col: prefix_len_clamp(prefix_w + left_pad + cursor_offset, available_width) as u16,
    }
}

fn prefix_len_clamp(val: usize, max: usize) -> usize {
    val.min(max.saturating_sub(1))
}

/// 在畫面右上方繪製小型輸入框，供 filter 與 preview search 這類短文字輸入重用。
fn render_top_right_input(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    title: &str,
    buffer: &str,
    cursor: usize,
) -> (u16, u16) {
    let input_area = top_right_input_rect(area);

    // 若浮動輸入框左側邊界剛好切在 2-width 中文字元中間，終端會因字元跨界而吃掉輸入框左邊框 (┌ / │)。
    // 預先將 left_x 上的寬字元清為半形空格，確保浮動視窗左邊框完整顯示。
    let buf = frame.buffer_mut();
    if input_area.x > 0 {
        let left_x = input_area.x - 1;
        for y in input_area.top()..input_area.bottom() {
            if let Some(cell) = buf.cell_mut((left_x, y))
                && UnicodeWidthStr::width(cell.symbol()) > 1
            {
                cell.set_symbol(" ");
            }
        }
    }

    frame.render_widget(Clear, input_area);
    let title_text = format!(" {} ", title.trim());
    let input_block = Block::default()
        .title(Line::from(Span::styled(
            title_text,
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.focused_border_style());
    let input_inner = input_block.inner(input_area);
    let scrolled = compute_scrolled_input(buffer, cursor, input_inner.width as usize, None, theme);
    frame.render_widget(
        Paragraph::new(Line::from(scrolled.spans)).block(input_block),
        input_area,
    );

    (
        input_inner.x.saturating_add(scrolled.cursor_col),
        input_inner.y,
    )
}

/// 計算 Panel 右上角短文字輸入框的實際範圍，並保證結果不會超出 Panel。
///
/// 一般寬度會保留 Panel 右側一欄空間並使用最多 32 欄；當多重分割讓 Panel
/// 小於預期寬度時，輸入框會跟著縮小，而不是覆蓋相鄰 Panel。高度也採相同規則，
/// 因此極小的 Panel 仍不會畫到自身邊界以外。
///
/// 參數：
/// - `area: Rect`，擁有這個輸入 UI 的 Panel 完整畫面範圍。
///
/// 回傳：`Rect`，限制在 `area` 內、靠右上方的輸入框範圍。
fn top_right_input_rect(area: Rect) -> Rect {
    let right_margin = u16::from(area.width > 1);
    let top_margin = u16::from(area.height > 1);
    let width = area.width.saturating_sub(right_margin).min(32);
    let height = area.height.saturating_sub(top_margin).min(3);

    Rect {
        x: area
            .x
            .saturating_add(area.width.saturating_sub(width + right_margin)),
        y: area.y.saturating_add(top_margin),
        width,
        height,
    }
}

/// 在畫面右上方繪製 filter 輸入框，並回傳游標應該停留的位置。
pub(crate) fn render_filter_input(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    title: &str,
    buffer: &str,
    cursor: usize,
) -> (u16, u16) {
    render_top_right_input(frame, area, theme, title, buffer, cursor)
}

/// 在畫面右上方繪製 preview search 輸入框，並回傳游標應該停留的位置。
pub(crate) fn render_preview_search_input(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    buffer: &str,
    cursor: usize,
) -> (u16, u16) {
    render_top_right_input(frame, area, theme, " Preview Search ", buffer, cursor)
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
    cursor: usize,
    _editing: bool,
) -> (u16, u16) {
    let width = area.width.clamp(24, 40);
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
    let scrolled = compute_scrolled_input(buffer, cursor, input_inner.width as usize, None, theme);
    frame.render_widget(
        Paragraph::new(Line::from(scrolled.spans)).block(block),
        panel_area,
    );

    (
        input_inner.x.saturating_add(scrolled.cursor_col),
        input_inner.y,
    )
}

/// 在畫面底部繪製排序選單，使用緊湊且可掃讀的快捷鍵提示面板。
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

/// 在畫面底部繪製 Move 與 LineMode 快捷鍵面板，供 `m` 使用。
pub(crate) fn render_linemode_picker(frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Move / LineMode ",
        &[
            ShortcutPanelItem {
                shortcut: "m",
                label: "move to path",
            },
            ShortcutPanelItem {
                shortcut: "p",
                label: "move to panel",
            },
            ShortcutPanelItem {
                shortcut: "1..9",
                label: "move to pane <id>",
            },
            ShortcutPanelItem {
                shortcut: "s",
                label: "linemode size",
            },
            ShortcutPanelItem {
                shortcut: "r",
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

/// 在畫面底部繪製 Yank 快捷鍵面板，供 `y` 使用。
pub(crate) fn render_yank_picker(frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Yank ",
        &[
            ShortcutPanelItem {
                shortcut: "y",
                label: "copy to clipboard",
            },
            ShortcutPanelItem {
                shortcut: "p",
                label: "copy to panel",
            },
            ShortcutPanelItem {
                shortcut: "1..9",
                label: "copy to pane <id>",
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
                label: "[D]ocuments",
            },
            ShortcutPanelItem {
                shortcut: "k",
                label: "des[K]top",
            },
            ShortcutPanelItem {
                shortcut: "l",
                label: "down[L]oads",
            },
            ShortcutPanelItem {
                shortcut: "Esc",
                label: "cancel",
            },
        ],
    );
}

/// 在畫面底部繪製 `t` 系列命令面板，供主題與 Trash 命令共用。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `area: Rect`，可繪製的終端範圍。
/// - `theme: Theme`，目前使用中的主題色盤。
///
/// 回傳：`()`, 直接把快捷鍵說明畫到底部面板。
pub(crate) fn render_theme_command_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Theme / Trash ",
        &[
            ShortcutPanelItem {
                shortcut: "l",
                label: "theme list",
            },
            ShortcutPanelItem {
                shortcut: "n",
                label: "theme next",
            },
            ShortcutPanelItem {
                shortcut: "t",
                label: "trash",
            },
            ShortcutPanelItem {
                shortcut: "u",
                label: "trash undo",
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
                shortcut: "r",
                label: "resize mode",
            },
            ShortcutPanelItem {
                shortcut: "=",
                label: "equalize panes",
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
                shortcut: "t",
                label: "terminal in current directory",
            },
            ShortcutPanelItem {
                shortcut: "d",
                label: "diff comparison (all panels)",
            },
            ShortcutPanelItem {
                shortcut: "Esc",
                label: "cancel",
            },
        ],
    );
}

/// 在畫面底部繪製 panel 尺寸調整快捷鍵面板，供 `wr` 連續調整模式使用。
pub(crate) fn render_window_resize_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
) {
    render_shortcut_grid_panel(
        frame,
        area,
        theme,
        " Resize Pane ",
        &[
            ShortcutPanelItem {
                shortcut: "h / Left",
                label: "width -4",
            },
            ShortcutPanelItem {
                shortcut: "l / Right",
                label: "width +4",
            },
            ShortcutPanelItem {
                shortcut: "k / Up",
                label: "height +2",
            },
            ShortcutPanelItem {
                shortcut: "j / Down",
                label: "height -2",
            },
            ShortcutPanelItem {
                shortcut: "=",
                label: "equalize all",
            },
            ShortcutPanelItem {
                shortcut: "Esc / Enter",
                label: "done",
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
#[allow(clippy::too_many_arguments)]
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
    cursor: usize,
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

    editing.then(|| render_top_right_input(frame, popup_area, theme, "Filter", search, cursor))
}

/// 在畫面中央繪製 zoxide 目錄列表彈窗，供 `Z` 與 `:zoxide` 共用。
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_zoxide_picker(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    lines: &[ZoxidePanelLine],
    selected: usize,
    search: &str,
    editing: bool,
    cursor: usize,
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

    editing.then(|| render_top_right_input(frame, popup_area, theme, "Filter", search, cursor))
}

/// 在指定 pane 區域中央繪製命令輸入視窗，並回傳游標位置。
pub(crate) fn render_command_palette(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    theme: Theme,
    state: CommandPaletteState<'_>,
) -> (u16, u16) {
    let popup_height = (state.suggestions.len().min(6) as u16)
        .saturating_add(3)
        .max(3);
    let popup_area = centered_rect(area, 70, popup_height);
    frame.render_widget(Clear, popup_area);
    let title_text = match state.mode {
        RenameMode::Insert => " Command (insert) ",
        RenameMode::Normal => " Command (normal) ",
    };
    let block = Block::default()
        .title(Line::from(Span::styled(
            title_text,
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
    let scrolled = compute_scrolled_input(
        state.buffer,
        state.cursor,
        chunks[0].width as usize,
        Some(":"),
        theme,
    );
    frame.render_widget(Paragraph::new(Line::from(scrolled.spans)), chunks[0]);

    if !state.suggestions.is_empty() && chunks.len() > 1 {
        let items = state
            .suggestions
            .iter()
            .map(|line| {
                let text = if line.description.trim().is_empty() {
                    line.display_command.clone()
                } else {
                    format!(
                        "{:<22}  {:<8}  {}",
                        truncate_text(&line.display_command, 22),
                        line.shortcut,
                        line.description
                    )
                };
                ListItem::new(Line::from(text))
            })
            .collect::<Vec<_>>();
        let mut list_state = ListState::default();
        list_state.select(Some(
            state
                .selected
                .min(state.suggestions.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(theme.selected_item_style())
                .highlight_symbol("▶ "),
            chunks[1],
            &mut list_state,
        );
    }

    (inner.x.saturating_add(scrolled.cursor_col), inner.y)
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

/// 將 task 紀錄整理成可在窄 panel 中完整閱讀的多行內容。
///
/// 第一行只放狀態、開始／結束時間與進度；操作、來源、目的地及結果各自使用有標籤的
/// 後續行。這樣工作完成後不會因 `detail` 被結果覆寫而遺失來源與目的地，超過 panel
/// 寬度時也會繼續換行，而不是由 terminal 直接裁掉。
///
/// 參數：
/// - `task: &TaskPanelLine`，已格式化的 task 顯示資料。
/// - `max_width: usize`，panel 邊框內可用的終端顯示寬度。
///
/// 回傳：`Vec<Line<'static>>`，可直接交給單一 [`ListItem`] 顯示的多行文字。
fn task_panel_display_lines(task: &TaskPanelLine, max_width: usize) -> Vec<Line<'static>> {
    let mark = if task.marked { "* " } else { "  " };
    let summary = format!(
        "{}{:<11} start {}  end {}  {}",
        mark,
        truncate_text(&task.state, 11),
        truncate_text(&task.started_at, 8),
        truncate_text(&task.finished_at, 8),
        task.progress
    );
    let mut lines = wrap_text_for_width(&summary, max_width, "");
    lines.extend(wrap_text_for_width(
        &format!("operation: {}", task.title),
        max_width,
        "  ",
    ));

    // 多選數百或數千個檔案時，完整來源仍保存在 task-history.json；面板只展開前五筆，
    // 讓後續 task 不會被單一工作推到畫面之外。
    const MAX_VISIBLE_SOURCES: usize = 5;
    for (index, source) in task
        .source_locations
        .iter()
        .take(MAX_VISIBLE_SOURCES)
        .enumerate()
    {
        let label = if task.source_locations.len() == 1 {
            "source".to_string()
        } else {
            format!("source {}", index + 1)
        };
        lines.extend(wrap_text_for_width(
            &format!("{label}: {source}"),
            max_width,
            "  ",
        ));
    }
    if task.source_locations.len() > MAX_VISIBLE_SOURCES {
        lines.extend(wrap_text_for_width(
            &format!(
                "source: ... and {} more",
                task.source_locations.len() - MAX_VISIBLE_SOURCES
            ),
            max_width,
            "  ",
        ));
    }
    if let Some(destination) = &task.destination_location {
        lines.extend(wrap_text_for_width(
            &format!("destination: {destination}"),
            max_width,
            "  ",
        ));
    }
    if !task.detail.trim().is_empty() {
        lines.extend(wrap_text_for_width(
            &format!("result: {}", task.detail),
            max_width,
            "  ",
        ));
    }
    lines.into_iter().map(Line::from).collect()
}

/// 依終端顯示寬度切割文字，並讓每一個輸出行保留相同縮排。
///
/// 這裡使用 Unicode display width 而非 byte 或字元數，避免中文路徑在 macOS／Windows
/// terminal 中被錯算寬度。單一長路徑即使沒有空格也會硬換行，確保尾端 OS error
/// 仍能看見。
///
/// 參數：`text` 是原始文字；`max_width` 是每行可用寬度；`indent` 是每行前綴。
/// 回傳：`Vec<String>`，至少包含一行，且每行顯示寬度不超過 `max_width`。
fn wrap_text_for_width(text: &str, max_width: usize, indent: &str) -> Vec<String> {
    let max_width = max_width.max(1);
    let indent_width = UnicodeWidthStr::width(indent);
    let effective_indent = if indent_width < max_width { indent } else { "" };
    let effective_indent_width = UnicodeWidthStr::width(effective_indent);
    let content_width = max_width.saturating_sub(effective_indent_width).max(1);
    let mut output = Vec::new();

    for logical_line in text.split('\n') {
        let mut current = String::from(effective_indent);
        let mut current_width = 0usize;
        for character in logical_line.chars() {
            let character_width = character.width().unwrap_or(0);
            if current_width > 0 && current_width.saturating_add(character_width) > content_width {
                output.push(current);
                current = String::from(effective_indent);
                current_width = 0;
            }
            current.push(character);
            current_width = current_width.saturating_add(character_width);
        }
        output.push(current);
    }

    if output.is_empty() {
        output.push(String::from(effective_indent));
    }
    output
}

/// 依終端機實際顯示寬度截短單行文字，並在內容被省略時加上省略號。
///
/// Rust 的 `str::len()` 是 byte 數，`chars().count()` 是 Unicode scalar 數，兩者都不
/// 等於終端機欄寬；例如大多數中文字會佔兩格。列表若用字元數配置右側欄位，中文
/// 名稱就會把 size、permissions 等資訊推到 panel 外。這個函數逐字累加
/// [`UnicodeWidthChar`] 的欄寬，因此 macOS 與 Windows terminal 都使用同一套規則。
///
/// 參數：
/// - `text: &str`：準備顯示的原始單行文字。
/// - `max_width: usize`：文字最多可佔用的終端機欄數。
///
/// 回傳：`String`。未超寬時保留原文；超寬時保留能容納的前綴並加上 `…`；寬度為
/// 0 時回傳空字串。
fn truncate_text_to_display_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }

    const ELLIPSIS: char = '…';
    let ellipsis_width = ELLIPSIS.width().unwrap_or(1);
    if max_width <= ellipsis_width {
        return ELLIPSIS.to_string();
    }

    let content_width = max_width - ellipsis_width;
    let mut output = String::new();
    let mut used_width = 0usize;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used_width.saturating_add(character_width) > content_width {
            break;
        }
        output.push(character);
        used_width = used_width.saturating_add(character_width);
    }
    output.push(ELLIPSIS);
    output
}

/// 根據目前排序模式，產生單一列表列的顯示內容。
#[allow(clippy::too_many_arguments)]
fn render_entry_line(
    entry: &super::entry::FileEntry,
    marked: bool,
    mark_column_active: bool,
    visual_selected: bool,
    detail_kind: SortDetailKind,
    width: usize,
    theme: Theme,
    icons_enabled: bool,
    icon_style: IconStyle,
    list_find_query: Option<&str>,
    list_find_position: Option<(usize, usize)>,
    active_job_badge: Option<&str>,
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
    let icon = if icons_enabled {
        format!("{} ", entry_icon(entry, icon_style))
    } else {
        String::new()
    };
    let mut display_name = entry.display_name();
    let badge = list_find_position.map(|(current, total)| format!("[{current}/{total}]"));
    let detail = format_sort_detail(entry, detail_kind);
    let marker_width = UnicodeWidthStr::width(marker);
    let icon_width = UnicodeWidthStr::width(icon.as_str());
    let badge_width = badge
        .as_ref()
        .map(|value| UnicodeWidthStr::width(value.as_str()) + 1)
        .unwrap_or(0);
    let job_badge_width = active_job_badge
        .map(|value| UnicodeWidthStr::width(value) + 1)
        .unwrap_or(0);
    let detail_width = UnicodeWidthStr::width(detail.as_str());
    let fixed_width = marker_width
        .saturating_add(icon_width)
        .saturating_add(job_badge_width)
        .saturating_add(badge_width)
        .saturating_add(detail_width)
        .saturating_add(1);

    if detail.is_empty() || width < fixed_width {
        let mut spans = Vec::new();
        if !marker.is_empty() {
            spans.push(Span::raw(marker.to_string()));
        }
        if !icon.is_empty() {
            spans.push(Span::styled(icon, entry_style(entry, theme)));
        }
        spans.extend(highlight_name_spans(
            &display_name,
            list_find_query,
            theme,
            entry_style(entry, theme),
        ));
        if let Some(job_badge) = active_job_badge {
            spans.push(Span::raw(" ".to_string()));
            spans.push(Span::styled(
                job_badge.to_string(),
                ratatui::style::Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
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

    // 右側資訊比完整檔名更不能遺失：先保留 detail 與至少一格間距，再把剩餘寬度
    // 分配給名稱。中文或其他寬字元名稱過長時，只截短名稱，不讓 detail 被裁掉。
    let available_name_width = width.saturating_sub(fixed_width);
    display_name = truncate_text_to_display_width(&display_name, available_name_width);
    let name_width = UnicodeWidthStr::width(display_name.as_str());
    let used_width = marker_width
        .saturating_add(icon_width)
        .saturating_add(name_width)
        .saturating_add(job_badge_width)
        .saturating_add(badge_width)
        .saturating_add(detail_width);
    let spacer_len = width.saturating_sub(used_width).max(1);

    let mut spans = Vec::new();
    if !marker.is_empty() {
        spans.push(Span::raw(marker.to_string()));
    }
    if !icon.is_empty() {
        spans.push(Span::styled(icon, entry_style(entry, theme)));
    }
    spans.extend(highlight_name_spans(
        &display_name,
        list_find_query,
        theme,
        entry_style(entry, theme),
    ));
    if let Some(job_badge) = active_job_badge {
        spans.push(Span::raw(" ".to_string()));
        spans.push(Span::styled(
            job_badge.to_string(),
            ratatui::style::Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
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

/// 根據檔案種類產生列表中的圖示。
///
/// 參數：
/// - `entry: &FileEntry`，目前要顯示的檔案或資料夾。
///
/// 回傳：`&'static str`，不依賴 Nerd Font 的跨平台 Unicode 圖示。
fn entry_icon(entry: &super::entry::FileEntry, style: IconStyle) -> &'static str {
    if style == IconStyle::Ascii {
        return ascii_entry_icon(entry);
    }
    if entry.is_dir {
        return "";
    }
    match file_category(entry) {
        FileCategory::Image => "",
        FileCategory::Archive => "",
        FileCategory::Source => "",
        FileCategory::Executable => "",
        FileCategory::File => "",
    }
}

/// 產生不依賴 Nerd Font 的純 ASCII 圖示，供跨平台 fallback 使用。
fn ascii_entry_icon(entry: &super::entry::FileEntry) -> &'static str {
    if entry.is_dir {
        return "[D]";
    }
    match file_category(entry) {
        FileCategory::Image => "[I]",
        FileCategory::Archive => "[A]",
        FileCategory::Source => "[S]",
        FileCategory::Executable => "[X]",
        FileCategory::File => "[F]",
    }
}

/// 表示列表需要區分的檔案類別。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileCategory {
    File,
    Executable,
    Image,
    Archive,
    Source,
}

/// 依照平台可取得的權限與副檔名判斷檔案類別。
///
/// Windows 沒有 Unix mode bits，因此會使用常見可執行副檔名作為 fallback。
fn file_category(entry: &super::entry::FileEntry) -> FileCategory {
    let extension = entry
        .path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if entry.unix_mode.is_some_and(|mode| mode & 0o111 != 0)
        || matches!(extension.as_str(), "exe" | "com" | "bat" | "cmd" | "ps1")
    {
        return FileCategory::Executable;
    }
    if matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico"
    ) {
        return FileCategory::Image;
    }
    if matches!(
        extension.as_str(),
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "tgz"
    ) {
        return FileCategory::Archive;
    }
    if matches!(
        extension.as_str(),
        "rs" | "toml"
            | "json"
            | "yaml"
            | "yml"
            | "js"
            | "ts"
            | "py"
            | "go"
            | "c"
            | "h"
            | "cpp"
            | "java"
            | "swift"
            | "rb"
            | "sh"
    ) {
        return FileCategory::Source;
    }
    FileCategory::File
}

/// 取得檔案類別對應的主題文字樣式。
fn entry_style(entry: &super::entry::FileEntry, theme: Theme) -> ratatui::style::Style {
    if entry.is_dir {
        return ratatui::style::Style::default().fg(theme.directory);
    }
    match file_category(entry) {
        FileCategory::Executable => ratatui::style::Style::default().fg(theme.executable),
        FileCategory::Image => ratatui::style::Style::default().fg(theme.image),
        FileCategory::Archive => ratatui::style::Style::default().fg(theme.archive),
        FileCategory::Source => ratatui::style::Style::default().fg(theme.source),
        FileCategory::File => ratatui::style::Style::default(),
    }
}

/// 依照目前的 list find 查詢，把檔名切成一般片段與高亮片段。
fn highlight_name_spans(
    name: &str,
    query: Option<&str>,
    theme: Theme,
    base_style: ratatui::style::Style,
) -> Vec<Span<'static>> {
    let Some(query) = query.filter(|value| !value.is_empty()) else {
        return vec![Span::styled(name.to_string(), base_style)];
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
            spans.push(Span::styled(prefix.to_string(), base_style));
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
        spans.push(Span::styled(suffix.to_string(), base_style));
    }

    if spans.is_empty() {
        vec![Span::styled(name.to_string(), base_style)]
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
                    .directory_size
                    .map(|size| {
                        let size = format_size_short(size);
                        if entry.directory_size_complete {
                            size
                        } else {
                            format!("~{size}")
                        }
                    })
                    .unwrap_or_else(|| String::from("…"))
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

/// 把 byte 大小轉成 PaneFM 在 macOS 與 Windows 共用的 1024 進位短格式。
///
/// 參數：`size: u64` 是檔案內容的 logical bytes。
/// 回傳：`String`，使用 `B/K/M/G/T`；每一級都是前一級的 1024 倍，且這不是
/// 檔案系統的磁碟配置空間。
fn format_size_short(size: u64) -> String {
    const K: f64 = 1_024.0;
    const M: f64 = K * 1_024.0;
    const G: f64 = M * 1_024.0;
    const T: f64 = G * 1_024.0;

    let size = size as f64;
    if size >= T {
        format_compact_size(size / T, "T")
    } else if size >= G {
        format_compact_size(size / G, "G")
    } else if size >= M {
        format_compact_size(size / M, "M")
    } else if size >= K {
        format_compact_size(size / K, "K")
    } else {
        format!("{}B", size as u64)
    }
}

/// 將大小數值格式化成最多兩位小數的緊湊字串。
///
/// 參數：`value: f64` 是已換算的單位數值；`suffix: &str` 是單位。
/// 回傳：`String`；小於 10 時保留兩位，使 Finder 顯示的 `6.27 GB` 不會被過度捨入。
fn format_compact_size(value: f64, suffix: &str) -> String {
    if value.fract() == 0.0 {
        format!("{:.0}{suffix}", value)
    } else if value < 10.0 {
        let number = format!("{value:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned();
        format!("{number}{suffix}")
    } else {
        format!("{value:.1}{suffix}")
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
    warning: Option<&str>,
    theme: Theme,
    _config: &AppConfig,
) {
    let (title, question) = if permanent {
        (
            " Confirm Delete ",
            format!("Delete {target_name} permanently?"),
        )
    } else {
        (" Confirm Trash ", format!("Move {target_name} to trash?"))
    };
    let mut lines = vec![Line::from(question)];
    if let Some(warn) = warning {
        lines.push(Line::from(Span::styled(
            warn.to_string(),
            theme.danger_title_style(),
        )));
        lines.push(Line::from(
            "Press D for instant background delete, y to trash, Esc.",
        ));
    } else {
        lines.push(Line::from("Press y to confirm, n or Esc to cancel."));
    }

    let max_line_len = lines.iter().map(|l| l.width()).max().unwrap_or(40);
    let required_width = ((max_line_len as u16) + 4)
        .max(56)
        .min(area.width.saturating_sub(2));
    let required_height = ((lines.len() as u16) + 2)
        .max(5)
        .min(area.height.saturating_sub(2));

    let x = area.x + (area.width.saturating_sub(required_width)) / 2;
    let y = area.y + (area.height.saturating_sub(required_height)) / 2;
    let dialog_area = Rect::new(x, y, required_width, required_height);

    frame.render_widget(Clear, dialog_area);
    frame.render_widget(
        Paragraph::new(lines).block(
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
                    " Theme List ",
                    theme.accent_style().add_modifier(Modifier::BOLD),
                )))
                .borders(Borders::ALL),
        )
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, dialog_area, &mut list_state);
}

/// 將比對路徑格式化為固定寬度欄位，超長時保留前端與後端檔名，中段以 `…` 縮略，並對齊寬度。
pub(crate) fn format_diff_path_column(path: &str, target_width: usize) -> String {
    let current_width = UnicodeWidthStr::width(path);
    if current_width == target_width {
        return path.to_string();
    }
    if current_width < target_width {
        let padding = target_width - current_width;
        return format!("{}{}", path, " ".repeat(padding));
    }

    if target_width <= 3 {
        return "…".to_string();
    }

    // 中段縮略演算法：保留開頭目錄與結尾檔名
    let chars = path.chars().collect::<Vec<_>>();
    let keep_head = (target_width / 4).clamp(3, 18);
    let keep_tail = target_width.saturating_sub(keep_head + 1);

    let head: String = chars.iter().take(keep_head).collect();
    let tail: String = chars
        .iter()
        .skip(chars.len().saturating_sub(keep_tail))
        .collect();
    let mut combined = format!("{}…{}", head, tail);

    let mut actual_w = UnicodeWidthStr::width(combined.as_str());
    if actual_w > target_width {
        combined = truncate_text_to_display_width(&combined, target_width);
        actual_w = UnicodeWidthStr::width(combined.as_str());
    }
    if actual_w < target_width {
        combined.push_str(&" ".repeat(target_width - actual_w));
    }
    combined
}

/// 渲染全螢幕 N 路目錄與檔案差異比對工作區 (Diff Matrix Overlay)。
pub(crate) fn render_diff_matrix(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    state: &mut DiffMatrixState,
    theme: Theme,
) {
    frame.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // 頂部標題、大綱統計與篩選列
            Constraint::Min(1),    // 中央矩陣表格
            Constraint::Length(1), // 底部快捷鍵提示列
        ])
        .split(area);

    // 1. 頂部標題與篩選狀態
    let roots_title = state
        .panel_labels
        .iter()
        .enumerate()
        .map(|(idx, label)| format!("#{}: {}", idx + 1, label))
        .collect::<Vec<_>>()
        .join(" ── ");

    let header_title = format!(" [Diff Matrix] {} ", roots_title);
    let search_part = if !state.search_query.is_empty() {
        format!(" │ 搜尋: \"{}\"", state.search_query)
    } else if state.search_active {
        String::from(" │ 搜尋: [/]")
    } else {
        String::new()
    };

    let gitignore_label = if state.git_ignore {
        "啟用"
    } else {
        "停用(含target/build)"
    };
    let hidden_label = if state.include_hidden {
        "包含"
    } else {
        "排除"
    };

    let total_count = state.rows.len();
    let diff_count = state.different_count();
    let same_count = state.identical_count();

    let diff_style = if diff_count > 0 {
        theme.danger_style().add_modifier(Modifier::BOLD)
    } else {
        theme.success_style().add_modifier(Modifier::BOLD)
    };
    let same_style = theme.success_style().add_modifier(Modifier::BOLD);

    let summary_line = Line::from(vec![
        Span::styled(
            " 大綱摘要: ",
            theme.accent_style().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("不同 {} 項", diff_count), diff_style),
        Span::styled(" (內容差異/單端新檔) │ ", theme.muted_style()),
        Span::styled(format!("相同 {} 項", same_count), same_style),
        Span::styled(" (完全一致) │ ", theme.muted_style()),
        Span::styled(format!("總計 {} 項", total_count), theme.accent_style()),
        Span::styled(
            format!(" (顯示 {} 項)", state.filtered_indices.len()),
            theme.muted_style(),
        ),
    ]);

    let filter_line = Line::from(vec![
        Span::styled(" 篩選: ", theme.muted_style()),
        Span::styled(
            format!("[{}] (按 f)", state.filter_mode.label()),
            theme.accent_style(),
        ),
        Span::styled(" │ 規則: ", theme.muted_style()),
        Span::styled(
            format!("[.gitignore: {} (按 i)]", gitignore_label),
            theme.muted_style(),
        ),
        Span::styled(
            format!(" [隱藏檔: {} (按 .)]", hidden_label),
            theme.muted_style(),
        ),
        Span::styled(" [.git: 排除]", theme.muted_style()),
        Span::styled(
            search_part,
            theme.accent_style().add_modifier(Modifier::BOLD),
        ),
    ]);

    let header_block = Block::default()
        .title(Line::from(Span::styled(
            header_title,
            theme.accent_style().add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .border_style(theme.focused_border_style());

    let header_para = Paragraph::new(vec![summary_line, filter_line]).block(header_block);
    frame.render_widget(header_para, outer[0]);

    // 2. 中央矩陣表格 / 載入中狀態
    if state.loading {
        let loading_msg = if state.discovered_count > 0 {
            format!(
                " 正在非阻塞掃描目錄... 已發現 {} 個項目 (按 Esc/q 可隨時退出) ",
                state.discovered_count
            )
        } else {
            String::from(" 正在非阻塞掃描目錄中... (按 Esc/q 可隨時退出) ")
        };
        let loading_block = Block::default()
            .title(Line::from(Span::styled(
                " [掃描中] ",
                theme.accent_style().add_modifier(Modifier::BOLD),
            )))
            .borders(Borders::ALL)
            .border_style(theme.focused_border_style());
        let loading_para = Paragraph::new(Line::from(vec![
            Span::styled(" ⏳ ", theme.accent_style().add_modifier(Modifier::BOLD)),
            Span::styled(loading_msg, theme.accent_style()),
        ]))
        .block(loading_block);
        frame.render_widget(loading_para, outer[1]);

        let shortcuts = " [Esc / q] 取消並退出比對 ";
        let footer_para = Paragraph::new(Line::from(Span::styled(
            shortcuts,
            theme.accent_style().add_modifier(Modifier::BOLD),
        )));
        frame.render_widget(footer_para, outer[2]);
        return;
    }

    let panel_count = state.panel_roots.len();
    let visible_height = outer[1].height.saturating_sub(2) as usize; // 扣除上下邊框

    // 動態計算各欄位寬度以保持完美垂直對齊
    let total_width = (outer[1].width as usize).saturating_sub(4);
    let prefix_w = 4; // cursor (2) + icon (2)
    let size_w = 10;
    let panels_w = panel_count * 8;
    let status_w = 16;
    let right_fixed_w = prefix_w + size_w + panels_w + status_w;
    let path_col_width = total_width.saturating_sub(right_fixed_w).max(25);

    let total_rows = state.filtered_indices.len();
    let (view_start, view_end) = visible_list_window_range(
        total_rows,
        state.selected_index,
        visible_height,
        state.scroll_offset,
    );
    state.scroll_offset = view_start;
    let selected_pos = state.selected_index;

    let display_rows = if total_rows == 0 {
        Vec::new()
    } else {
        state.filtered_indices[view_start..view_end]
            .iter()
            .enumerate()
            .map(|(rel_idx, &row_idx)| {
                let is_selected = view_start + rel_idx == selected_pos;
                let row = &state.rows[row_idx];

                let cursor_str = if is_selected { "> " } else { "  " };
                let icon = if row.is_dir { "📁 " } else { "📄 " };
                let path_str = row.relative_path.to_string_lossy();

                let size_str = if row.is_dir {
                    String::from("DIR")
                } else {
                    format_size_short(row.display_size)
                };

                // 組合各 Panel 狀態指示（嚴格 8 個字元寬度）
                let mut panel_spans = Vec::new();
                for (p_idx, p_state) in row.panel_states.iter().enumerate() {
                    let badge = match p_state {
                        DiffEntryState::Present { .. } => match row.status {
                            DiffStatus::Identical => {
                                Span::styled(" [ ✔ ]  ", theme.success_style())
                            }
                            DiffStatus::Modified => Span::styled(
                                " [ ≠ ]  ",
                                theme.danger_style().add_modifier(Modifier::BOLD),
                            ),
                            DiffStatus::Subset => Span::styled(" [ ✔ ]  ", theme.accent_style()),
                            DiffStatus::Exclusive { panel_index } if panel_index == p_idx => {
                                Span::styled(
                                    " [ + ]  ",
                                    theme.accent_style().add_modifier(Modifier::BOLD),
                                )
                            }
                            _ => Span::styled(" [ ✔ ]  ", theme.success_style()),
                        },
                        DiffEntryState::Missing => Span::styled(" [ -- ] ", theme.muted_style()),
                    };
                    panel_spans.push(badge);
                }

                let status_span = match row.status {
                    DiffStatus::Identical => Span::styled("  完全一致", theme.success_style()),
                    DiffStatus::Modified => Span::styled(
                        "  內容不同",
                        theme.danger_style().add_modifier(Modifier::BOLD),
                    ),
                    DiffStatus::Exclusive { panel_index } => Span::styled(
                        format!("  僅 #{} 獨有", panel_index + 1),
                        theme.accent_style(),
                    ),
                    DiffStatus::Subset => Span::styled("  子集一致", theme.accent_style()),
                };

                let path_formatted = format_diff_path_column(&path_str, path_col_width);
                let size_formatted = format!("{:>8}  ", size_str);

                let mut line_spans = vec![
                    Span::styled(
                        cursor_str,
                        if is_selected {
                            theme.accent_style()
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(icon, Style::default()),
                    Span::styled(
                        path_formatted,
                        if is_selected {
                            theme.selected_item_style()
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(size_formatted, theme.muted_style()),
                ];
                line_spans.extend(panel_spans);
                line_spans.push(status_span);

                let item_style = if is_selected {
                    theme.selected_item_style()
                } else {
                    Style::default()
                };

                ListItem::new(Line::from(line_spans)).style(item_style)
            })
            .collect::<Vec<_>>()
    };

    let mut table_title_spans = vec![
        Span::styled(
            format!("  {:<w$}", "Path", w = path_col_width + 2),
            theme.accent_style(),
        ),
        Span::styled(format!("{:>8}  ", "Size"), theme.accent_style()),
    ];
    for idx in 0..panel_count {
        table_title_spans.push(Span::styled(
            format!("{:^8}", format!("#{}", idx + 1)),
            theme.accent_style(),
        ));
    }
    table_title_spans.push(Span::styled("  Status", theme.accent_style()));

    let table_block = Block::default()
        .title(Line::from(table_title_spans))
        .borders(Borders::ALL)
        .border_style(theme.focused_border_style());

    let list_widget = List::new(display_rows).block(table_block);
    frame.render_widget(list_widget, outer[1]);

    // 3. 底部快捷鍵提示列
    let shortcuts = " [Enter] 查看內容差異  [f] 篩選模式  [i] gitignore切換  [.] 隱藏檔切換  [/] 搜尋路徑  [r] 重新掃描  [q/Esc] 退出比對 ";
    let footer_para = Paragraph::new(Line::from(Span::styled(
        shortcuts,
        theme.accent_style().add_modifier(Modifier::BOLD),
    )));
    frame.render_widget(footer_para, outer[2]);
}

#[cfg(test)]
#[path = "tests/ui_test.rs"]
mod tests;
