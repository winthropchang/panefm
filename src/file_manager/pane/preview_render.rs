use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::theme::Theme;

pub(crate) fn build_search_preview_lines(
    path: &Path,
    viewport_height: usize,
    query: &str,
    current_match_line: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let snippet = read_search_snippet(
        path,
        query,
        current_match_line,
        viewport_height.max(1),
        theme,
    );
    if snippet.is_empty() {
        vec![Line::from("no matching snippet available")]
    } else {
        snippet
    }
}

/// 讀取命中行附近的片段內容，讓搜尋 preview 能直接顯示上下文。
fn read_search_snippet(
    path: &Path,
    query: &str,
    current_match_line: usize,
    snippet_height: usize,
    theme: Theme,
) -> Vec<Line<'static>> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let context_before = snippet_height.saturating_sub(1) / 2;
    let context_after = snippet_height.saturating_sub(context_before + 1);
    let start_line = current_match_line.saturating_sub(context_before).max(1);
    let end_line = current_match_line.saturating_add(context_after);

    reader
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            if line_number < start_line || line_number > end_line {
                return None;
            }
            let Ok(line) = line else {
                return None;
            };
            let numbered = format!("{:>3} {}", line_number, line);
            let current_match_start = if line_number == current_match_line {
                numbered.to_lowercase().find(&query.to_lowercase())
            } else {
                None
            };
            Some(highlight_preview_line(
                Line::from(numbered),
                &query.to_lowercase(),
                theme,
                line_number == current_match_line,
                current_match_start,
            ))
        })
        .collect()
}

/// 將命中的搜尋字串套用到 preview 行內容上，讓目前查詢結果更容易辨識。
pub(crate) fn highlight_preview_matches(
    lines: Vec<Line<'static>>,
    query: &str,
    theme: Theme,
    current_match_index: Option<usize>,
) -> Vec<Line<'static>> {
    let lower_query = query.to_lowercase();
    if lower_query.is_empty() {
        return lines;
    }

    let match_positions = preview_match_positions(&lines, &lower_query);
    let current_match = current_match_index.and_then(|index| match_positions.get(index).copied());

    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let text = line.to_string();
            if !is_preview_searchable_line(&text) {
                return line;
            }
            highlight_preview_line(
                line,
                &lower_query,
                theme,
                current_match
                    .map(|(line_index, _)| line_index == index)
                    .unwrap_or(false),
                current_match
                    .filter(|(line_index, _)| *line_index == index)
                    .map(|(_, start)| start),
            )
        })
        .collect()
}

/// 計算 preview 中每一個搜尋命中的實際位置。
pub(crate) fn preview_match_positions(
    lines: &[Line<'static>],
    lower_query: &str,
) -> Vec<(usize, usize)> {
    if lower_query.is_empty() {
        return Vec::new();
    }

    let mut positions = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let text = line.to_string();
        if !is_preview_searchable_line(&text) {
            continue;
        }
        let lower_text = text.to_lowercase();
        let mut cursor = 0usize;

        while cursor <= lower_text.len() {
            let Some(found) = lower_text
                .get(cursor..)
                .and_then(|segment| segment.find(lower_query))
            else {
                break;
            };
            let start = cursor + found;
            positions.push((line_index, start));
            cursor = start.saturating_add(lower_query.len().max(1));
        }
    }

    positions
}

/// 判斷這一行是否屬於 preview 中真正可搜尋的內容區。
fn is_preview_searchable_line(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("Information")
        || trimmed.starts_with("path:")
        || trimmed.starts_with("size:")
        || trimmed.starts_with("modified:")
        || trimmed.starts_with("[Directories do not support")
        || trimmed.starts_with("[No VCS modifications detected")
        || trimmed == "empty directory"
    {
        return false;
    }
    let trimmed_start = text.trim_start();
    let digit_count = trimmed_start
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();

    if digit_count > 0
        && trimmed_start
            .chars()
            .nth(digit_count)
            .is_some_and(char::is_whitespace)
    {
        return true;
    }

    if trimmed_start.starts_with('+')
        || trimmed_start.starts_with('-')
        || trimmed_start.starts_with('@')
        || trimmed_start.starts_with("diff")
        || trimmed_start.starts_with("index")
        || trimmed_start.starts_with("Index:")
        || trimmed_start.starts_with("===")
        || text.starts_with(' ')
    {
        return true;
    }

    false
}

