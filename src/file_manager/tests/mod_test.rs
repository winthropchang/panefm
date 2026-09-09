use std::fs;
use std::sync::atomic::AtomicBool;

use crossterm::event::KeyEventKind;
use tempfile::tempdir;

use super::{
    format_helper_bool_flag, fzf_bindings, parse_helper_bool_flag, should_handle_key_event,
    stream_fzf_candidates,
};

/// 驗證主事件迴圈只會處理按下或長按重複事件，避免放開事件造成快捷鍵重複觸發。
///
/// 參數：無。
/// 回傳：無。
/// 保護目的：避免終端事件正規化調整後，一次實體按鍵因 Release 事件而執行兩次操作。
#[test]
fn should_handle_only_press_and_repeat_key_events() {
    assert!(should_handle_key_event(KeyEventKind::Press));
    assert!(should_handle_key_event(KeyEventKind::Repeat));
    assert!(!should_handle_key_event(KeyEventKind::Release));
}

#[test]
/// 驗證 fzf 候選串流會遞迴輸出相對路徑，而不是只列第一層目錄。
/// 保護目的：避免終端生命週期或外部工具整合調整後，破壞候選串流、Esc 取消或 helper 參數契約。
fn stream_fzf_candidates_writes_nested_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
    fs::create_dir(dir.path().join("docs")).expect("docs");
    fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

    let cancel = AtomicBool::new(false);
    let mut output = Vec::new();
    stream_fzf_candidates(dir.path(), false, true, &cancel, &mut output).expect("stream");
    let text = String::from_utf8(output).expect("utf8");

    assert!(text.contains("alpha.txt\n"));
    assert!(text.contains("docs/\n"));
    assert!(text.contains("docs/guide.md\n"));
}

#[test]
/// 驗證啟用 hidden 選項後，fzf 候選會包含點號開頭項目。
/// 保護目的：避免終端生命週期或外部工具整合調整後，破壞候選串流、Esc 取消或 helper 參數契約。
fn stream_fzf_candidates_includes_hidden_entries_when_enabled() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".secret.txt"), "secret").expect("secret");
    fs::create_dir(dir.path().join(".cache")).expect("cache");
    fs::write(dir.path().join(".cache").join("item.txt"), "item").expect("item");

    let cancel = AtomicBool::new(false);
    let mut output = Vec::new();
    stream_fzf_candidates(dir.path(), true, true, &cancel, &mut output).expect("stream");
    let text = String::from_utf8(output).expect("utf8");

    assert!(text.contains(".secret.txt\n"));
    assert!(text.contains(".cache/\n"));
    assert!(text.contains(".cache/item.txt\n"));
}

#[cfg(unix)]
#[test]
/// 驗證 follow-links 選項能走訪符號連結目錄，同時避免無限制遞迴。
/// 保護目的：避免終端生命週期或外部工具整合調整後，破壞候選串流、Esc 取消或 helper 參數契約。
fn stream_fzf_candidates_can_follow_symlinked_directories() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().expect("tempdir");
    let real_dir = dir.path().join("real");
    fs::create_dir(&real_dir).expect("real");
    fs::write(real_dir.join("nested.txt"), "nested").expect("nested");
    symlink(&real_dir, dir.path().join("linked")).expect("symlink");

    let cancel = AtomicBool::new(false);
    let mut without_follow = Vec::new();
    stream_fzf_candidates(dir.path(), true, false, &cancel, &mut without_follow).expect("stream");
    let without_follow = String::from_utf8(without_follow).expect("utf8");
    assert!(without_follow.contains("linked\n") || without_follow.contains("linked/\n"));
    assert!(!without_follow.contains("linked/nested.txt\n"));

    let cancel = AtomicBool::new(false);
    let mut with_follow = Vec::new();
    stream_fzf_candidates(dir.path(), true, true, &cancel, &mut with_follow).expect("stream");
    let with_follow = String::from_utf8(with_follow).expect("utf8");
    assert!(with_follow.contains("linked/nested.txt\n"));
}

#[test]
/// 驗證啟動 fzf 時明確綁定 Esc abort，讓使用者可可靠回到 PaneFM。
/// 保護目的：避免終端生命週期或外部工具整合調整後，破壞候選串流、Esc 取消或 helper 參數契約。
fn fzf_bindings_include_escape_abort() {
    assert!(fzf_bindings().contains("esc:abort"));
    assert!(fzf_bindings().contains("enter:accept"));
}

#[test]
/// 驗證傳給候選 helper process 的布林旗標可完整編碼再解析。
/// 保護目的：避免終端生命週期或外部工具整合調整後，破壞候選串流、Esc 取消或 helper 參數契約。
fn helper_bool_flags_round_trip() {
    assert_eq!(format_helper_bool_flag(true), "1");
    assert_eq!(format_helper_bool_flag(false), "0");
    assert!(parse_helper_bool_flag(Some("1".into()), "flag").expect("true"));
    assert!(!parse_helper_bool_flag(Some("0".into()), "flag").expect("false"));
}

#[test]
/// 驗證候選 helper 遇到未知布林值會拒絕執行，不默默採用錯誤預設值。
/// 保護目的：避免終端生命週期或外部工具整合調整後，破壞候選串流、Esc 取消或 helper 參數契約。
fn helper_bool_flags_reject_invalid_values() {
    assert!(parse_helper_bool_flag(Some("yes".into()), "flag").is_err());
    assert!(parse_helper_bool_flag(None, "flag").is_err());
}
