//! 版本控制（Git 與 SVN）非同步偵測與狀態解析模組。
//!
//! 提供版本控制工作區根目錄偵測、分支/版本號識別、檔案變更狀態（M/A/D/?/C/R）
//! 解析，以及背景非阻塞 Worker 與快取機制。

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::theme::Theme;

/// 版本控制系統種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VcsType {
    Git,
    Svn,
}

/// 單一檔案或資料夾在版本控制中的狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VcsFileStatus {
    /// 檔案已修改（Modified）
    Modified,
    /// 檔案已暫存或排程新增（Added）
    Added,
    /// 檔案已刪除或遺失（Deleted / Missing）
    Deleted,
    /// 未納入版控 / 未追蹤（Untracked / Unversioned）
    Untracked,
    /// 發生衝突（Conflict）
    Conflict,
    /// 檔案重新命名（Renamed）
    Renamed,
    /// 檔案型別變更或替換（Type Changed / Replaced）
    TypeChanged,
    /// 檔案被忽略（Ignored）
    Ignored,
}

impl VcsFileStatus {
    /// 回傳適合列表顯示的單一字元標籤。
    pub(crate) fn badge_char(&self) -> char {
        match self {
            VcsFileStatus::Modified => 'M',
            VcsFileStatus::Added => 'A',
            VcsFileStatus::Deleted => 'D',
            VcsFileStatus::Untracked => '?',
            VcsFileStatus::Conflict => 'C',
            VcsFileStatus::Renamed => 'R',
            VcsFileStatus::TypeChanged => '~',
            VcsFileStatus::Ignored => '!',
        }
    }

    /// 依據當前主題與狀態種類回傳對應的高對比前景色。
    pub(crate) fn color(&self, theme: &Theme) -> Color {
        match self {
            VcsFileStatus::Modified => Color::Yellow,
            VcsFileStatus::Added => Color::Green,
            VcsFileStatus::Deleted => Color::Red,
            VcsFileStatus::Untracked => Color::Cyan,
            VcsFileStatus::Conflict => Color::LightRed,
            VcsFileStatus::Renamed => Color::Blue,
            VcsFileStatus::TypeChanged => Color::Magenta,
            VcsFileStatus::Ignored => theme.muted,
        }
    }

    /// 狀態優先權順序（供目錄向上匯總時判定）。
    pub(crate) fn priority(&self) -> u8 {
        match self {
            VcsFileStatus::Conflict => 7,
            VcsFileStatus::Modified => 6,
            VcsFileStatus::Added => 5,
            VcsFileStatus::Deleted => 4,
            VcsFileStatus::Renamed => 3,
            VcsFileStatus::Untracked => 2,
            VcsFileStatus::TypeChanged => 1,
            VcsFileStatus::Ignored => 0,
        }
    }
}

/// 版本控制儲存庫的資訊與狀態對照表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VcsRepoInfo {
    pub(crate) vcs_type: VcsType,
    pub(crate) repo_root: PathBuf,
    pub(crate) branch_or_rev: String,
    pub(crate) statuses: HashMap<PathBuf, VcsFileStatus>,
}

impl VcsRepoInfo {
    /// 格式化標題列徽章文字，如 `[git:main]` 或 `[svn:trunk:r42]`。
    pub(crate) fn format_header_label(&self) -> String {
        match self.vcs_type {
            VcsType::Git => format!("git:{}", self.branch_or_rev),
            VcsType::Svn => {
                if self.branch_or_rev == "svn" {
                    "svn".to_string()
                } else {
                    format!("svn:{}", self.branch_or_rev)
                }
            }
        }
    }

    /// 查詢指定路徑的版控狀態。
    /// 查詢指定路徑的版控狀態（自動進行跨平台路徑分隔符標準化）。
    pub(crate) fn status_for_path(&self, path: &Path) -> Option<VcsFileStatus> {
        let normalized = normalize_path(path);
        self.statuses.get(&normalized).copied()
    }
}

/// 將路徑分隔符統一標準化為系統原生格式（Windows 統一轉為 `\`，消除與 Git 輸出的不一致）。
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if s.contains('/') {
            PathBuf::from(s.replace('/', "\\"))
        } else {
            path.to_path_buf()
        }
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// 嚴格驗證指定目錄下是否存在有效的 Git 儲存庫標記（避免空目錄或殘留資料夾誤判）。
pub(crate) fn is_valid_git_dir(dir: &Path) -> bool {
    let git_marker = dir.join(".git");
    if git_marker.is_dir() {
        git_marker.join("HEAD").exists() || git_marker.join("config").exists()
    } else if git_marker.is_file() {
        if let Ok(content) = fs::read_to_string(&git_marker) {
            content.trim().starts_with("gitdir:")
        } else {
            false
        }
    } else {
        false
    }
}