/// 將預覽中游標所在行套用高亮：首個數字行號套用 accent 粗體，其餘 span 套用 preview_current_line_bg。
pub(crate) fn highlight_cursor_line(line: Line<'static>, theme: Theme) -> Line<'static> {
    if line.spans.is_empty() {
        return Line::from(vec![Span::styled(" ", theme.preview_current_line_style())]);
    }
    let styled_spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .enumerate()
        .map(|(span_idx, span)| {
            let mut style = span.style.bg(theme.preview_current_line_bg);
            let trimmed = span.content.trim();
            if span_idx == 0 && !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                style = style.fg(theme.accent).add_modifier(Modifier::BOLD);
            }
            Span::styled(span.content, style)
        })
        .collect();
    let mut res = Line::from(styled_spans);
    res.alignment = line.alignment;
    res
}

/// 將單一 preview `Line` 套用搜尋高亮，並完整保留原本的語法高亮色彩與樣式。
pub(crate) fn highlight_preview_line(
    line: Line<'static>,
    lower_query: &str,
    theme: Theme,
    is_current_line: bool,
    current_match_start: Option<usize>,
) -> Line<'static> {
    let text = line.to_string();
    let lower_text = text.to_lowercase();

    if !lower_text.contains(lower_query) {
        return if is_current_line {
            let styled_spans: Vec<_> = line
                .spans
                .into_iter()
                .enumerate()
                .map(|(span_idx, span)| {
                    let mut style = span.style.bg(theme.preview_current_line_bg);
                    let trimmed = span.content.trim();
                    if span_idx == 0
                        && !trimmed.is_empty()
                        && trimmed.chars().all(|c| c.is_ascii_digit())
                    {
                        style = style.fg(theme.accent).add_modifier(Modifier::BOLD);
                    }
                    Span::styled(span.content, style)
                })
                .collect();
            let mut res = Line::from(styled_spans);
            res.alignment = line.alignment;
            res
        } else {
            line
        };
    }

    let mut match_ranges = Vec::new();
    let mut cursor = 0usize;

    while cursor <= lower_text.len() {
        let Some(found) = lower_text
            .get(cursor..)
            .and_then(|segment| segment.find(lower_query))
        else {
            break;
        };
        let start = cursor + found;
        let end = start.saturating_add(lower_query.len());
        match_ranges.push((start, end));
        cursor = start.saturating_add(lower_query.len().max(1));
    }

    let match_style = if is_current_line {
        Style::default()
            .bg(theme.preview_current_line_bg)
            .fg(theme.preview_match_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.preview_match_style().add_modifier(Modifier::BOLD)
    };
    let current_match_style = Style::default()
        .bg(theme.preview_match_bg)
        .fg(theme.preview_match_fg)
        .add_modifier(Modifier::BOLD);

    let mut new_spans = Vec::new();
    let mut span_global_offset = 0usize;

    for (span_idx, span) in line.spans.into_iter().enumerate() {
        let span_len = span.content.len();
        let span_start = span_global_offset;
        let span_end = span_global_offset + span_len;
        span_global_offset = span_end;

        let base_style = if is_current_line {
            let mut style = span.style.bg(theme.preview_current_line_bg);
            let trimmed = span.content.trim();
            if span_idx == 0 && !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
                style = style.fg(theme.accent).add_modifier(Modifier::BOLD);
            }
            style
        } else {
            span.style
        };

        // 篩選出與目前 span 重疊之搜尋命中區段
        let overlapping: Vec<(usize, usize, usize)> = match_ranges
            .iter()
            .copied()
            .filter(|&(m_start, m_end)| m_start < span_end && m_end > span_start)
            .map(|(m_start, m_end)| {
                let l_start = m_start.max(span_start) - span_start;
                let l_end = m_end.min(span_end) - span_start;
                (l_start, l_end, m_start)
            })
            .collect();

        if overlapping.is_empty() {
            new_spans.push(Span::styled(span.content, base_style));
            continue;
        }

        let span_str = span.content.as_ref();
        let mut local_cursor = 0usize;

        for (l_start, l_end, global_m_start) in overlapping {
            if l_start > local_cursor
                && span_str.is_char_boundary(local_cursor)
                && span_str.is_char_boundary(l_start)
                && let Some(head) = span_str.get(local_cursor..l_start)
                && !head.is_empty()
            {
                new_spans.push(Span::styled(head.to_string(), base_style));
            }

            if span_str.is_char_boundary(l_start)
                && span_str.is_char_boundary(l_end)
                && let Some(body) = span_str.get(l_start..l_end)
                && !body.is_empty()
            {
                let style = if current_match_start == Some(global_m_start) {
                    current_match_style
                } else {
                    match_style
                };
                new_spans.push(Span::styled(body.to_string(), style));
            }

            local_cursor = l_end;
        }

        if local_cursor < span_len
            && span_str.is_char_boundary(local_cursor)
            && let Some(tail) = span_str.get(local_cursor..)
            && !tail.is_empty()
        {
            new_spans.push(Span::styled(tail.to_string(), base_style));
        }
    }

    let mut res = Line::from(new_spans);
    res.alignment = line.alignment;
    res
}
