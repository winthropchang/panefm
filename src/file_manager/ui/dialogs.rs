use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{config::AppConfig, file_manager::app::TrashConfirmAction, theme::Theme};

/// 計算在指定區域內水平與垂直置中的矩形區域。
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