/// 向上搜尋目錄階層以找出所有可能的 VCS 候選根目錄（由深至淺，支援回退）。
///
/// 優先權規範：
/// 若專案同時混用 SVN 與 Git（例如 SVN 作為主要團隊版本庫，Git 作為本地暫存），
/// 必須優先採用 SVN，避免被本地過期之 Git HEAD 覆蓋。
pub(crate) fn find_vcs_candidates(start_dir: &Path) -> Vec<(VcsType, PathBuf)> {
    let mut svn_candidates = Vec::new();
    let mut git_candidates = Vec::new();
    let mut current = if start_dir.is_file() {
        start_dir.parent()
    } else {
        Some(start_dir)
    };

    while let Some(dir) = current {
        let svn_marker = dir.join(".svn");
        if svn_marker.is_dir() {
            // SVN 1.7+ 通常在工作區根目錄有一個 .svn 目錄。若父層也有，以最高層為準。
            let mut top_svn = dir.to_path_buf();
            let mut check_parent = dir.parent();
            while let Some(p) = check_parent {
                if p.join(".svn").is_dir() {
                    top_svn = p.to_path_buf();
                    check_parent = p.parent();
                } else {
                    break;
                }
            }
            if !svn_candidates
                .iter()
                .any(|(t, p)| *t == VcsType::Svn && p == &top_svn)
            {
                svn_candidates.push((VcsType::Svn, top_svn));
            }
        }

        if is_valid_git_dir(dir) {
            git_candidates.push((VcsType::Git, dir.to_path_buf()));
        }

        current = dir.parent();
    }

    // 依據「SVN 優先於 Git」原則：SVN 候選放置於最前列，Git 作為次要備用
    let mut candidates = svn_candidates;
    candidates.extend(git_candidates);
    candidates
}

/// 向上搜尋目錄階層以找出最接近的有效 VCS 儲存庫根目錄。
#[allow(dead_code)]
pub(crate) fn find_vcs_root(start_dir: &Path) -> Option<(VcsType, PathBuf)> {
    find_vcs_candidates(start_dir).into_iter().next()
}

/// 執行指令並帶有嚴格的超時保護（避免外部指令在大型專案或網路硬碟卡死）。
pub(crate) fn run_command_with_timeout(mut cmd: Command, timeout: Duration) -> Option<String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    let mut output = String::new();
                    if let Some(mut stdout) = child.stdout.take() {
                        let _ = stdout.read_to_string(&mut output);
                    }
                    return Some(output);
                } else {
                    return None;
                }
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    return None;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

/// 快速從 Git 的 `.git/HEAD` 讀取當前分支名稱（極速路徑，0 行程啟動開銷）。
pub(crate) fn read_git_branch_from_head(repo_root: &Path) -> Option<String> {
    let git_path = repo_root.join(".git");
    let head_path = if git_path.is_file() {
        // 處理 submodule 或 git worktree：.git 檔案內容為 "gitdir: <path>"
        let content = fs::read_to_string(&git_path).ok()?;
        let trimmed = content.trim();
        let target = trimmed.strip_prefix("gitdir:")?.trim();
        let target_path = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            repo_root.join(target)
        };
        target_path.join("HEAD")
    } else {
        git_path.join("HEAD")
    };

    let content = fs::read_to_string(head_path).ok()?;
    let line = content.lines().next()?.trim();
    if let Some(branch) = line.strip_prefix("ref: refs/heads/") {
        Some(branch.to_string())
    } else if line.len() >= 7 {
        // Detached HEAD：取前 7 位 commit SHA
        Some(line[..7].to_string())
    } else {
        None
    }
}

