use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use super::{
    HelpAction, expand_tilde_path, help_entries, is_unc_path, is_windows_drive_path,
    key_matches_ctrl_letter, key_matches_shifted_letter, looks_like_navigation_path,
};
use crate::file_manager::ui::CommandSuggestionLine;

/// 記錄 command mode 目前是否正拿同一組路徑候選做 Tab 輪詢補全。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionCycle {
    pub(crate) suggestions: Vec<CommandSuggestionLine>,
}

/// 描述 command mode 補全候選目前要往前還是往後切換。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionNavigation {
    Next,
    Previous,
}

/// 把不同終端可能送出的快捷鍵格式，統一轉成 command 補全的切換方向。
///
/// 目前支援：
/// - `Shift+N` / `Shift+P`
/// - `Ctrl+N` / `Ctrl+P`
/// - `Tab` / `Shift+Tab`
/// - `Down` / `Up`
///
/// 這樣就算不同 terminal 對 modifier 的回報格式不一致，
/// command mode 仍然至少有一組可用的候選切換方式。
pub(crate) fn command_suggestion_navigation(key: &KeyEvent) -> Option<SuggestionNavigation> {
    if key_matches_shifted_letter(key, 'N')
        || key_matches_ctrl_letter(key, 'n')
        || key.code == KeyCode::Tab
        || key.code == KeyCode::Down
    {
        return Some(SuggestionNavigation::Next);
    }

    if key_matches_shifted_letter(key, 'P')
        || key_matches_ctrl_letter(key, 'p')
        || key.code == KeyCode::BackTab
        || key.code == KeyCode::Up
    {
        return Some(SuggestionNavigation::Previous);
    }

    None
}

/// 根據目前 command mode 的輸入內容，整理出適合顯示的補全候選。
pub(crate) fn command_suggestions_for_buffer(
    base_dir: Option<&Path>,
    query: &str,
) -> Vec<CommandSuggestionLine> {
    if let Some(context) = command_path_completion_context(base_dir, query) {
        return path_completion_suggestions(&context);
    }
    command_suggestions(query)
}

/// 根據目前 command mode 的輸入內容，整理出適合顯示的命令補全候選。
pub(crate) fn command_suggestions(query: &str) -> Vec<CommandSuggestionLine> {
    let trimmed = query.trim();
    let mut suggestions = Vec::new();
    let mut seen = BTreeSet::new();

    for entry in help_entries("") {
        let HelpAction::Command(command) = entry.action else {
            continue;
        };
        if !(trimmed.is_empty()
            || command.starts_with(trimmed)
            || command
                .split_whitespace()
                .next()
                .is_some_and(|head| head.starts_with(trimmed)))
        {
            continue;
        }
        if !seen.insert(command.to_string()) {
            continue;
        }
        suggestions.push(CommandSuggestionLine {
            command: command.to_string(),
            display_command: entry.line.command,
            shortcut: entry.line.shortcut,
            description: entry.line.description,
        });
    }

    if trimmed.chars().count() > 1 {
        suggestions.sort_by(|left, right| {
            command_suggestion_sort_key(trimmed, &left.command)
                .cmp(&command_suggestion_sort_key(trimmed, &right.command))
        });
    }
    suggestions.truncate(8);

    suggestions
}

/// 計算 command 補全候選的排序鍵，讓較接近使用者輸入的指令排在前面。
///
/// 目前會優先比較：
/// 1. 指令第一段名稱和查詢字串的長度差距
/// 2. 指令第一段名稱的字母順序
/// 3. 完整命令模板，作為最後的穩定排序條件
pub(crate) fn command_suggestion_sort_key(query: &str, command: &str) -> (usize, String, String) {
    let head = command.split_whitespace().next().unwrap_or(command);
    let remainder = head.chars().count().saturating_sub(query.chars().count());
    (remainder, head.to_string(), command.to_string())
}

