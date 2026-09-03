//! 全螢幕 N 路目錄與檔案差異比對系統 (N-Way Directory & File Diff System)。
//!
//! 支援 2 個、3 個、4 個或更多 Panel 進行非阻塞背景遞迴目錄結構與檔案內容比對。
//! 採用智慧過濾（排除 `.git` 等內部暫存目錄，遵循 `.gitignore`）、零 I/O 大小預篩選，
//! 與取樣特徵碼比對，並支援一鍵調用 `delta`、`difftastic`、`nvim -d` 或 `vimdiff` 查看詳細檔案差異。

use std::{
    collections::BTreeSet,
    fs::{self, File},
    hash::{DefaultHasher, Hasher},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
};

use ignore::WalkBuilder;

use super::open::{LaunchMode, LaunchSpec};

/// 描述單一項目在某個 Panel 中的存在與內容特徵。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffEntryState {
    Present { size: u64, hash: u64, is_dir: bool },
    Missing,
}

/// 描述多個 Panel 之間對同一相對路徑的比對結論。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStatus {
    /// 所有包含此檔案的端點內容完全一致，且所有 Panel 都有此檔案。
    Identical,
    /// 存在於多個端點，但內容或大小有差異。
    Modified,
    /// 僅存在於單一端點（獨有項目）。
    Exclusive { panel_index: usize },
    /// 存在於部分端點（子集），且在存在的端點間內容一致。
    Subset,
}

/// 差異矩陣表格中的單一列資料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffMatrixRow {
    pub relative_path: PathBuf,
    pub is_dir: bool,
    pub panel_states: Vec<DiffEntryState>,
    pub status: DiffStatus,
    pub display_size: u64,
}

/// 矩陣清單的過濾篩選模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffFilterMode {
    #[default]
    All,
    DiffOnly,
    ExclusiveOnly,
    IdenticalOnly,
}

impl DiffFilterMode {
    pub fn next(self) -> Self {
        match self {
            DiffFilterMode::All => DiffFilterMode::DiffOnly,
            DiffFilterMode::DiffOnly => DiffFilterMode::ExclusiveOnly,
            DiffFilterMode::ExclusiveOnly => DiffFilterMode::IdenticalOnly,
            DiffFilterMode::IdenticalOnly => DiffFilterMode::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiffFilterMode::All => "All (全部)",
            DiffFilterMode::DiffOnly => "Diff Only (僅差異/缺失)",
            DiffFilterMode::ExclusiveOnly => "Exclusive Only (僅單端獨有)",
            DiffFilterMode::IdenticalOnly => "Identical Only (僅完全一致)",
        }
    }
}

/// 背景比對工作事件。
#[derive(Debug)]
pub enum DiffJobEvent {
    Discovered(usize),
    Done(Vec<DiffMatrixRow>),
    #[allow(dead_code)]
    Error(String),
}

/// 全螢幕差異比對工作區狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffMatrixState {
    pub panel_ids: Vec<usize>,
    pub panel_roots: Vec<PathBuf>,
    pub panel_labels: Vec<String>,
    pub rows: Vec<DiffMatrixRow>,
    pub filtered_indices: Vec<usize>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub filter_mode: DiffFilterMode,
    pub search_query: String,
    pub search_active: bool,
    pub loading: bool,
    pub discovered_count: usize,
    pub git_ignore: bool,
    pub include_hidden: bool,
}

impl DiffMatrixState {
    /// 建立新的差異比對狀態（初始化為 loading 狀態）。
    pub fn new_loading(
        panel_ids: Vec<usize>,
        panel_roots: Vec<PathBuf>,
        panel_labels: Vec<String>,
    ) -> Self {
        Self {
            panel_ids,
            panel_roots,
            panel_labels,
            rows: Vec::new(),
            filtered_indices: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            filter_mode: DiffFilterMode::All,
            search_query: String::new(),
            search_active: false,
            loading: true,
            discovered_count: 0,
            git_ignore: true,
            include_hidden: true,
        }
    }

    /// 建立同步比對狀態（主要供單元測試使用）。
    #[allow(dead_code)]
    pub fn new_sync(
        panel_ids: Vec<usize>,
        panel_roots: Vec<PathBuf>,
        panel_labels: Vec<String>,
    ) -> io::Result<Self> {
        let mut rows = compute_diff_matrix(&panel_roots)?;
        sort_diff_rows(&mut rows);
        let mut state = Self {
            panel_ids,
            panel_roots,
            panel_labels,
            rows,
            filtered_indices: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            filter_mode: DiffFilterMode::All,
            search_query: String::new(),
            search_active: false,
            loading: false,
            discovered_count: 0,
            git_ignore: true,
            include_hidden: true,
        };
        state.refresh_filtered_indices();
        Ok(state)
    }

