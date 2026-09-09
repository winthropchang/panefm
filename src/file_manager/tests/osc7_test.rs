use std::path::PathBuf;

use super::{encode_path_for_osc7, format_osc7_sequence};

#[test]
/// 驗證 Windows 磁碟機代號路徑會正確正規化為前綴斜線並保留代號與冒號。
fn osc7_formats_windows_drive_path() {
    let path = PathBuf::from(r"C:\Users\otto\project");
    let encoded = encode_path_for_osc7(&path);
    assert_eq!(encoded, "/C:/Users/otto/project");
    assert_eq!(
        format_osc7_sequence(&path),
        "\x1b]7;file://localhost/C:/Users/otto/project\x1b\\"
    );
}

#[test]
/// 驗證包含空白與特殊字元的路徑會正確進行 Percent-Encoding。
fn osc7_encodes_spaces_and_special_characters() {
    let path = PathBuf::from("/Users/otto/My Documents/Special & Cool Project");
    let encoded = encode_path_for_osc7(&path);
    assert_eq!(
        encoded,
        "/Users/otto/My%20Documents/Special%20%26%20Cool%20Project"
    );
    assert_eq!(
        format_osc7_sequence(&path),
        "\x1b]7;file://localhost/Users/otto/My%20Documents/Special%20%26%20Cool%20Project\x1b\\"
    );
}

#[test]
/// 驗證標準 POSIX Unix 路徑不會遺失開頭斜線。
fn osc7_formats_posix_path() {
    let path = PathBuf::from("/etc/nginx/sites-available");
    let encoded = encode_path_for_osc7(&path);
    assert_eq!(encoded, "/etc/nginx/sites-available");
    assert_eq!(
        format_osc7_sequence(&path),
        "\x1b]7;file://localhost/etc/nginx/sites-available\x1b\\"
    );
}
