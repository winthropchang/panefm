//! 鍵盤事件比對與修飾鍵判讀工具。
//!
//! 負責處理跨終端（如 iTerm2、WezTerm、Windows Terminal）常見的鍵位編碼相容性，
//! 包括大寫/Shift 字元映射、`?` 與 `~` 多格式支援，以及視窗切換之數字解析。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// 這個 helper 主要用在命令列、rename、search 這類文字輸入框。
/// 某些終端在 macOS 上會把 `Shift+6` 回報成 `Char('6') + Shift`，
/// 若直接把底層字元寫進 buffer，就會得到 `6` 而不是 `^`。
pub(crate) fn typed_char_from_key(key: &KeyEvent) -> Option<char> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };

    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }

    if !key.modifiers.contains(KeyModifiers::SHIFT) {
        return Some(c);
    }

    Some(match c {
        'a'..='z' => c.to_ascii_uppercase(),
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '\\' => '|',
        '`' => '~',
        other => other,
    })
}

/// 判斷目前按鍵是否是沒有 modifier 的一般小寫命令鍵。
///
/// 這類按鍵主要用在 normal mode、panel 導航或 Vim 風格命令，
/// 目的是把「文字輸入」與「功能命令」分成兩條不同路徑處理。
pub(crate) fn key_matches_plain_letter(key: &KeyEvent, lowercase: char) -> bool {
    if key.code == KeyCode::Char(lowercase) && key.modifiers.is_empty() {
        return true;
    }

    if !key.modifiers.is_empty() {
        return false;
    }

    matches!(
        (lowercase, key.code),
        ('h', KeyCode::Left) | ('j', KeyCode::Down) | ('k', KeyCode::Up) | ('l', KeyCode::Right)
    )
}

/// 判斷某些終端送出的 `Shift+字母` 是否應視為大寫命令。
///
/// 參數：
/// - `key: &KeyEvent`，目前收到的鍵盤事件。
/// - `uppercase: char`，邏輯上希望匹配的大寫英文字母。
///
/// 回傳：`bool`。
/// - `true` 代表事件要視為這個大寫命令。
/// - `false` 代表不是。
pub(crate) fn key_matches_shifted_letter(key: &KeyEvent, uppercase: char) -> bool {
    let lower = uppercase.to_ascii_lowercase();
    key.code == KeyCode::Char(uppercase)
        || (key.code == KeyCode::Char(lower) && key.modifiers.contains(KeyModifiers::SHIFT))
}

/// 判斷目前按鍵是否應視為 `~`，支援不同終端可能回報的格式差異。
///
/// 常見情況：
/// - 直接回報 `Char('~')`
/// - 回報 `Char('`') + Shift`
pub(crate) fn key_matches_tilde(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('~')
        || (key.code == KeyCode::Char('`') && key.modifiers.contains(KeyModifiers::SHIFT))
}

/// 判斷目前按鍵是否應視為 `?`，支援不同終端可能回報的格式差異（如帶有 Shift 或全形問號）。
///
/// 常見情況：
/// - 直接回報 `Char('?')`（通常帶有 Shift，亦相容無 modifier）
/// - 回報 `Char('/') + Shift`
/// - 全形問號 `Char('？')`
pub(crate) fn key_matches_question_mark(key: &KeyEvent) -> bool {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return false;
    }
    match key.code {
        KeyCode::Char('?' | '？') => true,
        KeyCode::Char('/') if key.modifiers.contains(KeyModifiers::SHIFT) => true,
        _ => false,
    }
}

/// 判斷某個英文字母命令是否要接受大小寫等價輸入。
///
/// 這主要用在 `y/n` 這種確認提示，或某些不區分大小寫的互動按鍵。
pub(crate) fn key_matches_letter_any_case(key: &KeyEvent, letter: char) -> bool {
    let lower = letter.to_ascii_lowercase();
    let upper = letter.to_ascii_uppercase();
    key_matches_plain_letter(key, lower) || key_matches_shifted_letter(key, upper)
}

/// 判斷 `Ctrl+字母` 指令，支援不同終端可能送出的大小寫字元格式。
pub(crate) fn key_matches_ctrl_letter(key: &KeyEvent, letter: char) -> bool {
    let lower = letter.to_ascii_lowercase();
    let upper = letter.to_ascii_uppercase();
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c == lower || c == upper)
}

/// 把 `Ctrl+數字` 轉成目標 pane 編號。
///
/// 目前規則：
/// - `Ctrl+1` 到 `Ctrl+9` 對應 pane 1..9
/// - `Ctrl+0` 對應 pane 10
pub(crate) fn ctrl_digit_target_pane_id(key: &KeyEvent) -> Option<usize> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }

    match key.code {
        KeyCode::Char('1') => Some(1),
        KeyCode::Char('2') => Some(2),
        KeyCode::Char('3') => Some(3),
        KeyCode::Char('4') => Some(4),
        KeyCode::Char('5') => Some(5),
        KeyCode::Char('6') => Some(6),
        KeyCode::Char('7') => Some(7),
        KeyCode::Char('8') => Some(8),
        KeyCode::Char('9') => Some(9),
        KeyCode::Char('0') => Some(10),
        _ => None,
    }
}

/// 把不帶修飾鍵的數字轉成目標 pane 編號，供多 pane 模式直接切換焦點。
///
/// 目前規則：
/// - `1` 到 `9` 對應 pane 1..9
/// - `0` 對應 pane 10
pub(crate) fn plain_digit_target_pane_id(key: &KeyEvent) -> Option<usize> {
    if !key.modifiers.is_empty() {
        return None;
    }

    match key.code {
        KeyCode::Char('1') => Some(1),
        KeyCode::Char('2') => Some(2),
        KeyCode::Char('3') => Some(3),
        KeyCode::Char('4') => Some(4),
        KeyCode::Char('5') => Some(5),
        KeyCode::Char('6') => Some(6),
        KeyCode::Char('7') => Some(7),
        KeyCode::Char('8') => Some(8),
        KeyCode::Char('9') => Some(9),
        KeyCode::Char('0') => Some(10),
        _ => None,
    }
}
