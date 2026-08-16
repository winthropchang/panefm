use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

#[cfg(test)]
use std::io;

use ignore::WalkBuilder;
use serde::Deserialize;

use super::rg::bundled_rg_command;

/// 表示 global search 面板中的單一搜尋結果。
///
/// 這個結構會保留完整路徑與相對路徑文字，
/// 讓畫面可以直接顯示結果，後續也能用完整路徑做跳轉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalSearchEntry {
    pub(crate) path: PathBuf,
    pub(crate) relative_path: String,
    pub(crate) is_dir: bool,
    pub(crate) match_line_number: Option<usize>,
    pub(crate) match_column: Option<usize>,
    pub(crate) match_preview: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RgJsonEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: RgJsonMatchData,
}

#[derive(Debug, Deserialize, Default)]
struct RgJsonMatchData {
    path: Option<RgJsonTextField>,
    lines: Option<RgJsonTextField>,
    line_number: Option<usize>,
    submatches: Option<Vec<RgJsonSubmatch>>,
}

#[derive(Debug, Deserialize)]
struct RgJsonTextField {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RgJsonSubmatch {
    start: usize,
}

/// 描述背景 global search 分批回傳給主執行緒的訊息類型。
#[derive(Debug)]
pub(crate) enum GlobalSearchEvent {
    Chunk {
        pane_id: usize,
        query: String,
        entries: Vec<GlobalSearchEntry>,
    },
    Done {
        pane_id: usize,
        query: String,
    },
}

#[cfg(test)]
/// 遞迴掃描指定根目錄下的檔案與資料夾，建立 global search 的候選資料集。
///
/// 參數：
/// - `root: &Path`，要遞迴搜尋的根目錄。
/// - `show_hidden: bool`，是否要把隱藏檔一起納入結果。
///
/// 回傳：`io::Result<Vec<GlobalSearchEntry>>`。
/// - 成功時回傳已排序好的搜尋候選清單。
/// - 失敗時回傳掃描目錄時遇到的 I/O 錯誤。
pub(crate) fn collect_search_entries(
    root: &Path,
    show_hidden: bool,
) -> io::Result<Vec<GlobalSearchEntry>> {
    let mut entries = Vec::new();
    let walker = WalkBuilder::new(root).hidden(!show_hidden).build();

    for entry in walker {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path == root {
            continue;
        }

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let is_dir = file_type.is_dir();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let relative_path = if is_dir {
            format!("{relative}/")
        } else {
            relative
        };

        entries.push(GlobalSearchEntry {
            path: path.to_path_buf(),
            relative_path,
            is_dir,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        });
    }

    entries.sort_by(|left, right| {
        left.relative_path
            .to_lowercase()
            .cmp(&right.relative_path.to_lowercase())
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });

    Ok(entries)
}

#[cfg(test)]
/// 依照使用者輸入的查詢文字，過濾目前可顯示的 global search 結果。
///
/// 參數：
/// - `entries: &[GlobalSearchEntry]`，完整候選資料集。
/// - `query: &str`，目前搜尋框中的文字。
/// - `limit: usize`，最多要回傳多少筆結果，避免畫面過重。
///
/// 回傳：`Vec<GlobalSearchEntry>`，已過濾好的搜尋結果。
pub(crate) fn filter_search_entries(
    entries: &[GlobalSearchEntry],
    query: &str,
    limit: usize,
) -> Vec<GlobalSearchEntry> {
    let trimmed = query.trim();
    let query_lower = trimmed.to_lowercase();

    entries
        .iter()
        .filter(|entry| {
            if query_lower.is_empty() {
                true
            } else {
                entry.relative_path.to_lowercase().contains(&query_lower)
            }
        })
        .take(limit.max(1))
        .cloned()
        .collect()
}

/// 以分批方式掃描搜尋結果，找到一批符合項目就立即透過 channel 傳回。
///
/// 參數：
/// - `pane_id: usize`，這次搜尋所屬的 pane。
/// - `root: &Path`，要遞迴搜尋的根目錄。
/// - `show_hidden: bool`，是否把隱藏檔一起納入搜尋。
/// - `query: &str`，目前要比對的搜尋文字。
/// - `limit: usize`，最多回傳多少筆結果。
/// - `chunk_size: usize`，每次分批回傳的結果數量。
/// - `cancelled: Arc<AtomicBool>`，主執行緒可用來要求背景搜尋提早停止。
/// - `sender: Sender<GlobalSearchEvent>`，用來回傳搜尋進度的 channel。
///
/// 回傳：`()`
pub(crate) fn stream_search_entries(
    pane_id: usize,
    root: &Path,
    show_hidden: bool,
    query: &str,
    limit: usize,
    chunk_size: usize,
    cancelled: Arc<AtomicBool>,
    sender: Sender<GlobalSearchEvent>,
) {
    let walker = WalkBuilder::new(root).hidden(!show_hidden).build();
    let query_lower = query.trim().to_lowercase();
    let mut batch = Vec::new();
    let mut matched = 0usize;
    let mut last_flush_at = Instant::now();

    for entry in walker {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }

        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path == root {
            continue;
        }

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let is_dir = file_type.is_dir();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let relative_path = if is_dir {
            format!("{relative}/")
        } else {
            relative
        };

        if !query_lower.is_empty() && !relative_path.to_lowercase().contains(&query_lower) {
            continue;
        }

        batch.push(GlobalSearchEntry {
            path: path.to_path_buf(),
            relative_path,
            is_dir,
            match_line_number: None,
            match_column: None,
            match_preview: None,
        });
        matched += 1;

        if should_flush_batch(&batch, chunk_size, &mut last_flush_at) {
            flush_search_batch(pane_id, query, &mut batch, &cancelled, &sender);
        }

        if matched >= limit.max(1) {
            break;
        }
    }