/// 解析 `git status --porcelain=v1 -uall` 的輸出文字。
pub(crate) fn parse_git_status_output(
    output: &str,
    repo_root: &Path,
) -> HashMap<PathBuf, VcsFileStatus> {
    let mut statuses = HashMap::new();

    for line in output.lines() {
        if line.len() < 4 {
            continue;
        }
        let index_status = line.as_bytes()[0] as char;
        let worktree_status = line.as_bytes()[1] as char;
        let raw_path = &line[3..].trim();

        let status = if index_status == 'U'
            || worktree_status == 'U'
            || (index_status == 'A' && worktree_status == 'A')
            || (index_status == 'D' && worktree_status == 'D')
        {
            VcsFileStatus::Conflict
        } else if index_status == '?' && worktree_status == '?' {
            VcsFileStatus::Untracked
        } else if index_status == '!' || worktree_status == '!' {
            VcsFileStatus::Ignored
        } else if index_status == 'M' || worktree_status == 'M' {
            VcsFileStatus::Modified
        } else if index_status == 'A' || worktree_status == 'A' {
            VcsFileStatus::Added
        } else if index_status == 'D' || worktree_status == 'D' {
            VcsFileStatus::Deleted
        } else if index_status == 'R' || worktree_status == 'R' {
            VcsFileStatus::Renamed
        } else if index_status == 'T' || worktree_status == 'T' {
            VcsFileStatus::TypeChanged
        } else {
            VcsFileStatus::Modified
        };

        // 處理 rename 格式：`R  old -> new`
        let path_str = if let Some((_, new_path)) = raw_path.split_once(" -> ") {
            new_path.trim()
        } else {
            raw_path
        };

        // 去除引號包裹（如帶有空白或特殊字元的路徑）並進行系統原生路徑標準化
        let clean_path = path_str.trim_matches('"');
        let rel_path = normalize_path(Path::new(clean_path));
        let full_path = normalize_path(&repo_root.join(rel_path));
        statuses.insert(full_path.clone(), status);

        // 將狀態向上匯總給父資料夾，讓資料夾在列表也能呈現變更標記
        rollup_status_to_ancestors(&mut statuses, &full_path, repo_root, status);
    }

    statuses
}

/// 解析 `svn status` 的本機輸出文字。
pub(crate) fn parse_svn_status_output(
    output: &str,
    repo_root: &Path,
) -> HashMap<PathBuf, VcsFileStatus> {
    let mut statuses = HashMap::new();

    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let first_char = line.chars().next().unwrap_or(' ');
        let status = match first_char {
            'M' => VcsFileStatus::Modified,
            'A' => VcsFileStatus::Added,
            'D' | '!' => VcsFileStatus::Deleted,
            '?' => VcsFileStatus::Untracked,
            'C' => VcsFileStatus::Conflict,
            'R' => VcsFileStatus::Renamed,
            '~' => VcsFileStatus::TypeChanged,
            'I' => VcsFileStatus::Ignored,
            _ => continue,
        };

        // SVN status 前 7-8 欄為狀態碼，之後為空格與路徑
        let path_part = if line.len() > 8 {
            line[8..].trim()
        } else if let Some(idx) = line.find(' ') {
            line[idx..].trim()
        } else {
            continue;
        };

        if path_part.is_empty() {
            continue;
        }

        let base_dir = normalize_path(repo_root);
        let rel_path = normalize_path(Path::new(path_part));
        let full_path = normalize_path(&base_dir.join(rel_path));
        statuses.insert(full_path.clone(), status);
        rollup_status_to_ancestors(&mut statuses, &full_path, &base_dir, status);
    }

    statuses
}

/// 解析 `svn info` 輸出以取得相對分支與修訂版本號。
pub(crate) fn parse_svn_info_output(output: &str) -> String {
    let mut branch = None;
    let mut revision = None;

    for line in output.lines() {
        if let Some(val) = line.strip_prefix("Relative URL:") {
            let trimmed = val.trim().trim_start_matches('^').trim_start_matches('/');
            let clean_branch = if let Some(b) = trimmed.strip_prefix("branches/") {
                b
            } else if let Some(t) = trimmed.strip_prefix("tags/") {
                t
            } else {
                trimmed
            };
            if !clean_branch.is_empty() {
                branch = Some(clean_branch.to_string());
            }
        } else if let Some(val) = line.strip_prefix("Revision:") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                revision = Some(format!("r{trimmed}"));
            }
        }
    }

    match (branch, revision) {
        (Some(b), Some(r)) => format!("{b}:{r}"),
        (Some(b), None) => b,
        (None, Some(r)) => r,
        (None, None) => "svn".to_string(),
    }
}

/// 將檔案的變更狀態依優先權向上匯總至其父層目錄。
fn rollup_status_to_ancestors(
    statuses: &mut HashMap<PathBuf, VcsFileStatus>,
    file_path: &Path,
    repo_root: &Path,
    status: VcsFileStatus,
) {
    let mut cur = file_path.parent();
    while let Some(parent) = cur {
        if parent == repo_root || !parent.starts_with(repo_root) {
            break;
        }
        let parent_path = parent.to_path_buf();
        statuses
            .entry(parent_path)
            .and_modify(|existing| {
                if status.priority() > existing.priority() {
                    *existing = status;
                }
            })
            .or_insert(status);
        cur = parent.parent();
    }
}

