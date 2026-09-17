//! 程式碼語法高亮、Syntect 引擎呼叫、語言別名映射與 TOML 解析。

use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::FontStyle;
use syntect::parsing::SyntaxSet;

use super::markdown::highlight_markdown_line;
use super::{SYNTAX_SET, THEME_SET};

/// 將 syntect 語法著色風格轉換為 ratatui 終端樣式。
pub(crate) fn syntect_style_to_ratatui(style: syntect::highlighting::Style) -> Style {
    let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
    let mut ratatui_style = Style::default().fg(fg);
    if style.font_style.contains(FontStyle::BOLD) {
        ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
    }
    ratatui_style
}

/// 為單一行 TOML 內容產生語法高亮 Span 清單。
pub fn highlight_toml_line(line: &str) -> Vec<Span<'static>> {
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

    // 1. 純註解行
    if trimmed.starts_with('#') {
        spans.push(Span::styled(
            trimmed.to_string(),
            Style::default()
                .fg(Color::Rgb(101, 115, 126))
                .add_modifier(Modifier::ITALIC),
        ));
        return spans;
    }

    // 2. 表格標題：[[section]] 或 [section]
    if (trimmed.starts_with("[[") && trimmed.contains("]]"))
        || (trimmed.starts_with('[') && trimmed.contains(']'))
    {
        let is_double = trimmed.starts_with("[[");
        let open_bracket = if is_double { "[[" } else { "[" };
        let close_bracket = if is_double { "]]" } else { "]" };

        let punct_style = Style::default().fg(Color::Rgb(192, 197, 206));
        let header_style = Style::default()
            .fg(Color::Rgb(235, 203, 139))
            .add_modifier(Modifier::BOLD);

        if let Some(open_pos) = trimmed.find(open_bracket) {
            let after_open = &trimmed[open_pos + open_bracket.len()..];
            if let Some(close_pos) = after_open.find(close_bracket) {
                let section_name = &after_open[..close_pos];
                let rest = &after_open[close_pos + close_bracket.len()..];

                spans.push(Span::styled(open_bracket.to_string(), punct_style));
                spans.push(Span::styled(section_name.to_string(), header_style));
                spans.push(Span::styled(close_bracket.to_string(), punct_style));

                if !rest.is_empty() {
                    if rest.trim_start().starts_with('#') {
                        spans.push(Span::styled(
                            rest.to_string(),
                            Style::default()
                                .fg(Color::Rgb(101, 115, 126))
                                .add_modifier(Modifier::ITALIC),
                        ));
                    } else {
                        spans.push(Span::raw(rest.to_string()));
                    }
                }
                return spans;
            }
        }
    }

    // 3. 一般鍵值行或內嵌結構：逐字元分詞解析
    let mut chars = trimmed.char_indices().peekable();
    let mut last_idx = 0;
    let mut in_key = true;

    let key_style = Style::default().fg(Color::Rgb(180, 142, 173));
    let str_style = Style::default().fg(Color::Rgb(163, 190, 140));
    let num_style = Style::default().fg(Color::Rgb(208, 135, 112));
    let bool_style = Style::default()
        .fg(Color::Rgb(180, 142, 173))
        .add_modifier(Modifier::BOLD);
    let punct_style = Style::default().fg(Color::Rgb(192, 197, 206));
    let comment_style = Style::default()
        .fg(Color::Rgb(101, 115, 126))
        .add_modifier(Modifier::ITALIC);

    while let Some(&(idx, ch)) = chars.peek() {
        if ch == '#' {
            let comment_text = &trimmed[idx..];
            spans.push(Span::styled(comment_text.to_string(), comment_style));
            return spans;
        }

        if ch == '"' || ch == '\'' {
            let quote = ch;
            chars.next();
            let start = idx;
            let mut escaped = false;
            while let Some(&(_, c)) = chars.peek() {
                chars.next();
                if escaped {
                    escaped = false;
                } else if c == '\\' && quote == '"' {
                    escaped = true;
                } else if c == quote {
                    break;
                }
            }
            let end = chars.peek().map(|&(i, _)| i).unwrap_or(trimmed.len());
            spans.push(Span::styled(trimmed[start..end].to_string(), str_style));
            last_idx = end;
            in_key = false;
            continue;
        }

        if ch == '=' {
            spans.push(Span::styled("=", punct_style));
            chars.next();
            in_key = false;
            last_idx = chars.peek().map(|&(i, _)| i).unwrap_or(trimmed.len());
            continue;
        }

        if ch == '{' || ch == '}' || ch == '[' || ch == ']' || ch == ',' {
            spans.push(Span::styled(ch.to_string(), punct_style));
            chars.next();
            last_idx = chars.peek().map(|&(i, _)| i).unwrap_or(trimmed.len());
            continue;
        }

        if ch.is_whitespace() {
            chars.next();
            let start = idx;
            while let Some(&(_, c)) = chars.peek() {
                if c.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }
            let end = chars.peek().map(|&(i, _)| i).unwrap_or(trimmed.len());
            spans.push(Span::raw(trimmed[start..end].to_string()));
            last_idx = end;
            continue;
        }

        let start = idx;
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace()
                || c == '='
                || c == '#'
                || c == '"'
                || c == '\''
                || c == '{'
                || c == '}'
                || c == '['
                || c == ']'
                || c == ','
            {
                break;
            }
            chars.next();
        }
        let end = chars.peek().map(|&(i, _)| i).unwrap_or(trimmed.len());
        let word = &trimmed[start..end];

        if in_key {
            spans.push(Span::styled(word.to_string(), key_style));
        } else if word == "true" || word == "false" {
            spans.push(Span::styled(word.to_string(), bool_style));
        } else if word.chars().all(|c| {
            c.is_ascii_digit()
                || c == '.'
                || c == '-'
                || c == '+'
                || c == '_'
                || c == 'e'
                || c == 'E'
                || c == 'x'
                || c == 'o'
                || c == 'b'
        }) && word.chars().any(|c| c.is_ascii_digit())
        {
            spans.push(Span::styled(word.to_string(), num_style));
        } else {
            spans.push(Span::styled(
                word.to_string(),
                Style::default().fg(Color::Rgb(192, 197, 206)),
            ));
        }
        last_idx = end;
    }

    if last_idx < trimmed.len() {
        spans.push(Span::raw(trimmed[last_idx..].to_string()));
    }

    spans
}