    if !batch.is_empty() {
        flush_search_batch(pane_id, query, &mut batch, &cancelled, &sender);
    }

    if cancelled.load(Ordering::Relaxed) {
        return;
    }
    let _ = sender.send(GlobalSearchEvent::Done {
        pane_id,
        query: query.to_string(),
    });
}

/// 以分批方式掃描檔案內容，找到至少一個文字命中的檔案就立即回傳。
///
/// 規則：
/// - 只搜尋檔案，不回傳資料夾。
/// - 以不分大小寫方式比對內容。
/// - 若檔案含有 NUL byte，視為 binary 檔並略過，避免把大量二進位資料灌進搜尋。
pub(crate) fn stream_content_search_entries(
    pane_id: usize,
    root: &Path,
    show_hidden: bool,
    query: &str,
    limit: usize,
    chunk_size: usize,
    cancelled: Arc<AtomicBool>,
    sender: Sender<GlobalSearchEvent>,
) {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        let _ = sender.send(GlobalSearchEvent::Done {
            pane_id,
            query: query.to_string(),
        });
        return;
    }

    if stream_content_search_entries_with_rg(
        pane_id,
        root,
        show_hidden,
        query,
        limit,
        chunk_size,
        cancelled.clone(),
        &sender,
    ) {
        return;
    }

    let walker = WalkBuilder::new(root).hidden(!show_hidden).build();
    let query_lower = trimmed.to_lowercase();
    let mut batch = Vec::new();
    let mut matched = 0usize;
    let content_chunk_size = chunk_size.clamp(8, 64);
    let mut last_flush_at = Instant::now();

    for entry in walker {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }

        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path == root {
            continue;
        }

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            continue;
        }

        let Some((match_line_number, match_column, match_preview)) =
            find_query_match_in_file(path, &query_lower, &cancelled)
        else {
            continue;
        };

        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        batch.push(GlobalSearchEntry {
            path: path.to_path_buf(),
            relative_path,
            is_dir: false,
            match_line_number: Some(match_line_number),
            match_column: Some(match_column),
            match_preview: Some(match_preview),
        });
        matched += 1;

        if should_flush_batch(&batch, content_chunk_size, &mut last_flush_at) {
            flush_search_batch(pane_id, query, &mut batch, &cancelled, &sender);
        }

        if matched >= limit.max(1) {
            break;
        }
    }

    if !batch.is_empty() {
        flush_search_batch(pane_id, query, &mut batch, &cancelled, &sender);
    }

    if cancelled.load(Ordering::Relaxed) {
        return;
    }
    let _ = sender.send(GlobalSearchEvent::Done {
        pane_id,
        query: query.to_string(),
    });
}

