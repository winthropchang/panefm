use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::{
    file_manager::diff::{DiffEntryState, DiffMatrixState, DiffStatus},
    theme::Theme,
};

use super::{
    pane::visible_list_window_range,
    text::{format_diff_path_column, format_size_short},
};

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
                let icon = if row.is_dir { " " } else { " " };
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