/// 查詢 Git 儲存庫完整資訊。
pub(crate) fn query_git_repo_info(repo_root: &Path, _target_dir: &Path) -> Option<VcsRepoInfo> {
    let branch = read_git_branch_from_head(repo_root).or_else(|| {
        let mut cmd = Command::new("git");
        cmd.args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_root);
        run_command_with_timeout(cmd, Duration::from_secs(2)).map(|s| s.trim().to_string())
    })?;

    let mut status_cmd = Command::new("git");
    status_cmd
        .args(["status", "--porcelain=v1", "-uall"])
        .current_dir(repo_root);

    let statuses = run_command_with_timeout(status_cmd, Duration::from_secs(2))
        .map(|out| parse_git_status_output(&out, repo_root))
        .unwrap_or_default();

    Some(VcsRepoInfo {
        vcs_type: VcsType::Git,
        repo_root: repo_root.to_path_buf(),
        branch_or_rev: branch,
        statuses,
    })
}

/// 查詢 SVN 工作副本完整資訊（針對當前目錄執行遞迴 status 查詢，使所有子目錄變更向上 rollup 至父資料夾呈現 M 標記）。
pub(crate) fn query_svn_repo_info(repo_root: &Path, target_dir: &Path) -> Option<VcsRepoInfo> {
    // 優先在目前瀏覽目錄執行 svn info，若失敗再回退至 repo_root
    let mut info_cmd = Command::new("svn");
    info_cmd
        .args(["info", "--non-interactive"])
        .current_dir(target_dir);

    let branch_or_rev = run_command_with_timeout(info_cmd, Duration::from_secs(3))
        .or_else(|| {
            let mut root_cmd = Command::new("svn");
            root_cmd
                .args(["info", "--non-interactive"])
                .current_dir(repo_root);
            run_command_with_timeout(root_cmd, Duration::from_secs(3))
        })
        .map(|out| parse_svn_info_output(&out))
        .unwrap_or_else(|| "svn".to_string());

    // 針對目前目錄執行遞迴 status 查詢，確保所有子目錄變更向上 rollup 至父資料夾呈現 M 標記
    let mut status_cmd = Command::new("svn");
    status_cmd
        .args(["status", "--non-interactive", "--ignore-externals"])
        .current_dir(target_dir);

    let statuses = run_command_with_timeout(status_cmd, Duration::from_secs(3))
        .map(|out| parse_svn_status_output(&out, target_dir))
        .unwrap_or_default();

    Some(VcsRepoInfo {
        vcs_type: VcsType::Svn,
        repo_root: repo_root.to_path_buf(),
        branch_or_rev,
        statuses,
    })
}

/// 查詢指定檔案在 Git 或 SVN 中的變更差異（Unified Diff 輸出）。
pub(crate) fn query_vcs_file_diff(file_path: &Path) -> Option<String> {
    let parent = file_path.parent()?;
    let candidates = find_vcs_candidates(parent);

    for (vcs_type, repo_root) in candidates {
        match vcs_type {
            VcsType::Svn => {
                let mut cmd = Command::new("svn");
                cmd.args(["diff", "--non-interactive"])
                    .arg(file_path)
                    .current_dir(parent);
                if let Some(out) = run_command_with_timeout(cmd, Duration::from_millis(1500)) {
                    let trimmed = out.trim();
                    if !trimmed.is_empty() {
                        return Some(out);
                    }
                }
            }
            VcsType::Git => {
                // 1. 優先比對 HEAD 的變更（包含已暫存與工作區未暫存修改）
                let mut cmd = Command::new("git");
                cmd.args(["diff", "HEAD", "--"])
                    .arg(file_path)
                    .current_dir(&repo_root);
                if let Some(out) = run_command_with_timeout(cmd, Duration::from_millis(800)) {
                    let trimmed = out.trim();
                    if !trimmed.is_empty() {
                        return Some(out);
                    }
                }

                // 2. 若 HEAD 比對為空（例如初始未 commit 狀態），嘗試普通工作區 git diff
                let mut cmd_fallback = Command::new("git");
                cmd_fallback
                    .args(["diff", "--"])
                    .arg(file_path)
                    .current_dir(&repo_root);
                if let Some(out) =
                    run_command_with_timeout(cmd_fallback, Duration::from_millis(800))
                {
                    let trimmed = out.trim();
                    if !trimmed.is_empty() {
                        return Some(out);
                    }
                }
            }
        }
    }

    None
}