/// 優先使用 ripgrep 做內容搜尋；若成功啟動並完成，回傳 `true`。
fn stream_content_search_entries_with_rg(
    pane_id: usize,
    root: &Path,
    show_hidden: bool,
    query: &str,
    limit: usize,
    chunk_size: usize,
    cancelled: Arc<AtomicBool>,
    sender: &Sender<GlobalSearchEvent>,
) -> bool {
    let Ok(mut command) = build_rg_content_search_command(root, show_hidden, query, limit) else {
        return false;
    };
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    };

    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut batch = Vec::new();
    let mut last_flush_at = Instant::now();
    let mut sent_anything = false;
    let max_results = limit.max(1);
    let content_chunk_size = chunk_size.clamp(8, 64);
    let mut matched = 0usize;
    let mut stopped_after_limit = false;

    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return true;
        }

        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            let _ = child.kill();
            let _ = child.wait();
            return false;
        };
        if read == 0 {
            break;
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }

        let Ok(event) = serde_json::from_str::<RgJsonEvent>(trimmed) else {
            continue;
        };
        if event.event_type != "match" {
            continue;
        }

        let Some(path_text) = event
            .data
            .path
            .as_ref()
            .and_then(|path| path.text.as_ref())
            .cloned()
        else {
            continue;
        };

        let path = PathBuf::from(path_text);
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let match_preview = event
            .data
            .lines
            .as_ref()
            .and_then(|lines| lines.text.as_deref())
            .map(normalize_match_preview);
        let match_column = event
            .data
            .submatches
            .as_ref()
            .and_then(|submatches| submatches.first())
            .map(|submatch| submatch.start.saturating_add(1));

        batch.push(GlobalSearchEntry {
            path,
            relative_path,
            is_dir: false,
            match_line_number: event.data.line_number,
            match_column,
            match_preview,
        });
        sent_anything = true;
        matched += 1;

        if should_flush_batch(&batch, content_chunk_size, &mut last_flush_at) {
            flush_search_batch(pane_id, query, &mut batch, &cancelled, sender);
        }
        if matched >= max_results {
            let _ = child.kill();
            stopped_after_limit = true;
            break;
        }
    }

    if !batch.is_empty() {
        flush_search_batch(pane_id, query, &mut batch, &cancelled, sender);
    }

    let Ok(status) = child.wait() else {
        return false;
    };
    if cancelled.load(Ordering::Relaxed) {
        return true;
    }

    // rg 在沒找到結果時會回傳 exit code 1，這仍是正常完成。
    if stopped_after_limit || status.success() || status.code() == Some(1) {
        let _ = sender.send(GlobalSearchEvent::Done {
            pane_id,
            query: query.to_string(),
        });
        return true;
    }

    sent_anything
}

/// 建立給 ripgrep 使用的內容搜尋命令。
fn build_rg_content_search_command(
    root: &Path,
    show_hidden: bool,
    query: &str,
    _limit: usize,
) -> Result<Command, std::io::Error> {
    let program = bundled_rg_command().unwrap_or_else(|_| "rg".into());
    let mut command = Command::new(program);
    command
        .arg("--json")
        .arg("--no-messages")
        .arg("--fixed-strings")
        .arg("--ignore-case")
        .arg("--max-count")
        .arg("1")
        .arg("--max-filesize")
        .arg("2M")
        .arg("--path-separator")
        .arg("/");

    if show_hidden {
        command.arg("--hidden");
    }

    command.arg("--").arg(query).arg(root);

    command.stdout(Stdio::piped()).stderr(Stdio::null());
    Ok(command)
}