    /// 套用完成的比對結果。
    pub fn set_completed_rows(&mut self, mut rows: Vec<DiffMatrixRow>) {
        sort_diff_rows(&mut rows);
        self.rows = rows;
        self.loading = false;
        self.refresh_filtered_indices();
    }

    /// 依據目前的 filter_mode 與 search_query 重新計算過濾後的列索引。
    pub fn refresh_filtered_indices(&mut self) {
        let query = self.search_query.trim().to_lowercase();
        self.filtered_indices = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                let matches_filter = match self.filter_mode {
                    DiffFilterMode::All => true,
                    DiffFilterMode::DiffOnly => row.status != DiffStatus::Identical,
                    DiffFilterMode::ExclusiveOnly => {
                        matches!(row.status, DiffStatus::Exclusive { .. })
                    }
                    DiffFilterMode::IdenticalOnly => row.status == DiffStatus::Identical,
                };
                if !matches_filter {
                    return false;
                }

                if query.is_empty() {
                    return true;
                }

                let path_str = row.relative_path.to_string_lossy().to_lowercase();
                path_str.contains(&query)
            })
            .map(|(idx, _)| idx)
            .collect();

        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
            self.scroll_offset = 0;
        } else if self.selected_index >= self.filtered_indices.len() {
            self.selected_index = self.filtered_indices.len().saturating_sub(1);
        }
    }

    /// 向上移動游標。
    pub fn move_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// 向下移動游標。
    pub fn move_down(&mut self) {
        if !self.filtered_indices.is_empty()
            && self.selected_index + 1 < self.filtered_indices.len()
        {
            self.selected_index += 1;
        }
    }

    /// 移動到開頭。
    pub fn move_to_top(&mut self) {
        self.selected_index = 0;
    }

    /// 移動到末尾。
    pub fn move_to_bottom(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.selected_index = self.filtered_indices.len() - 1;
        }
    }

    /// 切換過濾模式。
    pub fn cycle_filter_mode(&mut self) {
        self.filter_mode = self.filter_mode.next();
        self.refresh_filtered_indices();
    }

    /// 取得目前選取的項目列。
    pub fn selected_row(&self) -> Option<&DiffMatrixRow> {
        let row_idx = *self.filtered_indices.get(self.selected_index)?;
        self.rows.get(row_idx)
    }

    /// 計算所有「不同」項目的數量（非 Identical 的項目，包含內容差異、單端獨有/新增、部分端點）。
    pub fn different_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.status != DiffStatus::Identical)
            .count()
    }

    /// 計算完全相同（Identical）項目的數量。
    pub fn identical_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.status == DiffStatus::Identical)
            .count()
    }
}

/// 排序比對項目：所有「不同」（status != Identical）的檔案與目錄排在最前面，
/// 完全相同（status == Identical）排在後面；同一組內按相對路徑字母排序。
pub fn sort_diff_rows(rows: &mut [DiffMatrixRow]) {
    rows.sort_by(|a, b| {
        let a_diff = a.status != DiffStatus::Identical;
        let b_diff = b.status != DiffStatus::Identical;
        match (a_diff, b_diff) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.relative_path.cmp(&b.relative_path),
        }
    });
}

/// 快速計算檔案的取樣特徵雜湊碼。
fn compute_file_sample_hash(path: &Path, file_size: u64) -> io::Result<u64> {
    let mut file = File::open(path)?;
    let mut hasher = DefaultHasher::new();
    hasher.write_u64(file_size);

    const SAMPLE_CHUNK: usize = 16 * 1024;
    if file_size <= (SAMPLE_CHUNK * 3) as u64 {
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;
        hasher.write(&buffer);
    } else {
        // 讀取開頭 16KB
        let mut head = vec![0u8; SAMPLE_CHUNK];
        let n = file.read(&mut head)?;
        hasher.write(&head[..n]);

        // 讀取中間 16KB
        let mid_offset = (file_size / 2).saturating_sub((SAMPLE_CHUNK / 2) as u64);
        use std::io::Seek;
        if file.seek(io::SeekFrom::Start(mid_offset)).is_ok() {
            let mut mid = vec![0u8; SAMPLE_CHUNK];
            let n = file.read(&mut mid)?;
            hasher.write(&mid[..n]);
        }

        // 讀取結尾 16KB
        let tail_offset = file_size.saturating_sub(SAMPLE_CHUNK as u64);
        if file.seek(io::SeekFrom::Start(tail_offset)).is_ok() {
            let mut tail = vec![0u8; SAMPLE_CHUNK];
            let n = file.read(&mut tail)?;
            hasher.write(&tail[..n]);
        }
    }

    Ok(hasher.finish())
}

