use std::collections::BTreeMap;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

use crate::file_manager::{
    fuzzy::fuzzy_matched_indices,
    pane::FilterMode,
    search::GlobalSearchEntry,
    tools::external_tool_statuses,
    ui::{
        CommandPaletteState, render_command_palette, render_filter_input,
        render_global_search_panel, render_preview_search_input,
    },
};

use super::{
    render_dialogs::render_pending_action_overlay,
    render_pane::render_single_pane,
    state::{App, PendingAction},
    status::{status_area_height, status_is_error, status_shortcut_line, wrap_status_text},
};

impl App {
    /// 根據目前應用程式狀態繪製整個畫面。
    ///
    /// 繪製前會先依 terminal cell 寬度切割狀態文字，再動態計算 status area 高度。
    /// 這讓一般通知仍只占一行，而貼上失敗的完整 destination 與 OS error 可以依實際
    /// 長度展開成多行，不會因終端視窗較窄而遺失錯誤尾端。
    ///
    /// 參數：
    /// - `self: &mut App`，提供目前 panel、輸入模式、狀態文字及 theme 等畫面狀態。
    /// - `frame: &mut ratatui::Frame<'_>`，ratatui 本次更新可使用的繪圖 frame。
    ///
    /// 回傳：`Option<(u16, u16)>`；畫面需要顯示文字輸入游標時回傳其 cell 座標，
    /// 否則回傳 `None`。畫面內容會直接寫入傳入的 `frame`。
    pub(crate) fn render(&mut self, frame: &mut ratatui::Frame<'_>) -> Option<(u16, u16)> {
        let raw_status_text = if self.command_mode {
            format!(":{}", self.command_buffer)
        } else {
            self.status.clone()
        };
        let status_text = wrap_status_text(&raw_status_text, frame.area().width);
        let status_style = if self.command_mode {
            Style::default()
        } else if status_is_error(&raw_status_text) {
            self.theme.danger_style()
        } else {
            Style::default()
        };
        let status_height = status_area_height(&status_text, frame.area().height.saturating_sub(3));
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(status_height),
            ])
            .split(frame.area());

        self.latest_pane_area = Some(outer[0]);
        let mut pane_rects = BTreeMap::new();
        self.layout.render_rects(outer[0], &mut pane_rects);
        // PATH 掃描只在 dependency 面板真正顯示時執行。一般檔案列表每幀都重查四個
        // 外部命令不但沒有畫面用途，也會讓鍵盤回應時間受磁碟與網路 PATH 影響。
        let tool_statuses = matches!(self.pending_action, Some(PendingAction::ToolPanel { .. }))
            .then(external_tool_statuses)
            .unwrap_or_default();
        let mut cursor_position = None;
        for (&pane_id, &rect) in &pane_rects {
            let pane_cursor = render_single_pane(self, frame, pane_id, rect, &tool_statuses);
            if cursor_position.is_none() {
                cursor_position = pane_cursor;
            }
        }

        let shortcut_hints = self.active_status_shortcut_hints();
        let help = Paragraph::new(status_shortcut_line(
            outer[1].width,
            self.theme,
            &shortcut_hints,
        ))
        .block(Block::default().borders(Borders::TOP));
        frame.render_widget(help, outer[1]);

        frame.render_widget(Paragraph::new(status_text).style(status_style), outer[2]);

        if self.command_mode
            && let Some(area) = pane_rects.get(&self.focused_pane)
        {
            let command_suggestions = self.command_suggestions();
            let command_cursor = render_command_palette(
                frame,
                *area,
                self.theme,
                CommandPaletteState {
                    buffer: &self.command_buffer,
                    suggestions: &command_suggestions,
                    selected: self.command_suggestion_selected,
                    cursor: self.text_input_cursor,
                    mode: self.text_input_mode,
                },
            );
            if cursor_position.is_none() {
                cursor_position = Some(command_cursor);
            }
        }

        if let Some(filter) = &self.filter
            && filter.editing
            && let Some(area) = pane_rects.get(&filter.pane_id)
        {
            let title = match filter.mode {
                FilterMode::Normal => " Filter [Normal] (Tab: Fuzzy) ",
                FilterMode::Fuzzy => " Filter [Fuzzy] (Tab: Normal) ",
            };
            let filter_cursor = render_filter_input(
                frame,
                *area,
                self.theme,
                title,
                &filter.buffer,
                self.text_input_cursor,
            );
            if cursor_position.is_none() {
                cursor_position = Some(filter_cursor);
            }
        }

        if let Some(search) = &self.preview_search
            && search.editing
            && let Some(area) = pane_rects.get(&search.pane_id)
        {
            let search_cursor = render_preview_search_input(
                frame,
                *area,
                self.theme,
                &search.buffer,
                self.text_input_cursor,
            );
            if cursor_position.is_none() {
                cursor_position = Some(search_cursor);
            }
        }

        if let Some(search) = &self.global_search
            && let Some(area) = pane_rects.get(&search.pane_id)
        {
            if search.editing {
                let search_cursor = render_global_search_panel(
                    frame,
                    *area,
                    self.theme,
                    search.mode.panel_title(true),
                    &search.buffer,
                    self.text_input_cursor,
                    true,
                );
                if cursor_position.is_none() {
                    cursor_position = Some(search_cursor);
                }
            } else if search.filter.editing {
                let filter_cursor = render_filter_input(
                    frame,
                    *area,
                    self.theme,
                    " Filter Results ",
                    &search.filter.buffer,
                    self.text_input_cursor,
                );
                if cursor_position.is_none() {
                    cursor_position = Some(filter_cursor);
                }
            }
        }

        render_pending_action_overlay(self, frame, &pane_rects, &mut cursor_position);

        if let Some((x, y)) = cursor_position {
            frame.set_cursor_position((x, y));
        }
        cursor_position
    }
}

/// 以共用模糊 matcher 過濾 `fd` 或 `rg` 已回傳的搜尋結果。
pub(crate) fn filtered_global_search_entries(
    entries: &[GlobalSearchEntry],
    query: &str,
) -> Vec<GlobalSearchEntry> {
    fuzzy_matched_indices(entries, query, |entry| entry.relative_path.clone().into())
        .into_iter()
        .map(|index| entries[index].clone())
        .collect()
}
