use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
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

/// 繪製貼上衝突確認選單視窗。
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_paste_overwrite_dialog(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    target_name: &str,
    current_index: usize,
    total_conflicts: usize,
    selected_option: usize,
    theme: Theme,
) {
    let is_multi = total_conflicts > 1;
    let title = if is_multi {
        format!(
            " File Conflict ({} of {}) ",
            current_index + 1,
            total_conflicts
        )
    } else {
        String::from(" File Conflict ")
    };

    const SINGLE_CHOICES: [crate::file_manager::app::PasteConflictChoice; 4] = [
        crate::file_manager::app::PasteConflictChoice::Overwrite,
        crate::file_manager::app::PasteConflictChoice::AutoRename,
        crate::file_manager::app::PasteConflictChoice::Skip,
        crate::file_manager::app::PasteConflictChoice::Cancel,
    ];
    const MULTI_CHOICES: [crate::file_manager::app::PasteConflictChoice; 7] = [
        crate::file_manager::app::PasteConflictChoice::Overwrite,
        crate::file_manager::app::PasteConflictChoice::OverwriteAll,
        crate::file_manager::app::PasteConflictChoice::AutoRename,
        crate::file_manager::app::PasteConflictChoice::AutoRenameAll,
        crate::file_manager::app::PasteConflictChoice::Skip,
        crate::file_manager::app::PasteConflictChoice::SkipAll,
        crate::file_manager::app::PasteConflictChoice::Cancel,
    ];

    let choices: &[crate::file_manager::app::PasteConflictChoice] = if is_multi {
        &MULTI_CHOICES[..]
    } else {
        &SINGLE_CHOICES[..]
    };

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "Target already exists: ",
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("\"{target_name}\""),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    for (idx, choice) in choices.iter().enumerate() {
        let is_selected = idx == selected_option;
        let cursor_str = if is_selected { "> " } else { "  " };
        let hotkey_tag = format!("[{}]", choice.hotkey_display());
        let hotkey_str = format!("{:<6}", hotkey_tag);
        let label_str = format!("{:<16}", choice.label());
        let desc_str = choice.description();

        let line = if is_selected {
            Line::from(vec![
                Span::styled(
                    cursor_str,
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    hotkey_str,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    label_str,
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {desc_str}"),
                    Style::default()
                        .fg(theme.selection_fg)
                        .bg(theme.selection_bg),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(cursor_str, Style::default().fg(theme.muted)),
                Span::styled(hotkey_str, Style::default().fg(Color::Yellow)),
                Span::styled(label_str, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {desc_str}"), Style::default().fg(theme.muted)),
            ])
        };
        lines.push(line);
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Use ↑/↓ or j/k to select, Enter to confirm, or press hotkey directly.",
        Style::default().fg(theme.muted),
    )));

    let content_height = (lines.len() as u16) + 2;
    let required_height = content_height.min(area.height.saturating_sub(2)).max(8);
    let required_width = 74u16.min(area.width.saturating_sub(2)).max(50);

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
