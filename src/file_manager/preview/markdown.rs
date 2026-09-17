//! Markdown 語法高亮、行內樣式解析與 GitHub Alert 格式化。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// 解析 Markdown 行內的 inline 標記（如 `code`、**bold**、[label](url) 等）。
pub fn highlight_markdown_inline(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut last_idx = 0;

    let code_style = Style::default().fg(Color::Rgb(235, 203, 139));
    let link_style = Style::default()
        .fg(Color::Rgb(143, 188, 187))
        .add_modifier(Modifier::UNDERLINED);
    let url_style = Style::default().fg(Color::Rgb(101, 115, 126));

    while let Some(&(idx, ch)) = chars.peek() {
        // 1. 行內程式碼 `code`
        if ch == '`' {
            if idx > last_idx {
                spans.push(Span::styled(text[last_idx..idx].to_string(), base_style));
            }
            chars.next();
            let code_start = idx;
            let mut closed = false;
            while let Some(&(_, c)) = chars.peek() {
                chars.next();
                if c == '`' {
                    let code_end = chars.peek().map(|&(i, _)| i).unwrap_or(text.len());
                    spans.push(Span::styled(
                        text[code_start..code_end].to_string(),
                        code_style,
                    ));
                    last_idx = code_end;
                    closed = true;
                    break;
                }
            }
            if !closed {
                spans.push(Span::styled(text[code_start..].to_string(), code_style));
                last_idx = text.len();
            }
            continue;
        }

        // 2. 粗體 **bold** 或 __bold__
        if (ch == '*' && text[idx..].starts_with("**"))
            || (ch == '_' && text[idx..].starts_with("__"))
        {
            let delim = if ch == '*' { "**" } else { "__" };
            if idx > last_idx {
                spans.push(Span::styled(text[last_idx..idx].to_string(), base_style));
            }
            chars.next();
            chars.next();
            let content_start = idx + 2;
            if let Some(close_pos) = text[content_start..].find(delim) {
                let bold_text = &text[content_start..content_start + close_pos];
                spans.push(Span::styled(
                    bold_text.to_string(),
                    base_style.add_modifier(Modifier::BOLD),
                ));
                let end_pos = content_start + close_pos + 2;
                while let Some(&(i, _)) = chars.peek() {
                    if i < end_pos {
                        chars.next();
                    } else {
                        break;
                    }
                }
                last_idx = end_pos;
                continue;
            }
        }

        // 3. 超連結 [label](url)
        if ch == '['
            && let Some(close_bracket) = text[idx..].find(']')
        {
            let label_text = &text[idx + 1..idx + close_bracket];
            let after_bracket = &text[idx + close_bracket + 1..];
            if after_bracket.starts_with('(')
                && let Some(close_paren) = after_bracket.find(')')
            {
                if idx > last_idx {
                    spans.push(Span::styled(text[last_idx..idx].to_string(), base_style));
                }
                let url_text = &after_bracket[1..close_paren];
                spans.push(Span::styled(format!("[{label_text}]"), link_style));
                spans.push(Span::styled(format!("({url_text})"), url_style));
                let end_pos = idx + close_bracket + 1 + close_paren + 1;
                while let Some(&(i, _)) = chars.peek() {
                    if i < end_pos {
                        chars.next();
                    } else {
                        break;
                    }
                }
                last_idx = end_pos;
                continue;
            }
        }

        // 4. 原始 URL <http...>
        if ch == '<'
            && (text[idx..].starts_with("<http://") || text[idx..].starts_with("<https://"))
            && let Some(close_pos) = text[idx..].find('>')
        {
            if idx > last_idx {
                spans.push(Span::styled(text[last_idx..idx].to_string(), base_style));
            }
            let raw_url = &text[idx..=idx + close_pos];
            spans.push(Span::styled(raw_url.to_string(), link_style));
            let end_pos = idx + close_pos + 1;
            while let Some(&(i, _)) = chars.peek() {
                if i < end_pos {
                    chars.next();
                } else {
                    break;
                }
            }
            last_idx = end_pos;
            continue;
        }

        chars.next();
    }

    if last_idx < text.len() {
        spans.push(Span::styled(text[last_idx..].to_string(), base_style));
    }

    spans
}