/// 將 Diff 原始文字格式化為帶有色彩高亮的 Ratatui 行清單。
pub(crate) fn format_diff_lines(diff_text: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for raw_line in diff_text.lines() {
        if raw_line.starts_with("+++") || raw_line.starts_with("---") {
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if raw_line.starts_with("@@") {
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if raw_line.starts_with('+') {
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(Color::Green),
            )));
        } else if raw_line.starts_with('-') {
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(Color::Red),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                raw_line.to_string(),
                Style::default().fg(theme.muted),
            )));
        }
    }
    lines
}

/// 向 VCS Worker 發送的指令。
pub(crate) enum VcsWorkerCommand {
    /// 查詢指定 Pane 所瀏覽的目錄。
    Query { pane_id: usize, directory: PathBuf },
    /// 將指定目錄（或全部）的快取標記失效。
    Invalidate { directory: Option<PathBuf> },
}

/// VCS Worker 查詢完成後回傳的結果。
pub(crate) struct VcsQueryResponse {
    pub(crate) pane_id: usize,
    pub(crate) directory: PathBuf,
    pub(crate) info: Option<Arc<VcsRepoInfo>>,
}

/// VCS 背景管理器與 Worker 通訊端點。
pub(crate) struct VcsManager {
    command_tx: Sender<VcsWorkerCommand>,
    response_rx: Receiver<VcsQueryResponse>,
}

impl std::fmt::Debug for VcsManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VcsManager").finish_non_exhaustive()
    }
}

impl VcsManager {
    /// 啟動 VCS 背景查詢 Worker 執行緒。
    pub(crate) fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<VcsWorkerCommand>();
        let (resp_tx, resp_rx) = mpsc::channel::<VcsQueryResponse>();

        thread::spawn(move || {
            let mut cache: HashMap<PathBuf, (Option<Arc<VcsRepoInfo>>, Instant)> = HashMap::new();
            let cache_ttl = Duration::from_secs(5);

            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    VcsWorkerCommand::Invalidate { directory } => match directory {
                        None => cache.clear(),
                        Some(dir) => {
                            cache.retain(|cached_dir, _| {
                                !cached_dir.starts_with(&dir) && !dir.starts_with(cached_dir)
                            });
                        }
                    },
                    VcsWorkerCommand::Query { pane_id, directory } => {
                        let now = Instant::now();

                        // 檢查是否有未過期快取（以 directory 為 key）
                        if let Some((cached_info, timestamp)) = cache.get(&directory)
                            && now.duration_since(*timestamp) < cache_ttl
                        {
                            let _ = resp_tx.send(VcsQueryResponse {
                                pane_id,
                                directory,
                                info: cached_info.clone(),
                            });
                            continue;
                        }

                        // 搜尋所有候選 VCS 根目錄，嘗試查詢（內層損毀自動回退至外層）
                        let candidates = find_vcs_candidates(&directory);
                        let mut resolved_info = None;

                        for (vcs_type, repo_root) in candidates {
                            let info = match vcs_type {
                                VcsType::Git => query_git_repo_info(&repo_root, &directory),
                                VcsType::Svn => query_svn_repo_info(&repo_root, &directory),
                            };
                            if let Some(info) = info {
                                resolved_info = Some(Arc::new(info));
                                break;
                            }
                        }

                        cache.insert(directory.clone(), (resolved_info.clone(), now));

                        let _ = resp_tx.send(VcsQueryResponse {
                            pane_id,
                            directory,
                            info: resolved_info,
                        });
                    }
                }
            }
        });

        Self {
            command_tx: cmd_tx,
            response_rx: resp_rx,
        }
    }

    /// 請求背景查詢特定 pane 之目錄 VCS 狀態。
    pub(crate) fn request_query(&self, pane_id: usize, directory: PathBuf) {
        let _ = self
            .command_tx
            .send(VcsWorkerCommand::Query { pane_id, directory });
    }

    /// 標記快取失效（例如檔案系統變更或使用者重新整理）。
    pub(crate) fn invalidate(&self, directory: Option<PathBuf>) {
        let _ = self
            .command_tx
            .send(VcsWorkerCommand::Invalidate { directory });
    }

    /// 非阻塞接收背景已完成的 VCS 查詢結果。
    pub(crate) fn try_recv_response(&self) -> Option<VcsQueryResponse> {
        self.response_rx.try_recv().ok()
    }
}

#[cfg(test)]
#[path = "tests/vcs_test.rs"]
mod tests;