/// 判斷路徑是否應該被過濾忽略（如 `.git` 內部物件目錄）。
fn should_ignore_component(comp: &str) -> bool {
    comp == ".git"
}

/// 啟動非阻塞背景比對執行緒。
pub fn spawn_background_diff(
    roots: Vec<PathBuf>,
    git_ignore: bool,
    include_hidden: bool,
    cancelled: Arc<AtomicBool>,
    sender: Sender<DiffJobEvent>,
) {
    thread::spawn(move || {
        let mut relative_paths = BTreeSet::new();

        // 1. 遍歷所有 root，收集相對路徑集合
        for root in &roots {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
            if !root.exists() {
                continue;
            }

            let walker = WalkBuilder::new(root)
                .hidden(!include_hidden)
                .parents(true)
                .git_ignore(git_ignore)
                .git_global(git_ignore)
                .git_exclude(git_ignore)
                .build();

            for result in walker {
                if cancelled.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(entry) = result else { continue };
                let path = entry.path();
                if path == root {
                    continue;
                }

                if let Ok(rel) = path.strip_prefix(root) {
                    if rel.as_os_str().is_empty() {
                        continue;
                    }
                    // 忽略 .git 內部物件
                    if rel
                        .components()
                        .any(|c| should_ignore_component(&c.as_os_str().to_string_lossy()))
                    {
                        continue;
                    }

                    relative_paths.insert(rel.to_path_buf());
                    if relative_paths.len() % 500 == 0 {
                        let _ = sender.send(DiffJobEvent::Discovered(relative_paths.len()));
                    }
                }
            }
        }

        let _ = sender.send(DiffJobEvent::Discovered(relative_paths.len()));

        // 2. 針對收集到的相對路徑進行矩陣比對
        let mut rows = Vec::with_capacity(relative_paths.len());

        for rel_path in relative_paths {
            if cancelled.load(Ordering::Relaxed) {
                return;
            }

            let mut panel_states = Vec::with_capacity(roots.len());
            let mut present_count = 0usize;
            let mut first_present_idx = None;
            let mut is_dir = false;
            let mut max_size = 0u64;

            // 第一步：收集檔案元資料與大小（零 I/O）
            for (idx, root) in roots.iter().enumerate() {
                let full_path = root.join(&rel_path);
                if let Ok(metadata) = fs::symlink_metadata(&full_path) {
                    let entry_is_dir = metadata.is_dir();
                    is_dir = is_dir || entry_is_dir;
                    let size = if entry_is_dir { 0 } else { metadata.len() };
                    max_size = max_size.max(size);

                    panel_states.push(DiffEntryState::Present {
                        size,
                        hash: 0,
                        is_dir: entry_is_dir,
                    });
                    present_count += 1;
                    if first_present_idx.is_none() {
                        first_present_idx = Some(idx);
                    }
                } else {
                    panel_states.push(DiffEntryState::Missing);
                }
            }

            // 第二步：快速判定狀態，僅在大小相同時取樣計算 Hash
            let status = if present_count == 0 {
                DiffStatus::Identical
            } else if present_count == 1 {
                DiffStatus::Exclusive {
                    panel_index: first_present_idx.unwrap_or(0),
                }
            } else {
                // 檢查所有 Present 項目的大小是否一致
                let mut size_match = true;
                let mut first_size = None;
                for state in &panel_states {
                    if let DiffEntryState::Present { size, .. } = state {
                        if let Some(s) = first_size {
                            if *size != s {
                                size_match = false;
                                break;
                            }
                        } else {
                            first_size = Some(*size);
                        }
                    }
                }

                if !size_match {
                    // 大小不同必定不同，不需讀取磁碟
                    DiffStatus::Modified
                } else if is_dir {
                    // 目錄只要兩端都存在且型態相同即一致
                    if present_count == roots.len() {
                        DiffStatus::Identical
                    } else {
                        DiffStatus::Subset
                    }
                } else {
                    // 大小相同：計算取樣雜湊確認內容
                    let mut hash_match = true;
                    let mut first_hash = None;

                    for (idx, state) in panel_states.iter_mut().enumerate() {
                        if let DiffEntryState::Present { size, hash, .. } = state {
                            let full_path = roots[idx].join(&rel_path);
                            let computed = compute_file_sample_hash(&full_path, *size).unwrap_or(0);
                            *hash = computed;

                            if let Some(h) = first_hash {
                                if computed != h {
                                    hash_match = false;
                                }
                            } else {
                                first_hash = Some(computed);
                            }
                        }
                    }

                    if hash_match {
                        if present_count == roots.len() {
                            DiffStatus::Identical
                        } else {
                            DiffStatus::Subset
                        }
                    } else {
                        DiffStatus::Modified
                    }
                }
            };

            rows.push(DiffMatrixRow {
                relative_path: rel_path,
                is_dir,
                panel_states,
                status,
                display_size: max_size,
            });
        }

        sort_diff_rows(&mut rows);
        let _ = sender.send(DiffJobEvent::Done(rows));
    });
}

