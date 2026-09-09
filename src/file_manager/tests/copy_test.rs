use std::path::PathBuf;

use super::{CopyAction, build_copy_text, copy_action_status_label};
use crate::file_manager::open::OpenTarget;

#[test]
/// 驗證複製檔案路徑保留原生絕對路徑，不轉成 percent-encoded URL。
/// 保護目的：避免剪貼簿文字格式調整後，產生其他終端、IDE 或檔案工具無法直接使用的路徑。
fn copy_file_path_preserves_plain_absolute_path() {
    let target = OpenTarget {
        path: PathBuf::from("/tmp/hello world.txt"),
        display_name: String::from("hello world.txt"),
        is_dir: false,
    };

    let text = build_copy_text(&target, CopyAction::FileUrl).expect("file path");
    assert_eq!(text, "/tmp/hello world.txt");
}

#[test]
/// 驗證檔案的 directory copy 取父目錄，而目錄目標則保留自己。
/// 保護目的：避免剪貼簿文字格式調整後，產生其他終端、IDE 或檔案工具無法直接使用的路徑。
fn copy_directory_path_uses_parent_for_files() {
    let target = OpenTarget {
        path: PathBuf::from("/tmp/docs/readme.md"),
        display_name: String::from("readme.md"),
        is_dir: false,
    };

    let text = build_copy_text(&target, CopyAction::DirectoryUrl).expect("dir path");
    assert_eq!(text, "/tmp/docs");
}

#[test]
/// 驗證 filename-without-extension 只移除檔案副檔名，不修改目錄名稱。
/// 保護目的：避免剪貼簿文字格式調整後，產生其他終端、IDE 或檔案工具無法直接使用的路徑。
fn copy_filename_without_extension_uses_stem_for_files() {
    let target = OpenTarget {
        path: PathBuf::from("/tmp/archive.tar.gz"),
        display_name: String::from("archive.tar.gz"),
        is_dir: false,
    };

    let text =
        build_copy_text(&target, CopyAction::FilenameWithoutExtension).expect("filename stem");
    assert_eq!(text, "archive.tar");
}

#[test]
/// 驗證每種 copy action 都提供可直接顯示在狀態列的清楚訊息。
/// 保護目的：避免剪貼簿文字格式調整後，產生其他終端、IDE 或檔案工具無法直接使用的路徑。
fn copy_status_labels_are_human_readable() {
    assert_eq!(
        copy_action_status_label(CopyAction::FileUrl),
        "copied file path"
    );
    assert_eq!(
        copy_action_status_label(CopyAction::FilenameWithoutExtension),
        "copied filename without extension"
    );
}
