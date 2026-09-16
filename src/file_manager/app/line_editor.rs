use super::*;

/// 依照編輯模式決定建立輸入框的標題文字。
pub(crate) fn create_editor_title(mode: RenameMode) -> &'static str {
    match mode {
        RenameMode::Insert => " Create (append / for dir): ",
        RenameMode::Normal => " Create (normal): ",
    }
}

/// 將字元插入到指定的字元游標位置，並在插入後把游標往右移一格。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的檔名字串。
/// - `cursor: &mut usize`，以字元數計算的游標位置。
/// - `ch: char`，要插入的新字元。
///
/// 回傳：`()`
pub(crate) fn insert_char(buffer: &mut String, cursor: &mut usize, ch: char) {
    let byte_index = char_to_byte_index(buffer, *cursor);
    buffer.insert(byte_index, ch);
    *cursor += 1;
}

/// 將字串插入到指定的字元游標位置，並在插入後把游標往右移該字串的字元數。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的文字字串。
/// - `cursor: &mut usize`，以字元數計算的游標位置。
/// - `text: &str`，要插入的新字串。
///
/// 回傳：`()`
pub(crate) fn insert_str(buffer: &mut String, cursor: &mut usize, text: &str) {
    if text.is_empty() {
        return;
    }
    let byte_index = char_to_byte_index(buffer, *cursor);
    buffer.insert_str(byte_index, text);
    *cursor += text.chars().count();
}

/// 清理從剪貼簿或終端貼上的文字，移除結尾多餘換行，並將內部換行轉換為空白，確保單行文字輸入框不會被破壞。
///
/// 參數：
/// - `text: &str`，由系統剪貼簿或終端事件取得的原始文字。
///
/// 回傳：`String`，清理後可安全插入單行輸入框的字串。
pub(crate) fn sanitize_pasted_text(text: &str) -> String {
    let trimmed = text.trim_end_matches(['\r', '\n']);
    trimmed.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// 刪除游標左側的一個字元，行為對齊一般文字編輯器的 Backspace。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的檔名字串。
/// - `cursor: &mut usize`，以字元數計算的游標位置。
///
/// 回傳：`()`
pub(crate) fn backspace_char(buffer: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }

    let remove_at = *cursor - 1;
    let start = char_to_byte_index(buffer, remove_at);
    let end = char_to_byte_index(buffer, *cursor);
    buffer.replace_range(start..end, "");
    *cursor -= 1;
}

/// 刪除字元游標目前指向的字元，供共用輸入器處理 Delete 鍵。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的 UTF-8 字串。
/// - `cursor: usize`，以字元數表示的游標位置。
///
/// 回傳：`()`, 游標在字串尾端時不修改內容。
pub(crate) fn delete_char_at(buffer: &mut String, cursor: usize) {
    if cursor >= buffer.chars().count() {
        return;
    }
    let start = char_to_byte_index(buffer, cursor);
    let end = char_to_byte_index(buffer, cursor + 1);
    buffer.replace_range(start..end, "");
}

/// 把字元游標位置轉成 Rust 字串的位元組索引，方便做安全的字串編輯。
///
/// 參數：
/// - `text: &str`，來源字串。
/// - `char_index: usize`，以字元數表示的目標位置。
///
/// 回傳：`usize`，對應的位元組索引。
pub(crate) fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

/// 將游標往右移動一個字元，但不會超過字串末端。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，移動後的新游標位置。
pub(crate) fn move_cursor_right(buffer: &str, cursor: usize) -> usize {
    let end = buffer.chars().count();
    (cursor + 1).min(end)
}

/// 將 Insert 模式的插入點轉成 Normal 模式可指向的字元位置。
pub(crate) fn normal_cursor(buffer: &str, cursor: usize) -> usize {
    cursor.min(buffer.chars().count().saturating_sub(1))
}

/// 在 Normal 模式向右移動一格，但不會移到最後一個字元之外。
pub(crate) fn normal_move_right(buffer: &str, cursor: usize) -> usize {
    (cursor + 1).min(buffer.chars().count().saturating_sub(1))
}

/// 回傳 normal 模式中 `$` 應該停留的位置，也就是最後一個可見字元。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
///
/// 回傳：`usize`，normal 模式游標應停留的字元索引。
pub(crate) fn rename_line_end_cursor(buffer: &str) -> usize {
    buffer.chars().count().saturating_sub(1)
}

/// 判斷某個字元是否應該被視為檔名中的「單字內容」。
///
/// 參數：
/// - `ch: char`，要判斷的字元。
///
/// 回傳：`bool`。
/// - `true` 代表它屬於單字本體，例如英數字。
/// - `false` 代表它是分隔符，例如 `-`、`_`、`.` 或空白。
pub(crate) fn is_rename_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
}

/// 找出 normal 模式下 `w` 應跳到的位置，也就是下一段名稱的開頭。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，下一個單字的起始字元索引。
pub(crate) fn rename_next_word_start(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 0;
    }

    let mut index = cursor.min(len);
    if index < len && is_rename_word_char(chars[index]) {
        while index < len && is_rename_word_char(chars[index]) {
            index += 1;
        }
    }

    while index < len && !is_rename_word_char(chars[index]) {
        index += 1;
    }

    index.min(rename_line_end_cursor(buffer))
}

/// 找出 normal 模式下 `b` 應跳到的位置，也就是前一段名稱的開頭。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，前一個單字的起始字元索引。
pub(crate) fn rename_previous_word_start(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    if chars.is_empty() {
        return 0;
    }

    let mut index = cursor.min(chars.len().saturating_sub(1));
    if !is_rename_word_char(chars[index]) {
        while index > 0 && !is_rename_word_char(chars[index]) {
            index -= 1;
        }
        if index == 0 && !is_rename_word_char(chars[index]) {
            return 0;
        }
    } else if index > 0 {
        index -= 1;
        while index > 0 && !is_rename_word_char(chars[index]) {
            index -= 1;
        }
        if index == 0 && !is_rename_word_char(chars[index]) {
            return 0;
        }
    }

    while index > 0 && is_rename_word_char(chars[index - 1]) {
        index -= 1;
    }

    index
}

/// 找出 normal 模式下 `e` 應跳到的位置，也就是目前或下一段名稱的結尾。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，目標單字最後一個字元的索引。
pub(crate) fn rename_word_end(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 0;
    }

    let mut index = cursor.min(len.saturating_sub(1));
    while index < len && !is_rename_word_char(chars[index]) {
        index += 1;
    }

    if index >= len {
        return rename_line_end_cursor(buffer);
    }

    while index + 1 < len && is_rename_word_char(chars[index + 1]) {
        index += 1;
    }

    index
}

/// 找出 rename 一開始應該停留的位置，預設會停在副檔名前，方便先改主檔名。
///
/// 參數：
/// - `name: &str`，目前選取項目的原始名稱。
///
/// 回傳：`usize`，以字元數表示的初始游標位置。
pub(crate) fn rename_basename_cursor(name: &str) -> usize {
    let dot_index = name.rfind('.').filter(|index| *index > 0);
    match dot_index {
        Some(index) => name[..index].chars().count(),
        None => name.chars().count(),
    }
}