/// 同步計算差異矩陣（供單元測試或本機小目錄使用）。
#[allow(dead_code)]
pub fn compute_diff_matrix(roots: &[PathBuf]) -> io::Result<Vec<DiffMatrixRow>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    spawn_background_diff(roots.to_vec(), true, true, cancelled, tx);

    let mut final_rows = Vec::new();
    while let Ok(event) = rx.recv() {
        match event {
            DiffJobEvent::Done(rows) => {
                final_rows = rows;
                break;
            }
            DiffJobEvent::Error(err) => {
                return Err(io::Error::new(io::ErrorKind::Other, err));
            }
            DiffJobEvent::Discovered(_) => {}
        }
    }
    Ok(final_rows)
}

/// 檢查系統環境中是否存在某命令。
fn is_command_available(cmd: &str) -> bool {
    super::tools::is_command_in_path(cmd)
}

/// 依據比對數量與系統已安裝工具，產生最佳的外部 Diff 檢視命令規格。
pub fn launch_content_diff_spec(roots: &[PathBuf], row: &DiffMatrixRow) -> Option<LaunchSpec> {
    if row.is_dir {
        return None;
    }

    // 收集該檔案實際存在的所有完整路徑
    let existing_paths: Vec<PathBuf> = roots
        .iter()
        .zip(row.panel_states.iter())
        .filter_map(|(root, state)| {
            if matches!(state, DiffEntryState::Present { is_dir: false, .. }) {
                Some(root.join(&row.relative_path))
            } else {
                None
            }
        })
        .collect();

    if existing_paths.is_empty() {
        return None;
    }

    if existing_paths.len() == 1 {
        // 僅單端存在時，以使用者預設編輯器或 pager 查看
        let target = existing_paths[0].to_string_lossy().into_owned();
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| String::from("vim"));
        return Some(LaunchSpec {
            program: editor,
            args: vec![target],
            mode: LaunchMode::TerminalBlocking,
        });
    }

    if existing_paths.len() == 2 {
        let p1 = existing_paths[0].to_string_lossy().into_owned();
        let p2 = existing_paths[1].to_string_lossy().into_owned();

        // 優先度 1: delta (git-delta)
        if is_command_available("delta") {
            return Some(LaunchSpec {
                program: String::from("delta"),
                args: vec![String::from("--side-by-side"), p1, p2],
                mode: LaunchMode::TerminalBlocking,
            });
        }

        // 優先度 2: difftastic (difft)
        if is_command_available("difft") {
            return Some(LaunchSpec {
                program: String::from("difft"),
                args: vec![p1, p2],
                mode: LaunchMode::TerminalBlocking,
            });
        }

        // 優先度 3: nvim -d
        if is_command_available("nvim") {
            return Some(LaunchSpec {
                program: String::from("nvim"),
                args: vec![String::from("-d"), p1, p2],
                mode: LaunchMode::TerminalBlocking,
            });
        }

        // 優先度 4: vimdiff
        if is_command_available("vimdiff") {
            return Some(LaunchSpec {
                program: String::from("vimdiff"),
                args: vec![p1, p2],
                mode: LaunchMode::TerminalBlocking,
            });
        }

        // 通用 Fallback: diff -u
        return Some(LaunchSpec {
            program: String::from("diff"),
            args: vec![String::from("-u"), p1, p2],
            mode: LaunchMode::TerminalBlocking,
        });
    }

    // 3 個或 4 個以上檔案比對
    let mut args = vec![String::from("-d")];
    for p in &existing_paths {
        args.push(p.to_string_lossy().into_owned());
    }

    if is_command_available("nvim") {
        return Some(LaunchSpec {
            program: String::from("nvim"),
            args,
            mode: LaunchMode::TerminalBlocking,
        });
    }

    if is_command_available("vim") {
        return Some(LaunchSpec {
            program: String::from("vim"),
            args,
            mode: LaunchMode::TerminalBlocking,
        });
    }

    // Fallback: 呼叫 delta 兩兩比對前兩者
    let p1 = existing_paths[0].to_string_lossy().into_owned();
    let p2 = existing_paths[1].to_string_lossy().into_owned();
    if is_command_available("delta") {
        return Some(LaunchSpec {
            program: String::from("delta"),
            args: vec![String::from("--side-by-side"), p1, p2],
            mode: LaunchMode::TerminalBlocking,
        });
    }

    Some(LaunchSpec {
        program: String::from("diff"),
        args: vec![String::from("-u"), p1, p2],
        mode: LaunchMode::TerminalBlocking,
    })
}
