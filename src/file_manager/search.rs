use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
};

#[cfg(test)]
use std::io;

use ignore::WalkBuilder;

/// 表示 global search 面板中的單一搜尋結果。
///
/// 這個結構會保留完整路徑與相對路徑文字，
/// 讓畫面可以直接顯示結果，後續也能用完整路徑做跳轉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalSearchEntry {
    pub(crate) path: PathBuf,
    pub(crate) relative_path: String,
    pub(crate) is_dir: bool,
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
        });
        matched += 1;

        if batch.len() >= chunk_size.max(1) {
            batch.sort_by(|left, right| {
                left.relative_path
                    .to_lowercase()
                    .cmp(&right.relative_path.to_lowercase())
                    .then_with(|| left.relative_path.cmp(&right.relative_path))
            });
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            if sender
                .send(GlobalSearchEvent::Chunk {
                    pane_id,
                    query: query.to_string(),
                    entries: std::mem::take(&mut batch),
                })
                .is_err()
            {
                return;
            }
        }

        if matched >= limit.max(1) {
            break;
        }
    }

    if !batch.is_empty() {
        batch.sort_by(|left, right| {
            left.relative_path
                .to_lowercase()
                .cmp(&right.relative_path.to_lowercase())
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        if sender
            .send(GlobalSearchEvent::Chunk {
                pane_id,
                query: query.to_string(),
                entries: batch,
            })
            .is_err()
        {
            return;
        }
    }

    if cancelled.load(Ordering::Relaxed) {
        return;
    }
    let _ = sender.send(GlobalSearchEvent::Done {
        pane_id,
        query: query.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{collect_search_entries, filter_search_entries};

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
}
