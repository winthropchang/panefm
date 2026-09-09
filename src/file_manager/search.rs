//! `fd` 檔名搜尋與 `rg` 內容搜尋的非同步串流轉接層。
//!
//! 搜尋程序的 stdout 會逐行解析並分批送回 `App`，第一批結果不等待整棵目錄樹
//! 完成。取消旗標同時負責停止讀取與終止 child process；修改此流程時要維持結果
//! 原順序，否則背景批次到達會讓使用者正在操作的游標跳動。

use serde::Deserialize;
use std::{
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

#[cfg(unix)]
use std::ffi::OsString;

use super::{fd::fd_command, rg::rg_command};

/// 表示 global search 面板中的單一搜尋結果。
///
/// 這個結構會保留完整路徑與相對路徑文字，
/// 讓畫面可以直接顯示結果，後續也能用完整路徑做跳轉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalSearchEntry {
    /// 可直接用於跳轉、預覽或開啟的絕對路徑。
    pub(crate) path: PathBuf,
    /// 相對搜尋根目錄的顯示文字，列表只顯示這個欄位以保持簡潔。
    pub(crate) relative_path: String,
    /// `true` 代表結果是目錄；內容搜尋的結果固定為檔案。
    pub(crate) is_dir: bool,
    /// rg 第一個命中的 1-based 行號；檔名搜尋不提供此資訊。
    pub(crate) match_line_number: Option<usize>,
    /// rg 第一個 submatch 的 1-based 欄位位置。
    pub(crate) match_column: Option<usize>,
    /// 經過空白正規化的首筆命中摘要，供小型 preview 快速判斷內容。
    pub(crate) match_preview: Option<String>,
}

/// ripgrep `--json` stdout 的最外層事件格式。
///
/// rg 除了 `match` 還會輸出 begin、end、summary 等事件；解析後會先檢查
/// `event_type`，只有 match 才轉成 PaneFM 的 `GlobalSearchEntry`。
#[derive(Debug, Deserialize)]
struct RgJsonEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: RgJsonMatchData,
}

/// rg match 事件中 PaneFM 實際需要的可選欄位。
///
/// 欄位保持 `Option` 是因為 rg 的 JSON protocol 允許文字改用 base64 表示，且部分
/// 事件沒有行號或 submatch；解析層會安全忽略無法顯示的資料，而不是 panic。
#[derive(Debug, Deserialize, Default)]
struct RgJsonMatchData {
    path: Option<RgJsonTextField>,
    lines: Option<RgJsonTextField>,
    line_number: Option<usize>,
    submatches: Option<Vec<RgJsonSubmatch>>,
}

/// rg JSON protocol 用來包裝 UTF-8 `text`（或未支援的 bytes/base64）的物件。
#[derive(Debug, Deserialize)]
struct RgJsonTextField {
    text: Option<String>,
}

/// 單一 rg submatch 的 byte 起點；目前只取第一筆換算預覽欄位。
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
    MissingTool {
        pane_id: usize,
        query: String,
        tool: String,
    },
}

/// 使用外部 `fd` 遞迴搜尋檔案名稱，並把命中結果即時送回主執行緒。
///
/// 參數：
/// - `pane_id: usize`，這次搜尋所屬的 pane。
/// - `root: &Path`，要遞迴搜尋的根目錄。
/// - `show_hidden: bool`，是否把隱藏檔一起納入搜尋。
/// - `query: &str`，目前要比對的搜尋文字。
/// - `limit: usize`，最多回傳多少筆結果。
/// - `_chunk_size: usize`，保留既有呼叫介面的批次大小；`fd` 結果會逐筆回傳。
/// - `cancelled: Arc<AtomicBool>`，主執行緒可用來要求背景搜尋提早停止。
/// - `sender: Sender<GlobalSearchEvent>`，用來回傳搜尋進度的 channel。
///
/// 回傳：`()`
#[allow(clippy::too_many_arguments)]
pub(crate) fn stream_search_entries(
    pane_id: usize,
    root: &Path,
    show_hidden: bool,
    query: &str,
    limit: usize,
    _chunk_size: usize,
    cancelled: Arc<AtomicBool>,
    sender: Sender<GlobalSearchEvent>,
) {
    let mut command = match build_fd_search_command(root, show_hidden, query) {
        Ok(command) => command,
        Err(_) => {
            let _ = sender.send(GlobalSearchEvent::MissingTool {
                pane_id,
                query: query.to_string(),
                tool: String::from("fd"),
            });
            return;
        }
    };
    let Ok(mut child) = command.spawn() else {
        let _ = sender.send(GlobalSearchEvent::MissingTool {
            pane_id,
            query: query.to_string(),
            tool: String::from("fd"),
        });
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };

    let mut reader = BufReader::new(stdout);
    let mut path_bytes = Vec::new();
    let max_results = limit.max(1);
    let mut matched = 0usize;

    loop {
        if cancelled.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }

        path_bytes.clear();
        let Ok(read) = reader.read_until(0, &mut path_bytes) else {
            let _ = child.kill();
            let _ = child.wait();
            return;
        };
        if read == 0 {
            break;
        }
        if path_bytes.last() == Some(&0) {
            path_bytes.pop();
        }
        if path_bytes.is_empty() {
            continue;
        }

        let fd_path = path_buf_from_fd_output(std::mem::take(&mut path_bytes));
        let relative_fd_path = fd_path.strip_prefix(".").unwrap_or(fd_path.as_path());
        let path = if fd_path.is_absolute() {
            fd_path
        } else {
            root.join(relative_fd_path)
        };
        let is_dir = path.is_dir();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        let relative_path = if is_dir {
            format!("{relative}/")
        } else {
            relative
        };

        let event = GlobalSearchEvent::Chunk {
            pane_id,
            query: query.to_string(),
            entries: vec![GlobalSearchEntry {
                path,
                relative_path,
                is_dir,
                match_line_number: None,
                match_column: None,
                match_preview: None,
            }],
        };
        if sender.send(event).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }

        matched += 1;
        if matched >= max_results {
            let _ = child.kill();
            break;
        }
    }

    let _ = child.wait();

    if cancelled.load(Ordering::Relaxed) {
        return;
    }
    let _ = sender.send(GlobalSearchEvent::Done {
        pane_id,
        query: query.to_string(),
    });
}

