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

use super::{app::RenameMode, pane::PaneState};

/// 描述 inline rename 視窗目前需要顯示的內容與游標位置。
///
/// 這個結構只負責把 `App` 的 rename 狀態轉交給 UI，
/// 讓繪圖函數可以知道目前文字內容、游標在哪裡，以及處於哪一種模式。
#[derive(Clone, Copy)]
pub(crate) struct InlineRenameState<'a> {
    pub(crate) buffer: &'a str,
    pub(crate) cursor: usize,
    pub(crate) mode: RenameMode,
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
/// - `rename_state: Option<InlineRenameState<'_>>`，若目前正在重新命名，這裡會帶入輸入內容、游標與模式。
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
    theme: Theme,
    rename_state: Option<InlineRenameState<'_>>,
) -> Option<(u16, u16)> {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(6)])
        .split(area);

    let border_style = if focused {
        theme.focused_border_style()
    } else {
        theme.muted_style()
    };

    let title = format!(" pane {}  {}", pane_id, pane.cwd.display());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let items: Vec<ListItem<'static>> = if pane.entries.is_empty() {
        vec![ListItem::new(Line::from("empty directory"))]
    } else {
        pane.entries
            .iter()
            .map(|entry| {
                let detail = if entry.is_dir {
                    String::from("dir")
                } else {
                    format!("{}b", entry.size)
                };
                ListItem::new(Line::from(vec![
                    Span::raw(entry.display_name()),
                    Span::styled(format!("  [{detail}]"), theme.muted_style()),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(theme.selected_item_style())
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, chunks[0], &mut pane.list_state);

    let mut rename_cursor = None;
    if let Some(state) = rename_state {
        rename_cursor = render_inline_rename(frame, chunks[0], pane, theme, state);
    }

    let preview = Paragraph::new(pane.preview_lines(4)).block(
        Block::default()
            .title("Preview")
            .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
            .border_style(border_style),
    );
    frame.render_widget(preview, chunks[1]);

    rename_cursor
}

/// 在目前選取項目下方繪製 inline rename 輸入視窗。
///
/// 參數：
/// - `frame: &mut ratatui::Frame<'_>`，目前的畫面物件。
/// - `list_area: Rect`，檔案列表所在的畫面區域。
/// - `pane: &PaneState`，目前 pane 狀態。
/// - `theme: Theme`，目前使用中的主題色盤。
/// - `state: InlineRenameState<'_>`，目前正在編輯的新名稱、游標與模式。
///
/// 回傳：`Option<(u16, u16)>`。
/// - `Some((x, y))` 代表輸入游標應該出現的位置。
/// - `None` 代表目前沒有足夠空間繪製 rename 區塊。
fn render_inline_rename(
    frame: &mut ratatui::Frame<'_>,
    list_area: Rect,
    pane: &PaneState,
    theme: Theme,
    state: InlineRenameState<'_>,
) -> Option<(u16, u16)> {
    if pane.entries.is_empty() {
        return None;
    }

    let inner = Block::default().borders(Borders::ALL).inner(list_area);
    let selected_row = inner.y.saturating_add(pane.selected as u16);
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
            match state.mode {
                RenameMode::Insert => " Rename (insert): ",
                RenameMode::Normal => " Rename (normal): ",
            },
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
        config.confirm_dialog_width_percent,
        config.confirm_dialog_height,
    );
    frame.render_widget(Clear, dialog_area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("Delete {target_name}?")),
            Line::from("Press y to confirm, n or Esc to cancel."),
        ])
        .block(
            Block::default()
                .title(Line::from(Span::styled(
                    " Confirm Delete ",
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
        config.theme_picker_width_percent,
        config.theme_picker_height,
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
