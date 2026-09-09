use std::fs;
use std::sync::{Arc, atomic::AtomicBool, mpsc};

use tempfile::tempdir;

use super::{
    GlobalSearchEvent, build_fd_search_command, build_rg_content_search_command,
    stream_content_search_entries, stream_search_entries,
};
use crate::file_manager::search::normalize_match_preview;

#[test]
/// 驗證檔名搜尋的第一筆命中會獨立送出，不會等待批次填滿或完整掃描結束。
/// 保護目的：避免搜尋命令或串流解析調整後，延遲首批結果、遺失資料或改變結果語意。
fn stream_search_entries_emits_first_match_immediately() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("887-first.txt"), "first").expect("first");
    fs::write(dir.path().join("887-second.txt"), "second").expect("second");

    let (tx, rx) = mpsc::channel();
    stream_search_entries(
        1,
        dir.path(),
        false,
        "887",
        20,
        100,
        Arc::new(AtomicBool::new(false)),
        tx,
    );

    let events = rx.try_iter().collect::<Vec<_>>();
    let chunks = events
        .iter()
        .filter_map(|event| match event {
            GlobalSearchEvent::Chunk { entries, .. } => Some(entries),
            GlobalSearchEvent::Done { .. } | GlobalSearchEvent::MissingTool { .. } => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].len(), 1);
    assert_eq!(chunks.iter().map(|entries| entries.len()).sum::<usize>(), 2);
    assert!(matches!(
        events.last(),
        Some(GlobalSearchEvent::Done { .. })
    ));
}

#[test]
/// 驗證檔名搜尋會使用 fd 的固定字串、不分大小寫與安全路徑分隔選項。
/// 保護目的：避免搜尋命令或串流解析調整後，延遲首批結果、遺失資料或改變結果語意。
fn build_fd_search_command_uses_streaming_safe_options() {
    let dir = tempdir().expect("tempdir");
    let command = build_fd_search_command(dir.path(), true, "887").expect("fd command");
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();

    assert!(args.iter().any(|arg| arg == "--print0"));
    assert!(args.iter().any(|arg| arg == "--ignore-case"));
    assert!(args.iter().any(|arg| arg == "--fixed-strings"));
    assert!(args.iter().any(|arg| arg == "--hidden"));
    assert_eq!(args[args.len() - 2], "887");
    assert_eq!(args.last().map(String::as_str), Some("."));
    assert_eq!(command.get_current_dir(), Some(dir.path()));
}

#[test]
/// 驗證內容搜尋只會回傳真正命中文字內容的檔案，且會略過 binary 檔。
/// 保護目的：避免搜尋命令或串流解析調整後，延遲首批結果、遺失資料或改變結果語意。
fn stream_content_search_entries_matches_file_contents() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("notes.txt"), "hello rust world\n").expect("notes");
    fs::write(dir.path().join("other.txt"), "python only\n").expect("other");
    fs::write(dir.path().join("image.bin"), [0, 159, 146, 150]).expect("binary");

    let (tx, rx) = mpsc::channel();
    stream_content_search_entries(
        1,
        dir.path(),
        false,
        "rust",
        20,
        10,
        Arc::new(AtomicBool::new(false)),
        tx,
    );

    let mut results = Vec::new();
    for event in rx.try_iter() {
        if let GlobalSearchEvent::Chunk { entries, .. } = event {
            results.extend(entries);
        }
    }

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].relative_path, "notes.txt");
    assert_eq!(results[0].match_line_number, Some(1));
    assert_eq!(results[0].match_column, Some(7));
    assert_eq!(
        results[0].match_preview.as_deref(),
        Some("hello rust world")
    );
}

#[test]
/// 驗證內容搜尋會把命中的檔案逐步分批送回，而不是全部累積到最後才一次回傳。
/// 保護目的：避免搜尋命令或串流解析調整後，延遲首批結果、遺失資料或改變結果語意。
fn stream_content_search_entries_emits_incremental_chunks() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("alpha.txt"), "rust alpha\n").expect("alpha");
    fs::write(dir.path().join("beta.txt"), "rust beta\n").expect("beta");

    let (tx, rx) = mpsc::channel();
    stream_content_search_entries(
        1,
        dir.path(),
        false,
        "rust",
        20,
        24,
        Arc::new(AtomicBool::new(false)),
        tx,
    );

    let events: Vec<GlobalSearchEvent> = rx.try_iter().collect();
    let chunk_sizes = events
        .iter()
        .filter_map(|event| match event {
            GlobalSearchEvent::Chunk { entries, .. } => Some(entries.len()),
            GlobalSearchEvent::Done { .. } | GlobalSearchEvent::MissingTool { .. } => None,
        })
        .collect::<Vec<_>>();

    assert!(!chunk_sizes.is_empty());
    assert_eq!(chunk_sizes.iter().sum::<usize>(), 2);
    assert_eq!(chunk_sizes[0], 1);
    assert!(matches!(
        events.last(),
        Some(GlobalSearchEvent::Done { .. })
    ));
}

#[test]
/// 驗證 content search 使用的 rg 命令會把選項放在 `--` 前面，避免被當成查詢字串。
/// 保護目的：避免搜尋命令或串流解析調整後，延遲首批結果、遺失資料或改變結果語意。
fn build_rg_content_search_command_places_hidden_before_separator() {
    let dir = tempdir().expect("tempdir");
    let command = build_rg_content_search_command(dir.path(), true, "needle", 50).expect("command");
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();

    let hidden_index = args
        .iter()
        .position(|arg| arg == "--hidden")
        .expect("hidden");
    let separator_index = args.iter().position(|arg| arg == "--").expect("separator");
    assert!(hidden_index < separator_index);
    assert_eq!(args[separator_index + 1], "needle");
    assert!(args.iter().any(|arg| arg == "--json"));
    assert!(args.iter().any(|arg| arg == "--line-buffered"));
    let glob_index = args
        .iter()
        .position(|arg| arg == "--glob")
        .expect("git exclusion glob");
    assert_eq!(args[glob_index + 1], "!.git");
}

#[test]
/// 驗證未開啟隱藏檔時，不會額外加入 Git 排除規則，避免改變既有設定語意。
/// 保護目的：避免搜尋命令或串流解析調整後，延遲首批結果、遺失資料或改變結果語意。
fn build_rg_content_search_command_does_not_add_hidden_glob_when_disabled() {
    let dir = tempdir().expect("tempdir");
    let command =
        build_rg_content_search_command(dir.path(), false, "needle", 50).expect("command");
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();

    assert!(!args.iter().any(|arg| arg == "--hidden"));
    assert!(!args.iter().any(|arg| arg == "--glob"));
}

#[test]
/// 驗證命中行摘要會被整理成單行文字，避免把換行直接帶進結果列表。
/// 保護目的：避免搜尋命令或串流解析調整後，延遲首批結果、遺失資料或改變結果語意。
fn normalize_match_preview_collapses_whitespace() {
    let preview = normalize_match_preview(" hello   rust \n  world \r\n");
    assert_eq!(preview, "hello rust world");
}
