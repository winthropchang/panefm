use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use crate::{
    config::AppConfig,
    theme::{Theme, ThemePreset},
};

use super::{
    dialogs::centered_rect,
    input::render_top_right_input,
    text::truncate_text,
    types::{BookmarkPanelLine, ShortcutPanelItem, ZoxidePanelLine},
};

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
pub(crate) fn render_shortcut_grid_panel(
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
pub(crate) fn shortcut_panel_lines(
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
pub(crate) fn shortcut_panel_item_width(item: &ShortcutPanelItem<'_>) -> usize {
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

/// 繪製主題選擇視窗。
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