/// 將 rg 回傳的命中行文字整理成適合列表顯示的摘要。
fn normalize_match_preview(raw: &str) -> String {
    let single_line = raw
        .trim_end_matches(['\r', '\n'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let max_chars = 120usize;
    let mut preview = single_line.chars().take(max_chars).collect::<String>();
    if single_line.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

/// 在單一檔案中找出第一個命中位置，並回傳行號、欄位與摘要。
fn find_query_match_in_file(
    path: &Path,
    query_lower: &str,
    cancelled: &AtomicBool,
) -> Option<(usize, usize, String)> {
    let Ok(file) = File::open(path) else {
        return None;
    };
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut line_number = 0usize;

    loop {
        if cancelled.load(Ordering::Relaxed) {
            return None;
        }

        buffer.clear();
        let Ok(read) = reader.read_until(b'\n', &mut buffer) else {
            return None;
        };
        if read == 0 {
            return None;
        }
        line_number += 1;
        if buffer.contains(&0) {
            return None;
        }
        let line = String::from_utf8_lossy(&buffer);
        let line_lower = line.to_lowercase();
        if let Some(byte_index) = line_lower.find(query_lower) {
            let column = line[..byte_index].chars().count().saturating_add(1);
            return Some((line_number, column, normalize_match_preview(&line)));
        }
    }
}

/// 判斷目前累積的搜尋結果是否應立刻回傳給主執行緒。
fn should_flush_batch(
    batch: &[GlobalSearchEntry],
    chunk_size: usize,
    last_flush_at: &mut Instant,
) -> bool {
    if batch.is_empty() {
        return false;
    }

    let now = Instant::now();
    let timed_out = now.duration_since(*last_flush_at) >= Duration::from_millis(40);
    let full = batch.len() >= chunk_size.max(1);
    if full || timed_out {
        *last_flush_at = now;
        true
    } else {
        false
    }
}

/// 把目前批次中的搜尋結果排序後送回主執行緒。
fn flush_search_batch(
    pane_id: usize,
    query: &str,
    batch: &mut Vec<GlobalSearchEntry>,
    cancelled: &AtomicBool,
    sender: &Sender<GlobalSearchEvent>,
) {
    batch.sort_by(|left, right| {
        left.relative_path
            .to_lowercase()
            .cmp(&right.relative_path.to_lowercase())
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    if cancelled.load(Ordering::Relaxed) {
        return;
    }
    let _ = sender.send(GlobalSearchEvent::Chunk {
        pane_id,
        query: query.to_string(),
        entries: std::mem::take(batch),
    });
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, atomic::AtomicBool, mpsc};

    use tempfile::tempdir;

    use super::{
        GlobalSearchEvent, build_rg_content_search_command, collect_search_entries,
        filter_search_entries, stream_content_search_entries,
    };
    use crate::file_manager::search::normalize_match_preview;

    #[test]
    /// 驗證 global search 會遞迴收集巢狀目錄中的檔案與資料夾。
    fn collect_search_entries_reads_nested_tree() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("src")).expect("src");
        fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").expect("main");

        let entries = collect_search_entries(dir.path(), false).expect("entries");
        let names: Vec<String> = entries
            .into_iter()
            .map(|entry| entry.relative_path)
            .collect();

        assert!(names.contains(&String::from("src/")));
        assert!(names.contains(&String::from("src/main.rs")));
    }

    #[test]
    /// 驗證 global search 過濾時會以不分大小寫方式比對路徑。
    fn filter_search_entries_matches_case_insensitively() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("Readme.md"), "doc").expect("readme");

        let entries = collect_search_entries(dir.path(), false).expect("entries");
        let matches = filter_search_entries(&entries, "read", 20);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].relative_path, "Readme.md");
    }

    #[test]
    /// 驗證內容搜尋只會回傳真正命中文字內容的檔案，且會略過 binary 檔。
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
                GlobalSearchEvent::Done { .. } => None,
            })
            .collect::<Vec<_>>();

        assert!(!chunk_sizes.is_empty());
        assert_eq!(chunk_sizes.iter().sum::<usize>(), 2);
        assert!(matches!(
            events.last(),
            Some(GlobalSearchEvent::Done { .. })
        ));
    }

    #[test]
    /// 驗證 content search 使用的 rg 命令會把選項放在 `--` 前面，避免被當成查詢字串。
    fn build_rg_content_search_command_places_hidden_before_separator() {
        let dir = tempdir().expect("tempdir");
        let command =
            build_rg_content_search_command(dir.path(), true, "needle", 50).expect("command");
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
    }

    #[test]
    /// 驗證命中行摘要會被整理成單行文字，避免把換行直接帶進結果列表。
    fn normalize_match_preview_collapses_whitespace() {
        let preview = normalize_match_preview(" hello   rust \n  world \r\n");
        assert_eq!(preview, "hello rust world");
    }
}