/// 為單一行 Markdown 內容產生語法高亮 Span 清單。
pub fn highlight_markdown_line(line: &str, in_code_fence: &mut bool) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];

    let mut spans = Vec::new();
    if !indent.is_empty() {
        spans.push(Span::raw(indent.to_string()));
    }

    if trimmed.is_empty() {
        return spans;
    }

    // 1. 程式碼區塊標記 (Code Fence)：``` 或 ~~~
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        *in_code_fence = !*in_code_fence;
        let fence_prefix = if trimmed.starts_with("```") {
            "```"
        } else {
            "~~~"
        };
        let rest = &trimmed[fence_prefix.len()..];
        let fence_style = Style::default()
            .fg(Color::Rgb(208, 135, 112))
            .add_modifier(Modifier::BOLD);
        spans.push(Span::styled(fence_prefix.to_string(), fence_style));
        if !rest.is_empty() {
            let lang_style = Style::default()
                .fg(Color::Rgb(235, 203, 139))
                .add_modifier(Modifier::ITALIC);
            spans.push(Span::styled(rest.to_string(), lang_style));
        }
        return spans;
    }

    // 2. 處於程式碼區塊內部：以代碼風格呈現
    if *in_code_fence {
        let code_style = Style::default().fg(Color::Rgb(192, 197, 206));
        spans.push(Span::styled(trimmed.to_string(), code_style));
        return spans;
    }

    // 3. 標題行 (#, ##, ###, ####, #####, ######)
    if trimmed.starts_with('#') {
        let hashes = trimmed.chars().take_while(|&c| c == '#').count();
        if hashes <= 6 && trimmed[hashes..].starts_with(' ') {
            let hash_str = &trimmed[..hashes];
            let rest = &trimmed[hashes..];
            let header_style = match hashes {
                1 => Style::default()
                    .fg(Color::Rgb(143, 188, 187))
                    .add_modifier(Modifier::BOLD),
                2 => Style::default()
                    .fg(Color::Rgb(129, 161, 193))
                    .add_modifier(Modifier::BOLD),
                3 => Style::default()
                    .fg(Color::Rgb(235, 203, 139))
                    .add_modifier(Modifier::BOLD),
                4 => Style::default()
                    .fg(Color::Rgb(163, 190, 140))
                    .add_modifier(Modifier::BOLD),
                _ => Style::default()
                    .fg(Color::Rgb(180, 142, 173))
                    .add_modifier(Modifier::BOLD),
            };
            spans.push(Span::styled(hash_str.to_string(), header_style));
            spans.extend(highlight_markdown_inline(rest, header_style));
            return spans;
        }
    }

    // 4. 引言行與 GitHub Alert 標記 (> [!NOTE], > [!TIP], etc.)
    if let Some(after_gt) = trimmed.strip_prefix('>') {
        let quote_mark_style = Style::default().fg(Color::Rgb(101, 115, 126));
        spans.push(Span::styled(">".to_string(), quote_mark_style));

        let quote_trimmed = after_gt.trim_start();
        let leading_spaces = &after_gt[..after_gt.len() - quote_trimmed.len()];
        if !leading_spaces.is_empty() {
            spans.push(Span::raw(leading_spaces.to_string()));
        }

        let alert_opt = if quote_trimmed.starts_with("[!NOTE]") {
            Some((
                "[!NOTE]",
                Style::default()
                    .fg(Color::Rgb(129, 161, 193))
                    .add_modifier(Modifier::BOLD),
            ))
        } else if quote_trimmed.starts_with("[!TIP]") {
            Some((
                "[!TIP]",
                Style::default()
                    .fg(Color::Rgb(163, 190, 140))
                    .add_modifier(Modifier::BOLD),
            ))
        } else if quote_trimmed.starts_with("[!IMPORTANT]") {
            Some((
                "[!IMPORTANT]",
                Style::default()
                    .fg(Color::Rgb(180, 142, 173))
                    .add_modifier(Modifier::BOLD),
            ))
        } else if quote_trimmed.starts_with("[!WARNING]") {
            Some((
                "[!WARNING]",
                Style::default()
                    .fg(Color::Rgb(235, 203, 139))
                    .add_modifier(Modifier::BOLD),
            ))
        } else if quote_trimmed.starts_with("[!CAUTION]") {
            Some((
                "[!CAUTION]",
                Style::default()
                    .fg(Color::Rgb(191, 97, 106))
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            None
        };

        if let Some((alert_tag, alert_style)) = alert_opt {
            spans.push(Span::styled(alert_tag.to_string(), alert_style));
            let rest = &quote_trimmed[alert_tag.len()..];
            if !rest.is_empty() {
                let quote_body_style = Style::default()
                    .fg(Color::Rgb(140, 150, 165))
                    .add_modifier(Modifier::ITALIC);
                spans.extend(highlight_markdown_inline(rest, quote_body_style));
            }
            return spans;
        }

        let quote_body_style = Style::default()
            .fg(Color::Rgb(140, 150, 165))
            .add_modifier(Modifier::ITALIC);
        spans.extend(highlight_markdown_inline(quote_trimmed, quote_body_style));
        return spans;
    }

    // 5. 分隔線 (---, ***, ___)
    if (trimmed.chars().all(|c| c == '-') && trimmed.len() >= 3)
        || (trimmed.chars().all(|c| c == '*') && trimmed.len() >= 3)
        || (trimmed.chars().all(|c| c == '_') && trimmed.len() >= 3)
    {
        spans.push(Span::styled(
            trimmed.to_string(),
            Style::default().fg(Color::Rgb(101, 115, 126)),
        ));
        return spans;
    }

    let text_style = Style::default().fg(Color::Rgb(192, 197, 206));

    // 6. 清單項目 (- , * , + , 1. , 及工作清單 - [ ] / - [x])
    let bullet_prefix = if trimmed.starts_with("- ") {
        Some("- ")
    } else if trimmed.starts_with("* ") {
        Some("* ")
    } else if trimmed.starts_with("+ ") {
        Some("+ ")
    } else {
        None
    };

    if let Some(prefix) = bullet_prefix {
        let bullet_style = Style::default().fg(Color::Rgb(180, 142, 173));
        spans.push(Span::styled(prefix.to_string(), bullet_style));
        let rest = &trimmed[prefix.len()..];

        if let Some(sub) = rest.strip_prefix("[ ] ") {
            spans.push(Span::styled(
                "[ ] ".to_string(),
                Style::default().fg(Color::Rgb(235, 203, 139)),
            ));
            spans.extend(highlight_markdown_inline(sub, text_style));
            return spans;
        } else if let Some(sub) = rest
            .strip_prefix("[x] ")
            .or_else(|| rest.strip_prefix("[X] "))
        {
            let marker = &rest[..4];
            spans.push(Span::styled(
                marker.to_string(),
                Style::default()
                    .fg(Color::Rgb(163, 190, 140))
                    .add_modifier(Modifier::BOLD),
            ));
            spans.extend(highlight_markdown_inline(sub, text_style));
            return spans;
        }

        spans.extend(highlight_markdown_inline(rest, text_style));
        return spans;
    }

    if let Some(dot_idx) = trimmed.find(". ")
        && dot_idx > 0
        && trimmed[..dot_idx].chars().all(|c| c.is_ascii_digit())
    {
        let num_prefix = &trimmed[..dot_idx + 2];
        let num_style = Style::default().fg(Color::Rgb(208, 135, 112));
        spans.push(Span::styled(num_prefix.to_string(), num_style));
        let rest = &trimmed[dot_idx + 2..];
        spans.extend(highlight_markdown_inline(rest, text_style));
        return spans;
    }

    // 7. 表格行 (| col1 | col2 |)
    if trimmed.starts_with('|') && trimmed.contains('|') {
        let border_style = Style::default().fg(Color::Rgb(101, 115, 126));
        if trimmed
            .chars()
            .all(|c| c == '|' || c == '-' || c == ':' || c.is_whitespace())
        {
            spans.push(Span::styled(trimmed.to_string(), border_style));
            return spans;
        }

        let parts = trimmed.split('|');
        let mut first = true;
        for part in parts {
            if !first {
                spans.push(Span::styled("|".to_string(), border_style));
            }
            first = false;
            if !part.is_empty() {
                spans.extend(highlight_markdown_inline(part, text_style));
            }
        }
        return spans;
    }

    // 8. 一般內文段落
    spans.extend(highlight_markdown_inline(trimmed, text_style));
    spans
}
