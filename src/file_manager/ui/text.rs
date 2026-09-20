use chrono::{DateTime, Local};
use ratatui::{
    style::Modifier,
    text::{Line, Span},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    file_manager::{entry::FileEntry, pane::SortDetailKind},
    theme::Theme,
};

use super::types::{ScrolledInputView, TaskPanelLine};

/// 將多個標題狀態片段去掉前後空白後重新用單一空格組合，避免出現多餘空隙。
pub(crate) fn normalize_title_status_segments(segments: &[&str]) -> String {
    segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 將 pane 標題的 prefix / path / suffix 以最少必要空格拼接，避免浪費可用寬度。
pub(crate) fn join_title_parts(prefix: &str, path: &str, suffix: &str) -> String {
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
pub(crate) fn title_separator_width(has_path: bool, has_suffix: bool) -> usize {
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
pub(crate) fn compact_path_for_title(path: &str, max_chars: usize) -> String {
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
pub(crate) fn truncate_text_end_preserving_tail(text: &str, max_chars: usize) -> String {
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
pub(crate) fn compact_last_segment_only(last_part: &str, max_chars: usize) -> String {
    if last_part.chars().count() <= max_chars {
        last_part.to_string()
    } else if max_chars <= 1 {
        String::from("…")
    } else {
        truncate_text_end_preserving_tail(last_part, max_chars)
    }
}

/// 回傳游標在終端機畫面上對應的顯示欄數。
#[allow(dead_code)]
pub(crate) fn cursor_display_width(text: &str, cursor_char_count: usize) -> usize {
    text.chars()
        .take(cursor_char_count)
        .map(|c| c.width().unwrap_or(0))
        .sum()
}

/// 針對長文字輸入框計算水平滑動視窗（Scheme A: Viewport Scrolling）。
///
/// 當文字長度超過可用寬度時，自動依目前游標位置決定可見切片，
/// 並在被截斷的一側或兩側加上 `<` / `>` 溢位提示符號。
pub(crate) fn compute_scrolled_input(
    buffer: &str,
    cursor: usize,
    available_width: usize,
    prefix: Option<&str>,
    theme: Theme,
) -> ScrolledInputView {
    let prefix_str = prefix.unwrap_or("");
    let prefix_w = UnicodeWidthStr::width(prefix_str);
    if available_width <= prefix_w {
        return ScrolledInputView {
            spans: vec![Span::raw(prefix_str.to_string())],
            cursor_col: 0,
        };
    }

    let w = available_width - prefix_w;
    let chars: Vec<char> = buffer.chars().collect();
    let char_widths: Vec<usize> = chars
        .iter()
        .map(|c| UnicodeWidthChar::width(*c).unwrap_or(1).max(1))
        .collect();
    let total_w: usize = char_widths.iter().sum();
    let cursor = cursor.min(chars.len());
    let cursor_w: usize = char_widths[..cursor].iter().sum();

    // 寬度足夠顯示全部內容（包括字尾游標預留空間），無需滑動視窗
    if total_w < w {
        let mut spans = Vec::with_capacity(2);
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::raw(buffer.to_string()));
        return ScrolledInputView {
            spans,
            cursor_col: prefix_len_clamp(prefix_w + cursor_w, available_width) as u16,
        };
    }

    // 空間極小時的最小保護
    if w <= 2 {
        let mut spans = Vec::new();
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::styled(
            ">",
            theme.accent_style().add_modifier(Modifier::BOLD),
        ));
        return ScrolledInputView {
            spans,
            cursor_col: prefix_w as u16,
        };
    }

    // 判斷左溢位 (<) 與右溢位 (>)
    let indicator_style = theme.accent_style().add_modifier(Modifier::BOLD);

    // Case 1: 游標靠左側（起點固定為 0，只有右側溢位）
    // 右側預留 1 格給 `>`，可用寬度為 w - 1
    if cursor_w < w.saturating_sub(1) {
        let budget = w.saturating_sub(1);
        let mut accumulated = 0;
        let mut end_idx = 0;
        for (i, &cw) in char_widths.iter().enumerate() {
            if accumulated + cw > budget {
                break;
            }
            accumulated += cw;
            end_idx = i + 1;
        }
        end_idx = end_idx.max(cursor).min(chars.len());

        let visible_str: String = chars[0..end_idx].iter().collect();
        let mut spans = Vec::with_capacity(3);
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::raw(visible_str));
        spans.push(Span::styled(">", indicator_style));

        return ScrolledInputView {
            spans,
            cursor_col: prefix_len_clamp(prefix_w + cursor_w, available_width) as u16,
        };
    }

    // 剩餘寬度從游標到尾端
    let remaining_w: usize = char_widths[cursor..].iter().sum();

    // Case 2: 游標靠右側尾端（終點固定為 chars.len()，只有左側溢位）
    if remaining_w <= 2 || cursor == chars.len() {
        let budget = w.saturating_sub(2);
        let mut accumulated = 0;
        let mut start_idx = chars.len();
        for (i, &cw) in char_widths.iter().enumerate().rev() {
            if accumulated + cw > budget {
                break;
            }
            accumulated += cw;
            start_idx = i;
        }
        start_idx = start_idx.min(cursor);

        let visible_str: String = chars[start_idx..chars.len()].iter().collect();
        let cursor_offset: usize = char_widths[start_idx..cursor].iter().sum();

        let mut spans = Vec::with_capacity(3);
        if !prefix_str.is_empty() {
            spans.push(Span::raw(prefix_str.to_string()));
        }
        spans.push(Span::styled("<", indicator_style));
        spans.push(Span::raw(visible_str));

        return ScrolledInputView {
            spans,
            cursor_col: prefix_len_clamp(prefix_w + 1 + cursor_offset, available_width) as u16,
        };
    }

    // Case 3: 游標在中間（兩側皆有溢位，左右各留 1 格給 `<` 與 `>`）
    let budget = w.saturating_sub(2);
    let margin_right = 3.min(budget / 3);
    let target_cursor_pos = budget.saturating_sub(margin_right).max(1);

    let mut accumulated = 0;
    let mut start_idx = cursor;
    for (i, &cw) in char_widths[..cursor].iter().enumerate().rev() {
        if accumulated + cw > target_cursor_pos {
            break;
        }
        accumulated += cw;
        start_idx = i;
    }

    let mut forward_acc = 0;
    let mut end_idx = start_idx;
    for (i, &cw) in char_widths[start_idx..].iter().enumerate() {
        if forward_acc + cw > budget {
            break;
        }
        forward_acc += cw;
        end_idx = start_idx + i + 1;
    }
    end_idx = end_idx.max(cursor).min(chars.len());

    let visible_str: String = chars[start_idx..end_idx].iter().collect();
    let cursor_offset: usize = char_widths[start_idx..cursor].iter().sum();

    let has_left = start_idx > 0;
    let has_right = end_idx < chars.len();

    let mut spans = Vec::with_capacity(4);
    if !prefix_str.is_empty() {
        spans.push(Span::raw(prefix_str.to_string()));
    }
    if has_left {
        spans.push(Span::styled("<", indicator_style));
    }
    spans.push(Span::raw(visible_str));
    if has_right {
        spans.push(Span::styled(">", indicator_style));
    }

    let left_pad = if has_left { 1 } else { 0 };
    ScrolledInputView {
        spans,
        cursor_col: prefix_len_clamp(prefix_w + left_pad + cursor_offset, available_width) as u16,
    }
}

pub(crate) fn prefix_len_clamp(val: usize, max: usize) -> usize {
    val.min(max.saturating_sub(1))
}

/// 將過長文字裁切成指定寬度，避免面板欄位爆掉。
pub(crate) fn truncate_text(text: &str, max_chars: usize) -> String {
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
pub(crate) fn task_panel_display_lines(
    task: &TaskPanelLine,
    max_width: usize,
) -> Vec<Line<'static>> {
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
pub(crate) fn wrap_text_for_width(text: &str, max_width: usize, indent: &str) -> Vec<String> {
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
pub(crate) fn truncate_text_to_display_width(text: &str, max_width: usize) -> String {
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

/// 根據指定排序屬性，格式化檔案條目的詳細資訊。
pub(crate) fn format_sort_detail(entry: &FileEntry, detail_kind: SortDetailKind) -> String {
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
            } else if entry.is_sparse_empty {
                format!("{} [0B]", format_size_short(entry.size))
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
pub(crate) fn format_permissions_detail(entry: &FileEntry) -> String {
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
pub(crate) fn format_unix_permissions(is_dir: bool, mode: u32) -> String {
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
pub(crate) fn format_system_time(value: std::time::SystemTime) -> String {
    let datetime: DateTime<Local> = value.into();
    datetime.format("%m/%d %H:%M").to_string()
}

/// 把 byte 大小轉成 PaneFM 在 macOS 與 Windows 共用的 1024 進位短格式。
pub(crate) fn format_size_short(size: u64) -> String {
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
pub(crate) fn format_compact_size(value: f64, suffix: &str) -> String {
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
