use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    file_manager::{app::RenameMode, pane::PaneState},
    theme::Theme,
};

use super::{
    dialogs::centered_rect,
    text::{compute_scrolled_input, truncate_text},
    types::{CommandPaletteState, InlineEditorState, InlinePickerState},
};

/// 在列表區域中繪製 inline 輸入視窗，供 rename / create 這類功能重用。
pub(crate) fn render_inline_editor(
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
pub(crate) fn render_inline_picker(
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

/// 在右上角繪製浮動輸入框，支援平滑水平滑動與中文字元保護。
pub(crate) fn render_top_right_input(
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
pub(crate) fn top_right_input_rect(area: Rect) -> Rect {
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