/// 找出多個候選字串的最長共同前綴，供路徑補全先延伸到共享部分。
pub(crate) fn longest_common_prefix(values: &[&str]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };

    let mut prefix = (*first).to_string();
    for value in values.iter().skip(1) {
        let mut shared = String::new();
        for (left, right) in prefix.chars().zip(value.chars()) {
            if left != right {
                break;
            }
            shared.push(left);
        }
        prefix = shared;
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

/// 描述 command mode 中一次路徑補全需要的上下文資訊。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandPathCompletionContext {
    pub(crate) replacement_prefix: String,
    pub(crate) typed_directory: String,
    pub(crate) search_dir: PathBuf,
    pub(crate) partial_name: String,
    pub(crate) preferred_separator: char,
    /// `true` 代表路徑會觸發 UNC/SMB 網路存取，不可在 command render 時同步掃描。
    pub(crate) network_path: bool,
}

/// 若目前 command buffer 正在輸入路徑，整理出路徑補全所需的上下文。
pub(crate) fn command_path_completion_context(
    base_dir: Option<&Path>,
    query: &str,
) -> Option<CommandPathCompletionContext> {
    let base_dir = base_dir?;
    let (replacement_prefix, raw_path) = if let Some(path) = query.strip_prefix("goto ") {
        (String::from("goto "), path)
    } else if looks_like_navigation_path(query) {
        (String::new(), query.trim())
    } else {
        return None;
    };

    let preferred_separator = if raw_path.contains('\\') { '\\' } else { '/' };
    let (typed_directory, partial_name) = split_typed_path(raw_path);
    let expanded_directory = expand_tilde_path(&typed_directory).unwrap_or(typed_directory.clone());
    let search_dir = if expanded_directory.is_empty() {
        base_dir.to_path_buf()
    } else {
        let expanded_path = PathBuf::from(&expanded_directory);
        if expanded_path.is_absolute()
            || is_windows_drive_path(&expanded_directory)
            || is_unc_path(&expanded_directory)
        {
            expanded_path
        } else {
            base_dir.join(expanded_path)
        }
    };

    Some(CommandPathCompletionContext {
        replacement_prefix,
        typed_directory,
        search_dir,
        partial_name,
        preferred_separator,
        network_path: is_unc_path(raw_path) || raw_path.trim_start().starts_with("smb://"),
    })
}

/// 依照目前的路徑補全上下文，建立 command palette 要顯示的候選列表。
pub(crate) fn path_completion_suggestions(
    context: &CommandPathCompletionContext,
) -> Vec<CommandSuggestionLine> {
    // command suggestions 會在每次按鍵與每次 render 時計算；若這裡讀取 UNC，失聯
    // 主機會把 TUI 主執行緒鎖住數十秒。網路路徑只在 Enter 後交給背景 goto。
    if context.network_path {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(&context.search_dir) else {
        return Vec::new();
    };

    let partial_lower = context.partial_name.to_ascii_lowercase();
    let mut candidates = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !partial_lower.is_empty() && !name.to_ascii_lowercase().starts_with(&partial_lower) {
                return None;
            }

            let mut completed = format!("{}{}", context.typed_directory, name);
            if file_type.is_dir() {
                completed.push(context.preferred_separator);
            }
            let mut display_name = name;
            if file_type.is_dir() {
                display_name.push(context.preferred_separator);
            }

            Some((
                file_type.is_dir(),
                display_name.to_ascii_lowercase(),
                CommandSuggestionLine {
                    command: format!("{}{}", context.replacement_prefix, completed),
                    display_command: display_name,
                    shortcut: String::new(),
                    description: String::new(),
                },
            ))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    candidates
        .into_iter()
        .map(|(_, _, suggestion)| suggestion)
        .take(8)
        .collect()
}

/// 將使用者目前輸入的路徑拆成「父目錄前綴」與「最後一段正在輸入的名稱」。
pub(crate) fn split_typed_path(input: &str) -> (String, String) {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }
    if trimmed.ends_with('/') || trimmed.ends_with('\\') {
        return (trimmed.to_string(), String::new());
    }

    let slash_index = trimmed.rfind(['/', '\\']);
    match slash_index {
        Some(index) => (
            trimmed[..=index].to_string(),
            trimmed[index + 1..].to_string(),
        ),
        None => (String::new(), trimmed.to_string()),
    }
}