/// 將程式碼特定區間（例如捲動到第 5000 行時的 20 行）直接轉換為帶有色彩與正確全域行號的預覽行清單。
pub fn highlight_code_preview_slice(
    path: &Path,
    contents: &str,
    start_line: usize,
    count: usize,
    total_lines: usize,
    theme_name: Option<&str>,
) -> Vec<Line<'static>> {
    let content_lines: Vec<&str> = contents.lines().skip(start_line).take(count).collect();
    if content_lines.is_empty() {
        return Vec::new();
    }

    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let is_toml = ext == "toml"
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".toml") || n == "Cargo.lock")
            .unwrap_or(false);

    let is_markdown = ext == "md"
        || ext == "markdown"
        || ext == "mdown"
        || ext == "mkd"
        || path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".md"))
            .unwrap_or(false);

    let num_width = total_lines.to_string().len().max(3);
    let line_num_style = Style::default().fg(Color::Rgb(110, 115, 128));

    if is_toml {
        let mut lines = Vec::new();
        for (index, line) in content_lines.into_iter().enumerate() {
            let line_num = start_line + index + 1;
            let line_num_str = format!("{:>width$} ", line_num, width = num_width);
            let line_num_span = Span::styled(line_num_str, line_num_style);

            let mut spans = vec![line_num_span];
            spans.extend(highlight_toml_line(line));
            lines.push(Line::from(spans));
        }
        return lines;
    }

    if is_markdown {
        let mut lines = Vec::new();
        let mut in_code_fence = false;
        if start_line > 0 {
            for l in contents.lines().take(start_line) {
                let t = l.trim_start();
                if t.starts_with("```") || t.starts_with("~~~") {
                    in_code_fence = !in_code_fence;
                }
            }
        }

        for (index, line) in content_lines.into_iter().enumerate() {
            let line_num = start_line + index + 1;
            let line_num_str = format!("{:>width$} ", line_num, width = num_width);
            let line_num_span = Span::styled(line_num_str, line_num_style);

            let mut spans = vec![line_num_span];
            spans.extend(highlight_markdown_line(line, &mut in_code_fence));
            lines.push(Line::from(spans));
        }
        return lines;
    }

    let syntax = find_syntax_for_path(&SYNTAX_SET, path);

    let theme_key = theme_name.unwrap_or("base16-ocean.dark");
    let theme = THEME_SET
        .themes
        .get(theme_key)
        .or_else(|| THEME_SET.themes.get("base16-ocean.dark"))
        .or_else(|| THEME_SET.themes.values().next())
        .expect("syntect themes must contain at least one default theme");

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();

    for (index, line) in content_lines.into_iter().enumerate() {
        let line_num = start_line + index + 1;
        let line_num_str = format!("{:>width$} ", line_num, width = num_width);
        let line_num_span = Span::styled(line_num_str, line_num_style);

        let line_with_newline = if line.ends_with('\n') {
            std::borrow::Cow::Borrowed(line)
        } else {
            std::borrow::Cow::Owned(format!("{line}\n"))
        };

        match highlighter.highlight_line(&line_with_newline, &SYNTAX_SET) {
            Ok(ranges) => {
                let mut spans = vec![line_num_span];
                for (style, text) in ranges {
                    let trimmed = text.trim_end_matches(['\r', '\n']);
                    if !trimmed.is_empty() {
                        spans.push(Span::styled(
                            trimmed.to_string(),
                            syntect_style_to_ratatui(style),
                        ));
                    }
                }
                lines.push(Line::from(spans));
            }
            Err(_) => {
                lines.push(Line::from(vec![line_num_span, Span::raw(line.to_string())]));
            }
        }
    }

    lines
}

