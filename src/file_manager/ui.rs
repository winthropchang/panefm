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
    app::TrashConfirmAction,
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
    frame.render_widget(
        Paragraph::new(buffer.to_string()).block(input_block),
        input_area,
    );

    (
        input_inner.x.saturating_add(
            cursor
                .min(buffer.chars().count())
                .min(input_inner.width as usize) as u16,
        ),
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
    frame.render_widget(Paragraph::new(buffer.to_string()).block(block), panel_area);

    (
        input_inner.x.saturating_add(
            cursor
                .min(buffer.chars().count())
                .min(input_inner.width as usize) as u16,
        ),
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
    buffer: &str,
    suggestions: &[CommandSuggestionLine],
    selected: usize,
    cursor: usize,
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
                cursor
                    .min(buffer.chars().count())
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
mod tests {
    use super::{
        FileCategory, IconStyle, SearchListState, TaskPanelLine, entry_icon, file_category,
        format_diff_path_column, format_pane_title, format_permissions_detail, format_size_short,
        format_sort_detail, regex_rename_status_style, render_entry_line, search_empty_message,
        search_list_selected_index, task_panel_display_lines, top_right_input_rect,
        truncate_text_to_display_width, visible_list_window_range,
    };
    use ratatui::layout::Rect;
    use std::path::Path;
    use std::time::SystemTime;
    use unicode_width::UnicodeWidthStr;

    use crate::file_manager::entry::FileEntry;
    use crate::file_manager::pane::SortDetailKind;
    use crate::file_manager::search::GlobalSearchEntry;
    use crate::theme::Theme;

    /// 建立 UI 格式測試使用的最小 FileEntry，避免測試依賴實際檔案系統 metadata。
    fn test_entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: Path::new(name).to_path_buf(),
            is_dir,
            size: 0,
            directory_size: None,
            directory_size_complete: false,
            modified: SystemTime::UNIX_EPOCH,
            created: SystemTime::UNIX_EPOCH,
            readonly: false,
            unix_mode: None,
        }
    }

    #[test]
    /// 驗證當項目處於背景傳輸或處理中時，列表列會直接顯示工作標籤與百分比。
    fn render_entry_line_displays_active_job_badge() {
        let entry = test_entry("terminal-file-manager", true);
        let line = render_entry_line(
            &entry,
            false,
            false,
            false,
            SortDetailKind::None,
            60,
            Theme::from(crate::theme::ThemePreset::Dracula),
            true,
            IconStyle::NerdFont,
            None,
            None,
            Some("[copying 99%]"),
        );
        let text = line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(
            text.contains("[copying 99%]"),
            "列表列必須包含工作進度標籤: {text}"
        );
    }

    #[test]
    /// 驗證中文目錄名稱會依終端顯示寬度截短，而不是把 linemode 資訊推到 panel 外。
    ///
    /// 保護目的：一個中文字通常佔兩個終端欄位。若排版誤用 `chars().count()`，size
    /// 與 permissions 看似沒有資料，實際上是被超寬名稱裁掉。測試同時覆蓋兩種右側
    /// 資訊，確保所有 `m` 選項共用的列表排版都保留完整 detail。
    fn render_entry_line_keeps_details_visible_after_wide_chinese_name() {
        let theme = Theme::from(crate::theme::ThemePreset::Dracula);
        let mut entry = test_entry("Ch02 單層感知器的數學原理與實作入門", true);
        entry.directory_size = Some(6 * 1_024);
        entry.directory_size_complete = true;
        entry.unix_mode = Some(0o755);

        for (detail_kind, expected_suffix) in [
            (SortDetailKind::Size, "6K"),
            (SortDetailKind::Permissions, "drwxr-xr-x"),
        ] {
            let line = render_entry_line(
                &entry,
                false,
                false,
                false,
                detail_kind,
                32,
                theme,
                false,
                IconStyle::Ascii,
                None,
                None,
                None,
            );
            let text = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();

            assert_eq!(UnicodeWidthStr::width(text.as_str()), 32);
            assert!(
                text.ends_with(expected_suffix),
                "右側 linemode 資訊不可被中文名稱裁掉: {text}"
            );
        }
    }

    #[test]
    /// 驗證顯示寬度截短函數不會把中文誤當成單欄字元。
    ///
    /// 保護目的：此函數是列表名稱與右側資訊正確對齊的基礎；未來調整圖示或樣式時，
    /// 仍須確保輸出寬度不超過限制，且被截短時有明確省略號。
    fn truncate_text_uses_terminal_width_for_chinese() {
        let truncated = truncate_text_to_display_width("中文目錄abcdef", 8);

        assert_eq!(truncated, "中文目…");
        assert_eq!(UnicodeWidthStr::width(truncated.as_str()), 7);
    }

    #[test]
    /// 驗證大型目錄只會渲染 viewport 內的少量項目，而不是替完整列表建立 widget。
    ///
    /// 保護目的：`target/debug/deps` 類型的目錄可能包含上萬個檔案。若一次 j/k 仍建立
    /// 10,000 個 ListItem，游標操作會明顯停頓；此測試固定要求 10,000 筆資料只選出
    /// 20 列，並確保游標離開舊畫面時 viewport 會跟隨但仍包含目前項目。
    fn large_list_window_only_contains_visible_rows() {
        let (start, end) = visible_list_window_range(10_000, 5_432, 20, 0);

        assert_eq!(end - start, 20);
        assert!(start <= 5_432 && 5_432 < end);
        assert_eq!((start, end), (5_413, 5_433));
    }

    #[test]
    /// 驗證游標仍在原 viewport 內時保持畫面起點，避免每按一次 j/k 整頁就跟著跳動。
    ///
    /// 保護目的：虛擬化不能只追求速度，也必須保留一般檔案管理器穩定的捲動體驗。
    fn list_window_keeps_previous_start_while_selection_is_visible() {
        assert_eq!(visible_list_window_range(10_000, 510, 30, 500), (500, 530));
        assert_eq!(visible_list_window_range(10_000, 499, 30, 500), (499, 529));
    }

    #[test]
    /// 驗證右上角輸入框會使用傳入 Panel 的座標，而不是退回整個 terminal 的右上角。
    /// 保護目的：避免多 Panel 重構後，Find／Filter UI 再次脫離 active Panel。
    fn top_right_input_rect_is_relative_to_owning_panel() {
        let panel = Rect::new(40, 2, 38, 18);

        let popup = top_right_input_rect(panel);

        assert_eq!(popup, Rect::new(45, 3, 32, 3));
        assert!(popup.x >= panel.x);
        assert!(popup.right() <= panel.right());
        assert!(popup.bottom() <= panel.bottom());
    }

    #[test]
    /// 驗證 Panel 小於一般輸入框寬度時，輸入框會縮小並完整留在 Panel 內。
    /// 保護目的：避免四分割或更小版面中，Filter 邊框與文字覆蓋相鄰 Panel。
    fn top_right_input_rect_shrinks_inside_narrow_panel() {
        let panel = Rect::new(12, 4, 10, 2);

        let popup = top_right_input_rect(panel);

        assert_eq!(popup, Rect::new(12, 5, 9, 1));
        assert!(popup.right() <= panel.right());
        assert!(popup.bottom() <= panel.bottom());
    }

    #[test]
    /// 驗證 task 的完整目的路徑會移到摘要下方，並依窄 panel 寬度換成多行。
    ///
    /// 參數：無。
    /// 回傳：無；若目的地尾端被裁掉，或任一行超出 panel 寬度，測試失敗。
    /// 保護目的：大型 copy 最重要的診斷資訊位於 detail 尾端；過去與固定欄位塞在
    /// 同一行，只看得到 `destination: /`，無法判斷工作實際寫到哪個目錄。
    fn task_detail_wraps_without_losing_the_destination_tail() {
        let task = TaskPanelLine {
            state: String::from("RUNNING"),
            started_at: String::from("14:30:47"),
            finished_at: String::from("--:--:--"),
            progress: String::from("24.4G / 77.2G"),
            title: String::from("copy 1 item(s)"),
            source_locations: vec![String::from(
                "/Users/otto/Documents/source/large-archive.zip",
            )],
            destination_location: Some(String::from(
                "/Users/otto/Documents/AB_Demo/very-long-target-directory",
            )),
            detail: String::from("pasted copy: 1 item"),
            marked: false,
        };

        let rendered = task_panel_display_lines(&task, 32);
        let rendered_text = rendered.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert!(rendered_text.len() >= 4);
        let source_start = rendered_text
            .iter()
            .position(|line| line.starts_with("  source:"))
            .expect("source must start on its own indented line");
        let destination_start = rendered_text
            .iter()
            .position(|line| line.starts_with("  destination:"))
            .expect("destination must start on its own indented line");
        let result_start = rendered_text
            .iter()
            .position(|line| line.starts_with("  result:"))
            .expect("result must start on its own indented line");
        let reconstructed_source = rendered_text[source_start..destination_start]
            .iter()
            .map(|line| line.trim_start())
            .collect::<String>();
        let reconstructed_destination = rendered_text[destination_start..result_start]
            .iter()
            .map(|line| line.trim_start())
            .collect::<String>();
        assert!(reconstructed_source.contains("large-archive.zip"));
        assert!(reconstructed_destination.contains("very-long-target-directory"));
        assert!(
            rendered_text
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= 32)
        );
    }

    #[test]
    /// 驗證多選工作只在面板展開前五個來源，並清楚顯示尚有多少來源未展開。
    ///
    /// 保護目的：完整來源必須寫進歷史供診斷，但一次操作數百個檔案時不能讓單一 task
    /// 佔滿整個 panel；此測試固定 UI 摘要與持久化資料必須彼此獨立。
    fn task_panel_summarizes_large_source_batches_without_losing_the_count() {
        let task = TaskPanelLine {
            state: String::from("RUNNING"),
            started_at: String::from("10:00:00"),
            finished_at: String::from("--:--:--"),
            progress: String::from("1M / 8M"),
            title: String::from("move 8 item(s)"),
            source_locations: (1..=8)
                .map(|index| format!("/source/file-{index}.txt"))
                .collect(),
            destination_location: Some(String::from("/destination")),
            detail: String::from("running"),
            marked: false,
        };

        let rendered = task_panel_display_lines(&task, 80)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("source 5: /source/file-5.txt"));
        assert!(rendered.contains("source: ... and 3 more"));
        assert!(!rendered.contains("file-6.txt"));
        assert!(rendered.contains("destination: /destination"));
    }

    #[test]
    /// 驗證搜尋仍在背景載入時，只要已有結果就不會再用 Loading 訊息蓋住列表。
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
    fn search_results_are_visible_while_loading() {
        let entry = GlobalSearchEntry {
            path: Path::new("result.txt").to_path_buf(),
            relative_path: String::from("result.txt"),
            is_dir: false,
            match_line_number: Some(1),
            match_column: Some(1),
            match_preview: Some(String::from("887")),
        };
        let results = vec![entry];
        let state = SearchListState {
            results: &results,
            selected: 0,
            loading: true,
            preview_query: None,
            preview_scroll: None,
            preview_current_match: None,
        };

        assert_eq!(search_empty_message(&state), None);
        assert_eq!(search_list_selected_index(&state), Some(0));
    }

    #[test]
    /// 驗證背景搜尋仍在回傳資料時，列表會立即顯示目前游標，而不是等待 Done 事件。
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
    fn search_cursor_is_visible_and_clamped_while_loading() {
        let results = vec![
            GlobalSearchEntry {
                path: Path::new("first.txt").to_path_buf(),
                relative_path: String::from("first.txt"),
                is_dir: false,
                match_line_number: None,
                match_column: None,
                match_preview: None,
            },
            GlobalSearchEntry {
                path: Path::new("second.txt").to_path_buf(),
                relative_path: String::from("second.txt"),
                is_dir: false,
                match_line_number: None,
                match_column: None,
                match_preview: None,
            },
        ];
        let state = SearchListState {
            results: &results,
            selected: 99,
            loading: true,
            preview_query: None,
            preview_scroll: None,
            preview_current_match: None,
        };

        assert_eq!(search_list_selected_index(&state), Some(1));
    }

    #[test]
    /// 驗證 regex 預覽右側狀態會依 ready、unchanged 與錯誤類型套用主題顏色。
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
    fn regex_rename_status_uses_theme_semantic_colors() {
        let theme = Theme::default_theme();
        assert_eq!(
            regex_rename_status_style(theme, "ready").fg,
            Some(theme.executable)
        );
        assert_eq!(
            regex_rename_status_style(theme, "unchanged").fg,
            Some(theme.muted)
        );
        assert_eq!(
            regex_rename_status_style(theme, "conflict").fg,
            Some(theme.danger)
        );
        assert_eq!(
            regex_rename_status_style(theme, "invalid").fg,
            Some(theme.danger)
        );
    }

    #[test]
    /// 驗證列表會依照目錄與常見副檔名分辨檔案類別與圖示。
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
    fn entry_kind_uses_cross_platform_categories() {
        assert_eq!(
            entry_icon(&test_entry("src", true), IconStyle::Ascii),
            "[D]"
        );
        assert_eq!(
            file_category(&test_entry("main.rs", false)),
            FileCategory::Source
        );
        assert_eq!(
            file_category(&test_entry("photo.png", false)),
            FileCategory::Image
        );
        assert_eq!(
            file_category(&test_entry("backup.zip", false)),
            FileCategory::Archive
        );
        assert_eq!(
            file_category(&test_entry("tool.exe", false)),
            FileCategory::Executable
        );
    }

    #[test]
    /// 驗證大小格式會轉成人類較容易閱讀的單位顯示。
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
    fn format_size_short_uses_compact_units() {
        assert_eq!(format_size_short(512), "512B");
        assert_eq!(format_size_short(2_048), "2K");
        assert_eq!(format_size_short(1_572_864), "1.5M");
        assert_eq!(format_size_short(6_270_192_614), "5.84G");
    }

    #[test]
    /// 驗證 size linemode 會區分背景計算中的部分容量與已完成的真實容量。
    ///
    /// 保護目的：使用者必須知道數字是否還會增加；掃描中以 `~` 標示，完成後同一列
    /// 應只留下最終大小，不能退回舊版的直接子項目數量。
    fn size_detail_marks_partial_directory_size_until_scan_completes() {
        let mut entry = test_entry("target", true);
        entry.directory_size = Some(1_572_864);

        assert_eq!(format_sort_detail(&entry, SortDetailKind::Size), "~1.5M");
        entry.directory_size_complete = true;
        assert_eq!(format_sort_detail(&entry, SortDetailKind::Size), "1.5M");
    }

    #[test]
    /// 驗證 pane 標題會把固定 pane 編號顯示在最前面，方便對照快捷鍵切換。
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
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
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
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
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
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
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
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
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
    fn format_pane_title_supports_linemode_status() {
        let title = format_pane_title(5, Path::new("/tmp/demo"), "", "", "", "linemode: size", 80);

        assert_eq!(title, "panel #5 /tmp/demo [linemode: size]");
    }

    #[test]
    /// 驗證非 Unix 平台或缺少 mode bits 時，permissions 仍有可讀的 fallback 顯示。
    /// 保護目的：避免畫面格式或主題重構後，造成狹窄 panel、選取狀態或語意顏色顯示錯誤。
    fn format_permissions_detail_falls_back_to_cross_platform_text() {
        let entry = FileEntry {
            name: String::from("notes.txt"),
            path: Path::new("/tmp/notes.txt").to_path_buf(),
            is_dir: false,
            size: 12,
            directory_size: None,
            directory_size_complete: false,
            modified: SystemTime::UNIX_EPOCH,
            created: SystemTime::UNIX_EPOCH,
            readonly: true,
            unix_mode: None,
        };

        assert_eq!(format_permissions_detail(&entry), "file readonly");
    }

    #[test]
    fn format_diff_path_column_truncates_and_pads_to_exact_width() {
        let short = "src/main.rs";
        let formatted = format_diff_path_column(short, 20);
        assert_eq!(UnicodeWidthStr::width(formatted.as_str()), 20);
        assert!(formatted.starts_with("src/main.rs"));

        let long =
            ".creator/asset-template/typescript/Custom Script Template Help Documentation.url";
        let formatted_long = format_diff_path_column(long, 35);
        assert_eq!(UnicodeWidthStr::width(formatted_long.as_str()), 35);
        assert!(formatted_long.contains('…'));
        assert!(formatted_long.starts_with(".cre"));
        assert!(formatted_long.ends_with(".url"));
    }
}
