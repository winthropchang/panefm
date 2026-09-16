use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    config::IconStyle,
    file_manager::{entry::FileEntry, pane::SortDetailKind, vcs::VcsFileStatus},
    theme::Theme,
};

use super::{
    text::{format_sort_detail, truncate_text_to_display_width},
    types::FileCategory,
};

/// 根據目前排序模式，產生單一列表列的顯示內容。
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_entry_line(
    entry: &FileEntry,
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
    jump_label: Option<char>,
    vcs_status: Option<VcsFileStatus>,
) -> Line<'static> {
    let (marker, jump_span) = if let Some(ch) = jump_label {
        let tag = format!("[{ch}] ");
        let style = Style::default()
            .fg(ratatui::style::Color::Yellow)
            .add_modifier(Modifier::BOLD);
        ("", Some(Span::styled(tag, style)))
    } else if mark_column_active {
        if marked || visual_selected {
            ("[*] ", None)
        } else {
            ("    ", None)
        }
    } else {
        ("", None)
    };
    let (vcs_span, vcs_width) = if let Some(status) = vcs_status {
        let text = format!("{} ", status.badge_char());
        let style = Style::default()
            .fg(status.color(&theme))
            .add_modifier(Modifier::BOLD);
        (Some(Span::styled(text, style)), 2usize)
    } else {
        (None, 0usize)
    };
    let base_entry_style = if jump_label.is_some() {
        theme.muted_style()
    } else {
        entry_style(entry, theme)
    };
    let icon = if icons_enabled {
        format!("{} ", entry_icon(entry, icon_style))
    } else {
        String::new()
    };
    let mut display_name = entry.display_name();
    let badge = list_find_position.map(|(current, total)| format!("[{current}/{total}]"));
    let detail = format_sort_detail(entry, detail_kind);
    let marker_width = if jump_label.is_some() {
        4
    } else {
        UnicodeWidthStr::width(marker)
    };
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
        .saturating_add(vcs_width)
        .saturating_add(icon_width)
        .saturating_add(job_badge_width)
        .saturating_add(badge_width)
        .saturating_add(detail_width)
        .saturating_add(1);

    if detail.is_empty() || width < fixed_width {
        let mut spans = Vec::new();
        if let Some(j_span) = jump_span {
            spans.push(j_span);
        } else if !marker.is_empty() {
            spans.push(Span::raw(marker.to_string()));
        }
        if let Some(v_span) = vcs_span {
            spans.push(v_span);
        }
        if !icon.is_empty() {
            spans.push(Span::styled(icon, base_entry_style));
        }
        spans.extend(highlight_name_spans(
            &display_name,
            list_find_query,
            theme,
            base_entry_style,
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
        .saturating_add(vcs_width)
        .saturating_add(icon_width)
        .saturating_add(name_width)
        .saturating_add(job_badge_width)
        .saturating_add(badge_width)
        .saturating_add(detail_width);
    let spacer_len = width.saturating_sub(used_width).max(1);

    let mut spans = Vec::new();
    if let Some(j_span) = jump_span {
        spans.push(j_span);
    } else if !marker.is_empty() {
        spans.push(Span::raw(marker.to_string()));
    }
    if let Some(v_span) = vcs_span {
        spans.push(v_span);
    }
    if !icon.is_empty() {
        spans.push(Span::styled(icon, base_entry_style));
    }
    spans.extend(highlight_name_spans(
        &display_name,
        list_find_query,
        theme,
        base_entry_style,
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
pub(crate) fn entry_icon(entry: &FileEntry, style: IconStyle) -> &'static str {
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
pub(crate) fn ascii_entry_icon(entry: &FileEntry) -> &'static str {
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

/// 依照平台可取得的權限與副檔名判斷檔案類別。
pub(crate) fn file_category(entry: &FileEntry) -> FileCategory {
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
pub(crate) fn entry_style(entry: &FileEntry, theme: Theme) -> ratatui::style::Style {
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
pub(crate) fn highlight_name_spans(
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
pub(crate) fn highlight_match_style(theme: Theme) -> Style {
    theme
        .accent_style()
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}
