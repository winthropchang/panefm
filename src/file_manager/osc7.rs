//! OSC 7（Operating System Command 7）終端工作目錄同步協議。
//!
//! 透過 ANSI 控制碼向外層宿主終端（WezTerm, Windows Terminal, Alacritty, Ghostty,
//! iTerm2, Kitty 等）通報目前 active panel 的目錄，讓使用者按下終端原生開分頁或
//! 開新視窗快捷鍵時，新終端會自動位於該目錄。

use std::{
    io::{self, Write},
    path::Path,
};

/// 將本機路徑轉為 RFC 8089 / OSC 7 相容的 Percent-Encoded URI 路徑。
///
/// 參數：`path: &Path`，要編碼的目錄路徑。
/// 回傳：`String`，開頭包含 `/` 且非保留字元完成 `%XX` 編碼的路徑字串。
pub(crate) fn encode_path_for_osc7(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let normalized = raw.replace('\\', "/");
    let with_leading_slash = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{normalized}")
    };

    let mut encoded = String::with_capacity(with_leading_slash.len() * 2);
    for byte in with_leading_slash.bytes() {
        match byte {
            // URI path unreserved characters & separators
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                encoded.push(byte as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

/// 產生標準 OSC 7 escape sequence，向 host 終端同步目錄。
///
/// 格式為：`\x1b]7;file://localhost{encoded_path}\x1b\`
pub(crate) fn format_osc7_sequence(path: &Path) -> String {
    let encoded_path = encode_path_for_osc7(path);
    format!("\x1b]7;file://localhost{encoded_path}\x1b\\")
}

/// 將目前工作目錄以 OSC 7 控制碼寫入終端輸出串流。
pub(crate) fn sync_terminal_working_directory<W: Write>(
    writer: &mut W,
    cwd: &Path,
) -> io::Result<()> {
    let sequence = format_osc7_sequence(cwd);
    writer.write_all(sequence.as_bytes())?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/osc7_test.rs"]
mod tests;