/// 建立檔名搜尋使用的 `fd` 命令，並讓輸出可安全地逐筆解析。
///
/// 參數：
/// - `root: &Path`，搜尋起始目錄。
/// - `show_hidden: bool`，是否包含隱藏檔案與目錄。
/// - `query: &str`，要比對的檔案名稱文字。
///
/// 回傳：`std::io::Result<Command>`，成功時可直接啟動的 `fd` 命令。
fn build_fd_search_command(
    root: &Path,
    show_hidden: bool,
    query: &str,
) -> Result<Command, std::io::Error> {
    let program = fd_command().map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut command = Command::new(program);
    command
        .arg("--color")
        .arg("never")
        // NUL 分隔可正確處理包含空格、換行或其他特殊字元的檔名。
        .arg("--print0")
        .arg("--ignore-case")
        .arg("--fixed-strings");

    if show_hidden {
        command.arg("--hidden");
    }

    // 從 root 內輸出相對路徑，避免 macOS 把 `/var` 正規化為 `/private/var`，
    // 也能在 Windows 保留使用者原本輸入的磁碟與路徑形式。
    command.current_dir(root).arg("--").arg(query).arg(".");
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    Ok(command)
}

#[cfg(unix)]
/// 將 `fd` 的原始路徑 bytes 轉為 Unix/macOS 可完整保留的 `PathBuf`。
fn path_buf_from_fd_output(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(bytes))
}

#[cfg(windows)]
/// 將 Windows 版 `fd` 的 UTF-8 輸出轉為 `PathBuf`。
fn path_buf_from_fd_output(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(not(any(unix, windows)))]
/// 在其他未正式支援的平台上，以 lossy UTF-8 方式轉換 `fd` 輸出。
fn path_buf_from_fd_output(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

/// 使用外部 `rg` 搜尋檔案內容，找到至少一個文字命中的檔案就立即回傳。
///
/// 規則：
/// - 只搜尋檔案，不回傳資料夾。
/// - 以不分大小寫方式比對內容。
/// - 不提供內建搜尋 fallback；缺少或無法啟動 `rg` 時，通知主程式顯示依賴狀態。
#[allow(clippy::too_many_arguments)]
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

    if !stream_content_search_entries_with_rg(
        pane_id,
        root,
        show_hidden,
        query,
        limit,
        chunk_size,
        cancelled.clone(),
        &sender,
    ) {
        let _ = sender.send(GlobalSearchEvent::MissingTool {
            pane_id,
            query: query.to_string(),
            tool: String::from("rg"),
        });
    }
}

/// 啟動 ripgrep 並串流解析內容搜尋結果。
///
/// 回傳：`bool`，成功完成或被使用者取消時為 `true`；無法使用 `rg` 時為 `false`。
#[allow(clippy::too_many_arguments)]
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
    let mut command = match build_rg_content_search_command(root, show_hidden, query, limit) {
        Ok(command) => command,
        Err(_) => return false,
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
        matched += 1;

        // 第一筆命中立刻送出，讓 TUI 不必等待批次計時或第二筆結果。
        if matched == 1 || should_flush_batch(&batch, content_chunk_size, &mut last_flush_at) {
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

    false
}

/// 建立給 ripgrep 使用的內容搜尋命令。
fn build_rg_content_search_command(
    root: &Path,
    show_hidden: bool,
    query: &str,
    _limit: usize,
) -> Result<Command, std::io::Error> {
    let program = rg_command().map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut command = Command::new(program);
    command
        .arg("--json")
        // stdout 會被 pipe 接收；沒有這個選項時，rg 可能累積輸出後才 flush，
        // 讓前端明明已經找到結果，卻要等很久才看得到第一筆。
        .arg("--line-buffered")
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
        command
            .arg("--hidden")
            // 隱藏檔仍然搜尋，但不要把 Git 內部物件與索引當成使用者內容。
            .arg("--glob")
            .arg("!.git");
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

/// 把目前批次中的搜尋結果依收到順序送回主執行緒。
///
/// 不在這裡排序，才能確保 `S` 的既有列表不會因後續結果而重新排列；
/// 主執行緒會把每個批次穩定追加到列表下方。
///
/// 參數：
/// - `pane_id: usize`，接收這批結果的 panel 編號。
/// - `query: &str`，用來辨識目前搜尋工作的查詢文字。
/// - `batch: &mut Vec<GlobalSearchEntry>`，依外部工具回傳順序累積的結果；送出後會清空。
/// - `cancelled: &AtomicBool`，背景搜尋的取消旗標。
/// - `sender: &Sender<GlobalSearchEvent>`，將批次傳回主執行緒的 channel。
///
/// 回傳：`()`，此函數只負責送出事件，不產生額外回傳值。
fn flush_search_batch(
    pane_id: usize,
    query: &str,
    batch: &mut Vec<GlobalSearchEntry>,
    cancelled: &AtomicBool,
    sender: &Sender<GlobalSearchEvent>,
) {
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
#[path = "tests/search_test.rs"]
mod tests;