/// 依據副檔名與檔案路徑尋找最佳語法定義。
///
/// 由於 syntect 預設內建語法庫（Sublime Text 預設包）未包含 TypeScript、JSX、SCSS 等現代前端格式，
/// 此函式提供智慧別名與相容語法映射（例如 `ts`/`tsx`/`jsx` 映射至 `JavaScript`，`scss`/`sass`/`less` 映射至 `CSS`），
/// 確保所有常見語言皆能享有完整的語法色彩渲染。
pub(crate) fn find_syntax_for_path<'a>(
    syntax_set: &'a SyntaxSet,
    path: &Path,
) -> &'a syntect::parsing::SyntaxReference {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase());

    if let Some(ref ext_str) = ext {
        if let Some(syntax) = syntax_set.find_syntax_by_extension(ext_str) {
            return syntax;
        }

        let fallback_ext = match ext_str.as_str() {
            // TypeScript 與現代 JavaScript 擴充
            "ts" | "tsx" | "mts" | "cts" | "jsx" | "mjs" | "cjs" => Some("js"),
            // 樣式表預處理器
            "scss" | "sass" | "less" => Some("css"),
            // Shell 與終端腳本（PowerShell / Unix Shell）
            "bash" | "zsh" | "fish" | "ksh" | "ps1" | "psm1" | "psd1" => Some("sh"),
            "cmd" => Some("bat"),
            // C / C++ 標頭檔與變體
            "hpp" | "hxx" | "hh" | "cxx" | "cc" => Some("cpp"),
            "h" => Some("c"),
            // JSON 變體
            "jsonc" | "json5" => Some("json"),
            // 現代前端單一元件檔
            "vue" | "svelte" => Some("html"),
            _ => None,
        };

        if let Some(syntax) =
            fallback_ext.and_then(|target| syntax_set.find_syntax_by_extension(target))
        {
            return syntax;
        }
    }

    syntax_set
        .find_syntax_for_file(path)
        .ok()
        .flatten()
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text())
}

/// 將程式碼或文字內容依據副檔名與語法定義轉換為帶有色彩與自適應暗色行號的預覽行清單。
pub fn highlight_code_preview(
    path: &Path,
    contents: &str,
    max_lines: usize,
    theme_name: Option<&str>,
) -> Vec<Line<'static>> {
    let total_lines = contents.lines().count().max(1);
    if contents.lines().next().is_none() {
        return vec![Line::from("[empty file]")];
    }
    highlight_code_preview_slice(path, contents, 0, max_lines, total_lines, theme_name)
}

/// 在背景預熱 syntect 常用語言語法引擎，消除使用者首次開啟程式碼預覽時的冷啟動編繹延遲。
pub(crate) fn preheat_syntect_common_syntaxes() {
    let syntax_set = &*SYNTAX_SET;
    let theme_set = &*THEME_SET;
    let theme = theme_set
        .themes
        .get("base16-ocean.dark")
        .or_else(|| theme_set.themes.values().next());

    if let Some(theme) = theme {
        for ext in [
            "rs", "py", "js", "ts", "json", "yaml", "sh", "c", "cpp", "go",
        ] {
            let fake_path = Path::new("file").with_extension(ext);
            let syntax = find_syntax_for_path(syntax_set, &fake_path);
            let mut highlighter = HighlightLines::new(syntax, theme);
            let _ = highlighter.highlight_line("// warm\n", syntax_set);
        }
    }
}
