//! PaneFM 的應用狀態機、命令分派與使用者操作流程。
//!
//! [`App`] 是第一層協調者：每個 panel 的瀏覽資料由 `PaneState` 保存，全域只保留
//! layout、焦點、剪貼簿與背景 task。`handle_key` 會按照「暫時 UI -> 文字輸入 ->
//! panel 模式 -> 一般列表」的優先順序分派事件，避免同一按鍵同時觸發兩種行為。
//!
//! 新功能應盡量把檔案系統或平台細節放進對應模組，這裡只保留狀態轉換；所有會
//! 阻塞的搜尋、外部程式或網路操作都必須排入背景流程，不能卡住 TUI 主執行緒。

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ignore::{WalkBuilder, WalkState};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

use crate::{
    config::{AppConfig, LoadedConfig, StartupSort, persist_theme},
    theme::{Theme, ThemePreset},
};

use super::{
    archive::{
        ExtractedArchive, compress_entries_to_zip, compress_entries_to_zip_with_progress,
        extract_entries, extract_entries_with_progress,
    },
    bookmark::{BookmarkEntry, BookmarkStore, BookmarkTarget, bookmark_file_path},
    copy::{CopyAction, build_copy_text, copy_action_status_label, copy_picker_options},
    debug_timing_log, debug_timing_message,
    diff::{DiffJobEvent, DiffMatrixState, launch_content_diff_spec, spawn_background_diff},
    filesystem_watcher::FilesystemWatcher,
    fuzzy::{fuzzy_matched_indices, fuzzy_matched_indices_by_fields},
    layout::{LayoutNode, SplitDirection, SplitPlacement},
    open::{
        LaunchSpec, OpenAction, OpenPickerAction, OpenPickerOption, OpenTarget,
        build_custom_launch_spec, build_launch_spec, build_terminal_launch_spec,
        custom_action_applies_to_target, default_open_action, open_picker_options,
    },
    operation_history::{
        DEFAULT_HISTORY_LIMIT, FileOperation, FileOperationKind, OperationHistory, OperationItem,
    },
    pane::{
        DirectoryLoadProgress, FilterMode, LineMode, PaneState, SortDetailKind, SortMode,
        TransferProgress, path_content_size,
    },
    platform::write_text_to_system_clipboard,
    search::{
        GlobalSearchEntry, GlobalSearchEvent, stream_content_search_entries, stream_search_entries,
    },
    smb::{ResolvedSmbLocation, build_smb_mount_launch, parse_smb_location},
    task_history::{load_task_history, save_task_history, task_history_file_path},
    tools::external_tool_statuses,
    trash::{TrashListEntry, TrashStore},
    ui::{
        BookmarkPanelLine, CommandSuggestionLine, HelpPanelLine, InlineEditorState,
        InlinePickerState, PaneListState, RegexRenamePanelLine, SearchListState, TaskPanelLine,
        TrashPanelLine, ZoxidePanelLine, render_bookmark_action_picker, render_bookmark_picker,
        render_command_palette, render_confirm_dialog, render_diff_matrix, render_filter_input,
        render_global_search_panel, render_go_picker, render_linemode_picker, render_pane,
        render_paste_overwrite_dialog, render_preview_search_input, render_theme_command_picker,
        render_theme_picker, render_trash_confirm_dialog, render_window_picker,
        render_zoxide_picker, visible_list_window_range,
    },
    zoxide::{ZoxideTracker, query_zoxide_directories},
};

#[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
use super::smb::resolve_smb_location;

#[cfg(any(all(not(target_os = "windows"), not(target_os = "macos")), test))]
use super::smb::resolve_smb_location_with_mount_root;

/// 表示 rename 輸入框目前採用的編輯模式。
///
/// `Insert` 代表可以直接插入文字，游標會顯示成細線；
/// `Normal` 代表遵循 Vim 的一般模式，只負責移動游標與切換模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameMode {
    Insert,
    Normal,
}

/// 表示共用文字輸入器處理按鍵後的結果。
///
/// 各輸入 UI 只需要處理自己的 Enter 與關閉行為；字元插入、刪除、游標移動及
/// Vim 模式切換都由這個結果統一描述，避免不同介面各自實作後產生操作差異。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextEditResult {
    Changed,
    Consumed,
    PassThrough,
}

/// 表示目前剪貼簿保存的是複製還是剪下操作。
///
/// 這個模式會決定 `p` 貼上時，是保留來源還是把來源移動到新位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardOperation {
    Copy,
    Cut,
}

/// 記錄目前暫存在檔案管理器內部剪貼簿中的單一項目。
///
/// 這一版先支援單一檔案或資料夾，之後若要擴充多選，
/// 可以再把這個結構改成清單形式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardEntry {
    pub(crate) source_path: PathBuf,
    pub(crate) display_name: String,
}

/// 表示目前內部剪貼簿保存的一批項目與其操作模式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardState {
    pub(crate) entries: Vec<ClipboardEntry>,
    pub(crate) operation: ClipboardOperation,
}

/// 記錄目前 filter 的目標 pane、查詢字串、比對模式與是否仍在輸入中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
    pub(crate) mode: FilterMode,
}

/// 記錄目前 preview search 的目標 pane、查詢字串與是否仍在輸入中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreviewSearchState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
}

/// 記錄目前列表內 find-next 的目標 pane 與輸入中的查詢字串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListFindState {
    pub(crate) pane_id: usize,
    pub(crate) buffer: String,
}

/// 描述目前 pane 已排隊、準備交給主事件迴圈執行的 `fzf` 跳轉請求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FzfJumpRequest {
    pub(crate) pane_id: usize,
    pub(crate) root_dir: PathBuf,
    pub(crate) show_hidden: bool,
    pub(crate) follow_links: bool,
    pub(crate) task_id: usize,
}

/// 記錄目前 global search 的目標 pane、查詢文字與搜尋結果狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GlobalSearchState {
    pub(crate) pane_id: usize,
    pub(crate) root_dir: PathBuf,
    pub(crate) mode: SearchMode,
    pub(crate) buffer: String,
    pub(crate) editing: bool,
    pub(crate) loading: bool,
    pub(crate) searched: bool,
    pub(crate) selected: usize,
    pub(crate) results: Vec<GlobalSearchEntry>,
    /// 只過濾已回傳結果的模糊 filter，不會重新執行 `fd` 或 `rg` 搜尋。
    pub(crate) filter: PanelSearchState,
    pub(crate) preview_scroll: Option<usize>,
    pub(crate) preview_current_match: Option<usize>,
    pub(crate) task_id: Option<usize>,
}

/// 表示目前搜尋面板是在找路徑，還是在找檔案內容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchMode {
    Path,
    Content,
}

impl SearchMode {
    /// 回傳狀態列與說明文字使用的搜尋模式標籤。
    fn status_label(self) -> &'static str {
        match self {
            SearchMode::Path => "global search",
            SearchMode::Content => "content search",
        }
    }

    /// 回傳搜尋輸入框標題。
    fn panel_title(self, _editing: bool) -> &'static str {
        match self {
            SearchMode::Path => " Global search file by fd ",
            SearchMode::Content => " Global search content by rg ",
        }
    }
}

/// 描述目前 task manager 中單一任務的狀態。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskState {
    Running,
    Done,
    Failed,
    Cancelled,
    Interrupted,
}

/// 描述單一背景或外部任務在 task manager 中的紀錄。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskRecord {
    pub(crate) id: usize,
    pub(crate) pane_id: usize,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) detail: String,
    /// 任務讀取或操作的來源位置。多選 copy、move、compress、extract 與 delete 會保留
    /// 每一筆來源，讓 task 歷史在工作完成後仍能回答「資料從哪裡來」。
    #[serde(default)]
    pub(crate) source_locations: Vec<String>,
    /// 任務寫入或跳轉的目的位置；純刪除或只讀工作沒有目的地時為 `None`。
    #[serde(default)]
    pub(crate) destination_location: Option<String>,
    pub(crate) state: TaskState,
    /// 背景檔案工作目前完成百分比；不支援進度的外部工作使用 `None`。
    ///
    /// 這個欄位只為向下相容舊版 `task-history.json` 保留；新介面改顯示原始 byte，
    /// 避免百分比掩蓋大型傳輸實際有沒有繼續前進。
    #[serde(default)]
    pub(crate) progress_percent: Option<u8>,
    /// 背景工作目前已完成的 byte；舊歷史沒有這個欄位時為 `None`。
    #[serde(default)]
    pub(crate) completed_bytes: Option<u64>,
    /// 背景工作目前已知或估算的總 byte；走訪目錄期間可持續增加。
    #[serde(default)]
    pub(crate) total_bytes: Option<u64>,
    pub(crate) started_at_unix_ms: u64,
    pub(crate) finished_at_unix_ms: Option<u64>,
}

/// 描述排隊中的外部命令與它對應的 task id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueuedLaunch {
    pub(crate) task_id: usize,
    pub(crate) launch: LaunchSpec,
}

/// 描述一次 UNC 網路路徑背景跳轉的完成訊息。
///
/// worker 會在主執行緒之外複製並載入 [`PaneState`]；主迴圈收到結果後，只有在
/// task 尚未被取消且目標 panel 仍存在時才套用，避免失聯 SMB 主機凍結整個 TUI。
#[derive(Debug)]
struct NetworkGotoEvent {
    /// 對應 task manager 中的任務編號。
    task_id: usize,
    /// 啟動跳轉時的 active panel 編號。
    pane_id: usize,
    /// 使用者輸入的 UNC 目標，供狀態列與錯誤訊息顯示。
    target: PathBuf,
    /// 背景載入完成的 panel 狀態，或作業系統回傳的 I/O 錯誤。
    result: io::Result<PaneState>,
}

/// 大型檔案工作完成後送回主執行緒的事件。
///
/// worker 只處理檔案 I/O，不直接修改 [`App`]；操作歷史、panel reload 與狀態列都在
/// 主迴圈收到事件後更新，避免跨執行緒共享可變 UI 狀態。
#[derive(Debug)]
enum FileJobEvent {
    /// 背景貼上已建立第一層目標，主執行緒可立即刷新目的 panel，不必等待整批完成。
    DestinationVisible { target_dir: PathBuf },
    /// worker 定期回報累計 byte，主執行緒直接更新 task 面板與持久化歷史。
    Progress {
        task_id: usize,
        completed_bytes: u64,
        total_bytes: u64,
    },
    Paste {
        task_id: usize,
        clipboard: ClipboardState,
        overwrite: bool,
        result: PasteJobResult,
    },
    Compress {
        task_id: usize,
        pane_id: usize,
        entry_count: usize,
        first_name: String,
        result: io::Result<PathBuf>,
    },
    Extract {
        task_id: usize,
        pane_id: usize,
        result: io::Result<(Vec<ExtractedArchive>, usize)>,
    },
    Delete {
        task_id: usize,
        target_name: String,
        result: io::Result<Vec<String>>,
    },
}

/// 目錄大小 worker 傳回主執行緒的增量事件。
#[derive(Debug)]
enum DirectorySizeEvent {
    /// 單一直接子目錄目前已統計的 byte，以及該子樹是否已完成。
    Update {
        path: PathBuf,
        bytes: u64,
        complete: bool,
    },
    /// 目前 panel 啟動的整批直接子目錄都已完成或已取消。
    Done,
}

/// 保存單一 panel 的目錄大小背景工作。
///
/// `cwd` 用來拒絕切換目錄後晚到的舊結果；`cancelled` 讓新掃描取代舊掃描時，舊
/// worker 能在下一個檔案邊界停止，不會持續佔用磁碟或 SMB 連線。
#[derive(Debug)]
struct DirectorySizeJob {
    cwd: PathBuf,
    receiver: Receiver<DirectorySizeEvent>,
    cancelled: Arc<AtomicBool>,
}

/// 保存單一 panel 的目錄清單背景載入工作。
///
/// `cwd` 用來比對當前目錄；`cancelled` 讓新導航發生時，舊 worker 能立即在分塊邊界停止，
/// 避免背景磁碟 I/O 阻塞主事件迴圈。
#[derive(Debug)]
struct DirectoryLoadJob {
    cwd: PathBuf,
    receiver: Receiver<DirectoryLoadEvent>,
    cancelled: Arc<AtomicBool>,
}

/// 大型目錄背景讀取完成或分批串流送回主迴圈的資料。
#[derive(Debug)]
struct DirectoryLoadEvent {
    pane_id: usize,
    cwd: PathBuf,
    selected_path: Option<PathBuf>,
    result: io::Result<DirectoryLoadProgress>,
}

/// `ms` 背景掃描回報部分容量的最小間隔。
///
/// 200ms 可讓大型目錄的數字明顯持續前進，同時不會為每個檔案都傳送事件而
/// 壓垮 TUI 主執行緒。
const DIRECTORY_SIZE_UPDATE_INTERVAL_MS: u64 = 200;

/// 背景 paste 完成的批次結果，包含成功項目及第一個失敗原因。
#[derive(Debug)]
struct PasteJobResult {
    history_items: Vec<OperationItem>,
    pasted_count: usize,
    failure: Option<PasteJobFailure>,
}

/// 記錄背景 paste 的失敗項目，供主執行緒顯示完整來源、目的與 OS error。
#[derive(Debug)]
struct PasteJobFailure {
    display_name: String,
    planned_target: PathBuf,
    error: io::Error,
}

/// 記錄目前是否處於範圍標記模式，以及起點和目前游標位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisualSelectionState {
    pub(crate) pane_id: usize,
    pub(crate) anchor: usize,
    pub(crate) current: usize,
}

/// 表示目前是否正在等待使用者補上書籤按鍵。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BookmarkPrompt {
    Jump,
}

/// 描述目前書籤列表面板是用來跳轉，還是用來刪除書籤。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BookmarkListMode {
    Jump,
    Delete,
}

/// 描述目前待確認的 trash 操作種類。
///
/// 這裡會把「直接復原最後一筆」與「在 trash 面板內針對項目做刪除/還原」
/// 統一收斂成同一套確認流程，避免不同入口各自維護一份邏輯。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TrashConfirmAction {
    RestoreFromPanel {
        pane_id: usize,
        target_ids: Vec<String>,
        search: PanelSearchState,
        selected: usize,
    },
    DeleteFromPanel {
        pane_id: usize,
        target_ids: Vec<String>,
        search: PanelSearchState,
        selected: usize,
    },
}

/// 描述暫時面板中的搜尋輸入狀態。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PanelSearchState {
    pub(crate) buffer: String,
    pub(crate) editing: bool,
}

/// 描述 regex 批次改名預覽中每一列的運算結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegexRenamePreview {
    pub(crate) source_path: PathBuf,
    pub(crate) original_name: String,
    pub(crate) new_name: String,
    pub(crate) outcome: RegexRenameOutcome,
}

/// 表示 regex 批次改名預覽中這一列目前是可套用、無變化或有衝突。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegexRenameOutcome {
    Ready,
    Unchanged,
    Conflict,
    Invalid,
}

/// 表示目前正在等待使用者完成的暫時互動。
///
/// 只要有 pending action，輸入會先被它攔截，
/// 而不會直接進到一般檔案瀏覽模式。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PendingAction {
    ConfirmDelete {
        pane_id: usize,
        target_name: String,
        permanent: bool,
    },
    ConfirmPasteOverwrite {
        pane_id: usize,
        target_name: String,
        entry_count: usize,
        operation: ClipboardOperation,
    },
    ConfirmTrashAction {
        action: TrashConfirmAction,
        target_name: String,
        entry_count: usize,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    },
    SortPicker {
        pane_id: usize,
    },
    GoPicker {
        pane_id: usize,
    },
    WindowPicker {
        pane_id: usize,
    },
    LineModePicker {
        pane_id: usize,
    },
    ThemePicker {
        selected: usize,
        original: ThemePreset,
    },
    ThemeCommandPicker {
        pane_id: usize,
    },
    TrashPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
        marked_ids: Vec<String>,
        visual_anchor: Option<usize>,
    },
    HelpPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
    },
    TaskPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
    },
    BookmarkPicker {
        pane_id: usize,
    },
    BookmarkList {
        pane_id: usize,
        selected: usize,
        mode: BookmarkListMode,
        search: PanelSearchState,
    },
    ZoxideList {
        pane_id: usize,
        selected: usize,
        entries: Vec<PathBuf>,
        search: PanelSearchState,
    },
    ToolPanel {
        pane_id: usize,
        selected: usize,
    },
    CopyPicker {
        pane_id: usize,
        target: OpenTarget,
        selected: usize,
    },
    OpenPicker {
        pane_id: usize,
        target: OpenTarget,
        selected: usize,
        options: Vec<OpenPickerOption>,
    },
    Rename {
        pane_id: usize,
        original_name: String,
        buffer: String,
        cursor: usize,
        mode: RenameMode,
    },
    CreateEntry {
        pane_id: usize,
        buffer: String,
        cursor: usize,
        mode: RenameMode,
    },
    RegexRename {
        pane_id: usize,
        pattern: String,
        replacement: String,
        selected: usize,
        previews: Vec<RegexRenamePreview>,
    },
    DiffMatrix(DiffMatrixState),
}

/// 表示整個應用程式的核心狀態。
///
/// 這個結構整合了設定、主題、視窗布局、焦點與互動模式，
/// 是整個 TUI 運作時最主要的狀態容器。
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) config: AppConfig,
    pub(crate) config_source: PathBuf,
    pub(crate) theme: Theme,
    pub(crate) theme_preset: ThemePreset,
    pub(crate) trash_store: TrashStore,
    pub(crate) bookmark_store: BookmarkStore,
    pub(crate) panes: BTreeMap<usize, PaneState>,
    pub(crate) layout: LayoutNode,
    pub(crate) focused_pane: usize,
    pub(crate) next_pane_id: usize,
    pub(crate) status: String,
    pub(crate) command_mode: bool,
    pub(crate) command_buffer: String,
    /// 所有非 inline 文字輸入 UI 共用的 Vim 編輯模式。
    pub(crate) text_input_mode: RenameMode,
    /// 所有非 inline 文字輸入 UI 共用的字元游標位置，不是 UTF-8 byte offset。
    pub(crate) text_input_cursor: usize,
    pub(crate) command_suggestion_selected: usize,
    pub(crate) command_completion_cycle: Option<CommandCompletionCycle>,
    pub(crate) pending_count: Option<usize>,
    pub(crate) pending_g: bool,
    pub(crate) pending_y: bool,
    pub(crate) pending_bookmark: Option<BookmarkPrompt>,
    pub(crate) clipboard: Option<ClipboardState>,
    /// 全域檔案操作歷史；跨 panel 的 copy/move 仍應以同一批次復原。
    pub(crate) operation_history: OperationHistory,
    pub(crate) filter: Option<FilterState>,
    pub(crate) preview_search: Option<PreviewSearchState>,
    pub(crate) list_find: Option<ListFindState>,
    pub(crate) global_search: Option<GlobalSearchState>,
    pub(crate) global_search_rx: Option<Receiver<GlobalSearchEvent>>,
    pub(crate) global_search_cancelled: Option<Arc<AtomicBool>>,
    pub(crate) active_global_search_task_id: Option<usize>,
    pub(crate) diff_job_rx: Option<Receiver<DiffJobEvent>>,
    pub(crate) diff_job_cancelled: Option<Arc<AtomicBool>>,
    /// 目前 UNC `goto` 背景工作的接收端；`None` 代表沒有等待中的網路跳轉。
    network_goto_rx: Option<Receiver<NetworkGotoEvent>>,
    /// 目前 UNC `goto` 對應的 task id，供 Esc 與 task panel 取消後捨棄晚到結果。
    active_network_goto_task_id: Option<usize>,
    /// 所有大型 paste/compress/extract 工作接收端，以 task id 區分並允許並行完成。
    file_job_receivers: BTreeMap<usize, Receiver<FileJobEvent>>,
    /// 記錄目前正在由背景工作處理（寫入、壓縮、解壓、刪除）的路徑集合，用來防止使用者在傳輸中途進入未完成的目錄。
    active_file_job_busy_paths: BTreeMap<usize, Vec<PathBuf>>,
    /// 每個 panel 各自擁有的 linemode size 背景掃描，不會互相覆蓋或阻塞 TUI。
    directory_size_jobs: BTreeMap<usize, DirectorySizeJob>,
    /// 每個 panel 最新一次非阻塞目錄讀取；新導航會取代舊 worker 並即時取消舊掃描。
    directory_load_jobs: BTreeMap<usize, DirectoryLoadJob>,
    /// 已成功讀取的目錄清單快取；重複進出大型目錄時先立即顯示，再由背景結果校正。
    directory_entry_cache: BTreeMap<PathBuf, Vec<super::entry::FileEntry>>,
    pub(crate) visual_selection: Option<VisualSelectionState>,
    pub(crate) pending_action: Option<PendingAction>,
    pub(crate) help_return: Option<HelpReturnState>,
    pub(crate) pending_launch: Option<QueuedLaunch>,
    pub(crate) pending_fzf_jump: Option<FzfJumpRequest>,
    pub(crate) task_log: Vec<TaskRecord>,
    pub(crate) next_task_id: usize,
    /// task 歷史的實際檔案位置；每次狀態變更與關閉前都會同步寫入。
    pub(crate) task_history_path: PathBuf,
    /// 非阻塞記錄瀏覽目錄，避免同步啟動 zoxide 拖慢 TUI。
    pub(crate) zoxide_tracker: ZoxideTracker,
    /// 監看 Finder、Explorer 與其他程式對目前 panel 目錄造成的外部變更。
    filesystem_watcher: Option<FilesystemWatcher>,
    /// watcher 短時間內回報的目錄先集中在這裡，等 debounce 到期再一起刷新。
    pending_watched_directories: BTreeSet<PathBuf>,
    /// 下一次允許套用 watcher 刷新的時間；`None` 代表目前沒有待處理事件。
    filesystem_refresh_deadline: Option<Instant>,
    /// 要求主事件迴圈在下一幀前清除實體 terminal 與 ratatui buffer。
    pub(crate) full_redraw_requested: bool,
}

/// 記錄 F1 help 關閉後應回復到哪一種互動上下文。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HelpReturnState {
    Pending(PendingAction),
    Filter(FilterState),
    PreviewSearch(PreviewSearchState),
    ListFind(ListFindState),
    GlobalSearch(GlobalSearchState),
    VisualSelection(VisualSelectionState),
    CommandMode(String),
    PendingBookmark(BookmarkPrompt),
    PreviewFocus(usize),
}

/// 記錄 command mode 目前是否正拿同一組路徑候選做 Tab 輪詢補全。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandCompletionCycle {
    pub(crate) suggestions: Vec<CommandSuggestionLine>,
}

impl App {
    /// 建立一個新的應用程式狀態。
    ///
    /// 參數：
    /// - `cwd: PathBuf`，啟動時第一個 pane 要打開的目錄。
    /// - `loaded_config: LoadedConfig`，啟動時已載入的設定與來源資訊。
    ///
    /// 回傳：`io::Result<App>`。
    /// - 成功時回傳完整初始化的應用程式狀態。
    /// - 失敗時回傳建立第一個 pane 或載入目錄時的 I/O 錯誤。
    pub(crate) fn new(cwd: PathBuf, loaded_config: LoadedConfig) -> io::Result<Self> {
        let trash_store = TrashStore::new(&cwd)?;
        let LoadedConfig { config, source } = loaded_config;
        let config_source = source.clone().unwrap_or_else(|| cwd.join("config.toml"));
        let task_history_path = task_history_file_path(&cwd, source.as_deref());
        let (mut task_log, task_history_warning) = match load_task_history(&task_history_path) {
            Ok(tasks) => (tasks, None),
            Err(error) => (
                Vec::new(),
                Some(format!("task history could not be loaded: {error}")),
            ),
        };
        let now = unix_time_ms_now();
        let mut recovered_interrupted_tasks = 0usize;
        for task in &mut task_log {
            // 上次程序若在 task 尚未完成時被關閉，背景 thread 已隨 process 消失。
            // 不可把它繼續顯示成 RUNNING，更不可未經確認就重新覆寫 SMB 目標。
            if task.state == TaskState::Running {
                task.state = TaskState::Interrupted;
                task.finished_at_unix_ms = Some(now);
                task.detail = format!("{}; interrupted when PaneFM closed", task.detail);
                recovered_interrupted_tasks += 1;
            }
            // 新 session 只建立 panel #1；把歷史歸到可見 panel，避免舊 panel id 讓
            // 使用者按 T 後找不到紀錄。原始目標仍完整保存在 title/detail。
            task.pane_id = 1;
        }
        if task_log.len() > 200 {
            let overflow = task_log.len() - 200;
            task_log.drain(0..overflow);
        }
        let next_task_id = task_log
            .iter()
            .map(|task| task.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let bookmark_store = BookmarkStore::load(bookmark_file_path(&cwd, source.as_deref()))
            .map_err(|error| io::Error::other(error.to_string()))?;
        let zoxide_tracker = ZoxideTracker::new();
        zoxide_tracker.track(&cwd);
        let mut pane = PaneState::new(cwd)?;
        apply_config_to_pane(&config, &mut pane);
        let mut panes = BTreeMap::new();
        panes.insert(1, pane);
        let theme_preset = config.ui.theme_preset;
        #[cfg(not(test))]
        let (filesystem_watcher, watcher_startup_warning) = if config.watcher.enabled {
            match FilesystemWatcher::new(config.watcher.fallback_poll_interval) {
                Ok(watcher) => (Some(watcher), None),
                Err(error) => (
                    None,
                    Some(format!("filesystem watcher unavailable: {error}")),
                ),
            }
        } else {
            (None, None)
        };
        // 單元測試會直接注入變更目錄驗證刷新邏輯，不為每個 App 測試建立兩條
        // 作業系統 watcher thread，避免數百個測試同時消耗平台資源。
        #[cfg(test)]
        let (filesystem_watcher, watcher_startup_warning): (
            Option<FilesystemWatcher>,
            Option<String>,
        ) = (None, None);
        let startup_status = match source {
            Some(path) => format!("loaded config: {}", path.display()),
            None => String::from("normal mode"),
        };
        let missing_tools = external_tool_statuses()
            .into_iter()
            .filter(|tool| !tool.installed)
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        let mut startup_status = if missing_tools.is_empty() {
            startup_status
        } else {
            format!(
                "{startup_status}; missing dependencies: {}",
                missing_tools.join(", ")
            )
        };
        if let Some(warning) = task_history_warning {
            startup_status = format!("{startup_status}; {warning}");
        } else if recovered_interrupted_tasks > 0 {
            startup_status = format!(
                "{startup_status}; recovered {recovered_interrupted_tasks} interrupted task(s)"
            );
        }
        if let Some(warning) = watcher_startup_warning {
            startup_status = format!("{startup_status}; {warning}");
        }

        let app = Self {
            config,
            config_source,
            theme: theme_preset.into(),
            theme_preset,
            trash_store,
            bookmark_store,
            panes,
            layout: LayoutNode::Leaf { pane_id: 1 },
            focused_pane: 1,
            next_pane_id: 2,
            status: startup_status,
            command_mode: false,
            command_buffer: String::new(),
            text_input_mode: RenameMode::Insert,
            text_input_cursor: 0,
            command_suggestion_selected: 0,
            command_completion_cycle: None,
            pending_count: None,
            pending_g: false,
            pending_y: false,
            pending_bookmark: None,
            clipboard: None,
            operation_history: OperationHistory::new(DEFAULT_HISTORY_LIMIT),
            filter: None,
            preview_search: None,
            list_find: None,
            global_search: None,
            global_search_rx: None,
            global_search_cancelled: None,
            active_global_search_task_id: None,
            diff_job_rx: None,
            diff_job_cancelled: None,
            network_goto_rx: None,
            active_network_goto_task_id: None,
            file_job_receivers: BTreeMap::new(),
            active_file_job_busy_paths: BTreeMap::new(),
            directory_size_jobs: BTreeMap::new(),
            directory_load_jobs: BTreeMap::new(),
            directory_entry_cache: BTreeMap::new(),
            visual_selection: None,
            pending_action: None,
            help_return: None,
            pending_launch: None,
            pending_fzf_jump: None,
            task_log,
            next_task_id,
            task_history_path,
            zoxide_tracker,
            filesystem_watcher,
            pending_watched_directories: BTreeSet::new(),
            filesystem_refresh_deadline: None,
            full_redraw_requested: false,
        };
        if recovered_interrupted_tasks > 0 {
            save_task_history(&app.task_history_path, &app.task_log)?;
        }
        Ok(app)
    }

    /// 嘗試把目前按鍵視為 count prefix 的下一個數字。
    ///
    /// 規則：
    /// - `1..=9` 永遠可以開始或延續 count。
    /// - `0` 只有在已經有 count 時，才會被視為後續位數。
    fn capture_pending_count_digit(&mut self, key: &KeyEvent) -> bool {
        if !key.modifiers.is_empty() {
            return false;
        }

        let KeyCode::Char(ch) = key.code else {
            return false;
        };

        if !ch.is_ascii_digit() {
            return false;
        }
        if ch == '0' && self.pending_count.is_none() {
            return false;
        }

        let digit = ch.to_digit(10).unwrap_or(0) as usize;
        let next = self
            .pending_count
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(digit);
        self.pending_count = Some(next);
        self.status = format!("count: {next}");
        true
    }

    /// 取出目前暫存的 count prefix。
    fn take_pending_count(&mut self) -> Option<usize> {
        self.pending_count.take()
    }

    /// 取出目前暫存的 count；若沒有則回傳 1。
    fn take_count_or_one(&mut self) -> usize {
        self.take_pending_count().unwrap_or(1).max(1)
    }

    /// 取出目前 count，並轉成固定大步長移動的實際步數。
    fn take_large_move_step(&mut self) -> usize {
        self.take_count_or_one()
            .saturating_mul(self.config.navigation.fast_move_step.max(1))
    }

    /// 取出目前 count，並轉成一般彈窗列表使用的 page 步長。
    fn take_panel_page_step(&mut self) -> usize {
        self.take_count_or_one()
            .saturating_mul(self.config.navigation.panel_page_step.max(1))
    }

    /// 清除目前暫存的 count prefix。
    fn clear_pending_count(&mut self) {
        self.pending_count = None;
    }

    /// 清除和一般移動相關的暫存狀態，例如 count、pending g、pending y。
    fn reset_pending_motion_state(&mut self) {
        self.clear_pending_count();
        self.pending_g = false;
        self.pending_y = false;
    }

    /// 打開 command UI，並可選擇先填入一段命令前綴，方便使用者直接補參數。
    fn open_prefilled_command(&mut self, prefill: impl Into<String>) {
        self.command_mode = true;
        self.command_buffer = prefill.into();
        self.begin_text_input_at_end();
        self.command_suggestion_selected = 0;
        self.command_completion_cycle = None;
        self.status = String::from("command mode");
        self.pending_g = false;
        self.pending_y = false;
        self.pending_bookmark = None;
    }

    /// 讓新開啟的文字輸入 UI 從 Insert 模式開始，並把游標放到文字尾端。
    ///
    /// 參數：無，文字內容會從目前的 `command_buffer` 取得；其他輸入框可在設定
    /// buffer 後直接重設這兩個欄位。
    /// 回傳：`()`, 只更新共用文字編輯狀態。
    fn begin_text_input_at_end(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = self.command_buffer.chars().count();
    }

    /// 使用共用 Vim 規則編輯指定字串。
    ///
    /// 參數：
    /// - `buffer: &mut String`，目前輸入框的 UTF-8 文字。
    /// - `key: &KeyEvent`，正規化後要處理的鍵盤事件。
    ///
    /// 回傳：`TextEditResult`，指出內容已改變、按鍵已被模式處理，或應交回 UI
    /// 處理 Enter、Tab、空輸入框的第一次 Esc，以及 Normal 模式下第二次 Esc 等
    /// 業務行為。空輸入框沒有可供 Vim 游標修正的文字，因此 Esc 會直接交回 UI 關閉。
    fn edit_text_buffer(&mut self, buffer: &mut String, key: &KeyEvent) -> TextEditResult {
        self.text_input_cursor = self.text_input_cursor.min(buffer.chars().count());
        match self.text_input_mode {
            RenameMode::Insert => match key.code {
                KeyCode::Char(_) => {
                    if let Some(character) = typed_char_from_key(key) {
                        insert_char(buffer, &mut self.text_input_cursor, character);
                        TextEditResult::Changed
                    } else {
                        TextEditResult::Consumed
                    }
                }
                KeyCode::Backspace => {
                    backspace_char(buffer, &mut self.text_input_cursor);
                    TextEditResult::Changed
                }
                KeyCode::Delete => {
                    delete_char_at(buffer, self.text_input_cursor);
                    TextEditResult::Changed
                }
                KeyCode::Left => {
                    self.text_input_cursor = self.text_input_cursor.saturating_sub(1);
                    TextEditResult::Consumed
                }
                KeyCode::Right => {
                    self.text_input_cursor = move_cursor_right(buffer, self.text_input_cursor);
                    TextEditResult::Consumed
                }
                KeyCode::Home => {
                    self.text_input_cursor = 0;
                    TextEditResult::Consumed
                }
                KeyCode::End => {
                    self.text_input_cursor = buffer.chars().count();
                    TextEditResult::Consumed
                }
                KeyCode::Esc => {
                    if buffer.trim().is_empty() {
                        return TextEditResult::PassThrough;
                    }
                    self.text_input_mode = RenameMode::Normal;
                    self.text_input_cursor = normal_cursor(buffer, self.text_input_cursor);
                    TextEditResult::Consumed
                }
                _ => TextEditResult::PassThrough,
            },
            RenameMode::Normal => {
                if key_matches_shifted_letter(key, 'A') {
                    self.text_input_cursor = buffer.chars().count();
                    self.text_input_mode = RenameMode::Insert;
                    return TextEditResult::Consumed;
                }
                match key.code {
                    KeyCode::Left => {
                        self.text_input_cursor = self.text_input_cursor.saturating_sub(1);
                    }
                    KeyCode::Right => {
                        self.text_input_cursor = normal_move_right(buffer, self.text_input_cursor);
                    }
                    KeyCode::Home | KeyCode::Char('0') => self.text_input_cursor = 0,
                    KeyCode::End | KeyCode::Char('$') => {
                        self.text_input_cursor = rename_line_end_cursor(buffer)
                    }
                    _ if key_matches_plain_letter(key, 'h') => {
                        self.text_input_cursor = self.text_input_cursor.saturating_sub(1)
                    }
                    _ if key_matches_plain_letter(key, 'l') => {
                        self.text_input_cursor = normal_move_right(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'w') => {
                        self.text_input_cursor =
                            rename_next_word_start(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'b') => {
                        self.text_input_cursor =
                            rename_previous_word_start(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'e') => {
                        self.text_input_cursor = rename_word_end(buffer, self.text_input_cursor)
                    }
                    _ if key_matches_plain_letter(key, 'i') => {
                        self.text_input_mode = RenameMode::Insert
                    }
                    _ if key_matches_plain_letter(key, 'a') => {
                        self.text_input_cursor = move_cursor_right(buffer, self.text_input_cursor);
                        self.text_input_mode = RenameMode::Insert;
                    }
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab => {
                        return TextEditResult::PassThrough;
                    }
                    _ => {}
                }
                TextEditResult::Consumed
            }
        }
    }

    /// 依照目前互動狀態，把一個鍵盤事件分派給唯一的處理流程。
    ///
    /// 參數：`key: KeyEvent`，已由終端事件迴圈過濾成 Press/Repeat 的按鍵。
    /// 回傳：`Result<bool>`；`true` 代表繼續執行，`false` 只由 quit 流程回傳，錯誤
    /// 則向上交給事件迴圈統一還原 terminal。
    ///
    /// 分派順序不可隨意調換：暫時面板與文字輸入必須先攔截按鍵，否則使用者在
    /// command 輸入 `d` 時可能同時觸發刪除；一般列表快捷鍵永遠是最後一層。
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // Help 被允許從任何上下文打開（F1 永遠支援；~ 僅在非文字輸入狀態下生效，
        // 避免在 filter、goto 或 rename 中輸入 ~ 被誤當成開啟說明）；
        // `help_return` 會保存原上下文，關閉說明後才能回到原 panel/輸入流程。
        if key.code == KeyCode::F(1) || (key_matches_tilde(&key) && !self.is_text_input_active()) {
            if matches!(self.pending_action, Some(PendingAction::HelpPanel { .. })) {
                return self.handle_pending_action_key(key);
            }
            self.open_help_from_current();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if self.pending_action.is_some() {
            return self.handle_pending_action_key(key);
        }
        // 下列輸入狀態互斥，依 UI 層級交給各自 handler；handler 內再共用
        // `edit_text_buffer`，以維持 Insert/Normal、Unicode 游標與 Esc 行為一致。
        if self.filter.as_ref().is_some_and(|filter| filter.editing) {
            return self.handle_filter_input_key(key);
        }
        if self
            .preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
        {
            return self.handle_preview_search_input_key(key);
        }
        if self.list_find.is_some() {
            return self.handle_list_find_input_key(key);
        }
        if self.global_search.is_some() {
            return self.handle_global_search_key(key);
        }
        if self.visual_selection.is_some() {
            return self.handle_visual_selection_key(key);
        }
        if self.command_mode {
            return self.handle_command_key(key);
        }
        if self.pending_bookmark.is_some() {
            return self.handle_bookmark_key(key);
        }
        // UNC 目錄可能被 Windows 網路層長時間阻塞。背景跳轉期間允許 Esc 立即
        // 捨棄接收端並回到一般操作，不必等待作業系統的檔案系統呼叫結束。
        if key.code == KeyCode::Esc && self.active_network_goto_task_id.is_some() {
            self.cancel_network_goto("cancelled by user");
            return Ok(true);
        }
        // 不在輸入或暫時 UI 時，數字與跨 panel 快捷鍵才有意義。這段必須放在
        // count prefix 之前，否則多 panel 下按 2 會被誤解成 `2j` 的前綴。
        if self.panes.len() > 1
            && let Some(target_pane_id) = plain_digit_target_pane_id(&key)
        {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.focus_pane_by_id(target_pane_id);
            return Ok(true);
        }
        if let Some(target_pane_id) = ctrl_digit_target_pane_id(&key) {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.focus_pane_by_id(target_pane_id);
            return Ok(true);
        }
        if key_matches_ctrl_letter(&key, 's') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.split_current(SplitDirection::Horizontal)?;
            return Ok(true);
        }
        if key_matches_ctrl_letter(&key, 'v') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.split_current(SplitDirection::Vertical)?;
            return Ok(true);
        }
        if self
            .panes
            .get(&self.focused_pane)
            .is_some_and(PaneState::is_preview_active)
        {
            return self.handle_preview_key(key);
        }
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }
        if key.code == KeyCode::Tab {
            self.clear_pending_count();
            self.open_preview_focus();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT) {
            self.open_selected_with_picker()?;
            self.reset_pending_motion_state();
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'O') {
            self.open_selected_with_picker()?;
            self.reset_pending_motion_state();
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            if let Some(count) = self.take_pending_count() {
                self.current_pane_mut()?
                    .move_to_visible_index(count.saturating_sub(1));
                self.status = format!("jumped to item {count}");
            } else {
                self.current_pane_mut()?.move_bottom();
                self.status = String::from("jumped to bottom");
            }
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V') {
            self.open_visual_selection()?;
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'c') {
            self.open_copy_picker()?;
            self.reset_pending_motion_state();
            return Ok(true);
        }

        let should_continue = match key.code {
            _ if key_matches_plain_letter(&key, 'q') => false,
            KeyCode::Char(':')
                if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.open_prefilled_command("");
                true
            }
            KeyCode::Char(';') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.open_prefilled_command("");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'p') => {
                self.open_prefilled_command("panel ");
                true
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_down_by(count);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_down_by(step);
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("fast down: {step}");
                true
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_up_by(count);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_up_by(step);
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("fast up: {step}");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                self.clear_pending_count();
                let step = self.current_pane_mut()?.page_down();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("half page down: {step}");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                self.clear_pending_count();
                let step = self.current_pane_mut()?.page_up();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("half page up: {step}");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'f') => {
                self.clear_pending_count();
                let step = self.current_pane_mut()?.full_page_down();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("page down: {step}");
                true
            }
            _ if key_matches_ctrl_letter(&key, 'b') => {
                self.clear_pending_count();
                let step = self.current_pane_mut()?.full_page_up();
                self.pending_g = false;
                self.pending_y = false;
                self.status = format!("page up: {step}");
                true
            }
            _ if key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                let pane_id = self.focused_pane;
                let is_loading = self.directory_load_jobs.contains_key(&pane_id);
                let previous_cwd = self.current_pane_mut()?.cwd.clone();
                let previous_entries = self.current_pane_mut()?.entries.as_slice();
                if !is_loading && !previous_entries.is_empty() {
                    let cached_chunk = if previous_entries.len() > 2000 {
                        previous_entries[..2000].to_vec()
                    } else {
                        previous_entries.to_vec()
                    };
                    self.directory_entry_cache
                        .insert(previous_cwd, cached_chunk);
                }
                if let Some((cwd, selected_path)) = self.current_pane_mut()?.begin_go_parent() {
                    self.start_directory_load(pane_id, cwd, Some(selected_path));
                }
                self.track_focused_pane_cwd_in_zoxide();
                self.status = String::from("moved to parent directory");
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                let pane_id = self.focused_pane;
                if let Some(entry) = self.panes.get(&pane_id).and_then(|p| p.selected_entry()) {
                    if entry.is_dir {
                        if let Some((task_id, title, progress)) =
                            self.active_file_job_for_path(&entry.path)
                        {
                            let pct_str = progress.map(|p| format!(" ({p}%)")).unwrap_or_default();
                            self.status = format!(
                                "cannot enter '{}': transfer in progress [task #{task_id}: {title}{pct_str}]",
                                entry.display_name()
                            );
                            self.pending_g = false;
                            self.pending_y = false;
                            return Ok(true);
                        }
                    }
                }
                let is_loading = self.directory_load_jobs.contains_key(&pane_id);
                let previous_cwd = self.current_pane_mut()?.cwd.clone();
                let previous_entries = self.current_pane_mut()?.entries.as_slice();
                if !is_loading && !previous_entries.is_empty() {
                    let cached_chunk = if previous_entries.len() > 2000 {
                        previous_entries[..2000].to_vec()
                    } else {
                        previous_entries.to_vec()
                    };
                    self.directory_entry_cache
                        .insert(previous_cwd, cached_chunk);
                }
                if let Some(cwd) = self.current_pane_mut()?.begin_enter_selected() {
                    self.start_directory_load(pane_id, cwd, None);
                }
                self.track_focused_pane_cwd_in_zoxide();
                self.status = String::from("opened directory");
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Enter => {
                self.clear_pending_count();
                self.open_selected_with_default()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'o') => {
                self.clear_pending_count();
                self.open_selected_with_default()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                self.pending_y = false;
                self.open_go_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 'd') => {
                self.clear_pending_count();
                self.start_delete_confirmation(false);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'D') => {
                self.clear_pending_count();
                self.start_delete_confirmation(true);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'r') => {
                self.clear_pending_count();
                self.start_rename();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'R') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_prefilled_command("rename-regex ");
                true
            }
            _ if key_matches_plain_letter(&key, 'z') => {
                self.clear_pending_count();
                self.open_fzf_jump();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'Z') => {
                self.clear_pending_count();
                self.open_zoxide_list();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_ctrl_shift_letter(&key, 'a') => {
                self.clear_pending_count();
                self.clear_marks_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_ctrl_letter(&key, 'a') => {
                self.clear_pending_count();
                self.mark_all_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_ctrl_letter(&key, 'r') => {
                self.clear_pending_count();
                self.invert_marks_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char(' ') => {
                self.clear_pending_count();
                self.toggle_mark_selected_in_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('/') => {
                self.clear_pending_count();
                self.open_list_find_input();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char(',') => {
                self.clear_pending_count();
                self.open_sort_picker();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'w') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_window_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 'f') => {
                self.clear_pending_count();
                self.open_filter_input(FilterMode::Normal);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'F') => {
                self.clear_pending_count();
                self.open_filter_input(FilterMode::Fuzzy);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 's') => {
                self.clear_pending_count();
                self.open_global_search()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'S') => {
                self.clear_pending_count();
                self.open_content_search()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            KeyCode::Char('.') => {
                self.clear_pending_count();
                self.toggle_hidden_files()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'a') => {
                self.clear_pending_count();
                self.start_create_entry();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'x') => {
                self.clear_pending_count();
                self.cut_selected();
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'X') => {
                self.clear_pending_count();
                self.clear_clipboard(ClipboardOperation::Cut);
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'p') => {
                self.clear_pending_count();
                self.paste_into_focused_pane()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'P') => {
                self.clear_pending_count();
                self.paste_into_focused_pane_with_overwrite()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'u') => {
                self.clear_pending_count();
                self.undo_latest_file_operation()?;
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'n') => {
                if self
                    .panes
                    .get(&self.focused_pane)
                    .is_some_and(|pane| pane.has_list_find())
                {
                    let count = self.take_count_or_one();
                    self.status = self.jump_list_find_match(true, count)?;
                } else {
                    self.clear_pending_count();
                }
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_shifted_letter(&key, 'N') => {
                if self
                    .panes
                    .get(&self.focused_pane)
                    .is_some_and(|pane| pane.has_list_find())
                {
                    let count = self.take_count_or_one();
                    self.status = self.jump_list_find_match(false, count)?;
                } else {
                    self.clear_pending_count();
                }
                self.pending_g = false;
                self.pending_y = false;
                true
            }
            _ if key_matches_plain_letter(&key, 'y') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_bookmark = None;
                self.pending_y = false;
                self.copy_selected();
                true
            }
            _ if key_matches_shifted_letter(&key, 'Y') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_bookmark = None;
                self.pending_y = false;
                self.clear_clipboard(ClipboardOperation::Copy);
                true
            }
            _ if key_matches_plain_letter(&key, 'b') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_bookmark_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 'm') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_linemode_picker();
                true
            }
            _ if key_matches_plain_letter(&key, 't') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_theme_command_picker();
                true
            }
            _ if key_matches_shifted_letter(&key, 'T') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_task_panel();
                true
            }
            _ if key_matches_shifted_letter(&key, 'C') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.compress_selected_entries()?;
                true
            }
            _ if key_matches_shifted_letter(&key, 'E') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.extract_selected_archives()?;
                true
            }
            _ if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::ALT) => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.open_diff_matrix(None)?;
                true
            }
            KeyCode::Char('\'') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.pending_y = false;
                self.pending_bookmark = Some(BookmarkPrompt::Jump);
                self.status = String::from("bookmark: press a key to jump");
                true
            }
            KeyCode::Esc => {
                self.reset_pending_motion_state();
                self.pending_bookmark = None;
                self.handle_escape_in_normal_mode();
                true
            }
            _ => {
                self.reset_pending_motion_state();
                self.pending_bookmark = None;
                true
            }
        };

        Ok(should_continue)
    }

    /// 處理 preview mode 的鍵盤輸入，讓使用者可以專心在預覽區捲動內容。
    pub(crate) fn handle_preview_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }
        if key.code == KeyCode::Tab {
            if self.clear_preview_search_if_active() {
                self.clear_pending_count();
                self.pending_g = false;
                return Ok(true);
            }
            self.current_pane_mut()?.set_preview_active(false);
            self.reset_pending_motion_state();
            self.status = String::from("normal mode");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'N') {
            let count = self.take_count_or_one();
            self.pending_g = false;
            self.status = self.jump_preview_match(false, count)?;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'J') {
            let step = self.take_large_move_step();
            self.current_pane_mut()?.scroll_preview_down(step);
            self.pending_g = false;
            self.status = format!("preview: fast down {step}");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'K') {
            let step = self.take_large_move_step();
            self.current_pane_mut()?.scroll_preview_up(step);
            self.pending_g = false;
            self.status = format!("preview: fast up {step}");
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            if let Some(count) = self.take_pending_count() {
                self.current_pane_mut()?
                    .scroll_preview_down(count.saturating_sub(1));
                self.status = format!("preview: moved {count}");
            } else {
                self.current_pane_mut()?.scroll_preview_bottom();
                self.status = String::from("preview: bottom");
            }
            self.pending_g = false;
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc => {
                if self.clear_preview_search_if_active() {
                    self.clear_pending_count();
                    self.pending_g = false;
                    return Ok(true);
                }
                self.current_pane_mut()?.set_preview_active(false);
                self.reset_pending_motion_state();
                self.status = String::from("normal mode");
            }
            KeyCode::Char('/') => {
                self.clear_pending_count();
                self.open_preview_search_input();
                self.pending_g = false;
            }
            _ if key_matches_plain_letter(&key, 'n') => {
                let count = self.take_count_or_one();
                self.pending_g = false;
                self.status = self.jump_preview_match(true, count)?;
            }
            _ if key_matches_plain_letter(&key, 'p') => {
                let count = self.take_count_or_one();
                self.pending_g = false;
                self.status = self.jump_preview_match(false, count)?;
            }
            KeyCode::Down => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.scroll_preview_down(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.scroll_preview_down(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            KeyCode::Up => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.scroll_preview_up(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.scroll_preview_up(count);
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                let pending_line = self.pending_count;
                if self.pending_g {
                    if let Some(count) = self.take_pending_count() {
                        self.current_pane_mut()?
                            .scroll_preview_down(count.saturating_sub(1));
                        self.status = format!("preview: moved {count}");
                    } else {
                        self.current_pane_mut()?.scroll_preview_top();
                        self.status = String::from("preview: top");
                    }
                    self.pending_g = false;
                } else {
                    self.pending_g = true;
                    self.status = if let Some(count) = pending_line {
                        format!("preview: pending {count}g")
                    } else {
                        String::from("preview: pending g")
                    };
                }
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                self.clear_pending_count();
                self.current_pane_mut()?.page_preview_down();
                self.pending_g = false;
                self.status = String::from("preview: half page down");
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                self.clear_pending_count();
                self.current_pane_mut()?.page_preview_up();
                self.pending_g = false;
                self.status = String::from("preview: half page up");
            }
            _ if key_matches_ctrl_letter(&key, 'f') => {
                self.clear_pending_count();
                self.current_pane_mut()?.full_page_preview_down();
                self.pending_g = false;
                self.status = String::from("preview: page down");
            }
            _ if key_matches_ctrl_letter(&key, 'b') => {
                self.clear_pending_count();
                self.current_pane_mut()?.full_page_preview_up();
                self.pending_g = false;
                self.status = String::from("preview: page up");
            }
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = String::from("preview mode");
            }
        }

        Ok(true)
    }

    /// 處理 visual selection 模式下的鍵盤輸入。
    pub(crate) fn handle_visual_selection_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }
        if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V') {
            self.clear_pending_count();
            self.commit_visual_selection()?;
            return Ok(true);
        }
        if key_matches_shifted_letter(&key, 'G') {
            if let Some(count) = self.take_pending_count() {
                self.current_pane_mut()?
                    .move_to_visible_index(count.saturating_sub(1));
            } else {
                self.current_pane_mut()?.move_bottom();
            }
            self.sync_visual_selection_cursor();
            self.pending_g = false;
            self.status = self.visual_status_label();
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc => {
                self.clear_pending_count();
                self.commit_visual_selection()?;
            }
            KeyCode::Down => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_down_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_down_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            KeyCode::Up => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_up_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                self.current_pane_mut()?.move_up_by(count);
                self.sync_visual_selection_cursor();
                self.status = self.visual_status_label();
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_down_by(step);
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                let step = self.take_large_move_step();
                self.current_pane_mut()?.move_up_by(step);
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                self.clear_pending_count();
                self.current_pane_mut()?.page_down();
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                self.clear_pending_count();
                self.current_pane_mut()?.page_up();
                self.sync_visual_selection_cursor();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                let pending_line = self.pending_count;
                if self.pending_g {
                    if let Some(count) = self.take_pending_count() {
                        self.current_pane_mut()?
                            .move_to_visible_index(count.saturating_sub(1));
                    } else {
                        self.current_pane_mut()?.move_top();
                    }
                    self.sync_visual_selection_cursor();
                    self.pending_g = false;
                    self.status = self.visual_status_label();
                } else {
                    self.pending_g = true;
                    self.status = if let Some(count) = pending_line {
                        format!("visual: pending {count}g")
                    } else {
                        String::from("visual: pending g")
                    };
                }
            }
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = self.visual_status_label();
            }
        }

        Ok(true)
    }

    /// 處理 preview search 輸入框中的鍵盤輸入，並在每次輸入後立即更新命中位置。
    pub(crate) fn handle_preview_search_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.preview_search.take() else {
            return Ok(true);
        };

        let edit_result = self.edit_text_buffer(&mut search.buffer, &key);
        if matches!(edit_result, TextEditResult::Changed) {
            self.apply_preview_search_buffer(&search);
            self.status =
                preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
            self.preview_search = Some(search);
            return Ok(true);
        }
        if matches!(edit_result, TextEditResult::Consumed) {
            self.preview_search = Some(search);
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc if search.buffer.trim().is_empty() => {
                self.apply_preview_search_buffer(&search);
                self.preview_search = None;
                self.status = String::from("preview mode");
            }
            KeyCode::Esc | KeyCode::Enter => {
                search.editing = false;
                self.status = if search.buffer.is_empty() {
                    String::from("preview mode")
                } else {
                    format!(
                        "preview search locked: {} ({})",
                        search.buffer,
                        self.preview_match_count(search.pane_id)
                    )
                };
                self.preview_search = Some(search);
            }
            _ => {
                self.preview_search = Some(search);
            }
        }

        Ok(true)
    }

    /// 處理 global search 面板中的輸入、結果瀏覽與跳轉。
    pub(crate) fn handle_global_search_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.global_search.take() else {
            return Ok(true);
        };

        if search.editing {
            let edit_result = self.edit_text_buffer(&mut search.buffer, &key);
            if matches!(edit_result, TextEditResult::Changed) {
                search.searched = false;
                search.loading = false;
                search.selected = 0;
                search.results.clear();
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
                return Ok(true);
            }
            if matches!(edit_result, TextEditResult::Consumed) {
                self.global_search = Some(search);
                return Ok(true);
            }
            match key.code {
                KeyCode::Enter => {
                    if matches!(search.mode, SearchMode::Content) && search.buffer.trim().is_empty()
                    {
                        self.status = global_search_status(
                            search.mode,
                            &search.buffer,
                            search.results.len(),
                            search.editing,
                            search.searched,
                            search.loading,
                        );
                        self.global_search = Some(search);
                        return Ok(true);
                    }
                    self.start_global_search(&mut search)?;
                    search.editing = false;
                }
                KeyCode::Esc => {
                    self.cancel_global_search();
                }
                _ => {}
            }

            if !matches!(key.code, KeyCode::Esc) {
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            return Ok(true);
        }

        if search.filter.editing {
            let edit_result = self.edit_text_buffer(&mut search.filter.buffer, &key);
            if matches!(edit_result, TextEditResult::Changed) {
                search.selected = 0;
                let visible =
                    filtered_global_search_entries(&search.results, &search.filter.buffer);
                search.selected = search.selected.min(visible.len().saturating_sub(1));
                self.status = global_search_filter_status(&search.filter, visible.len());
                self.global_search = Some(search);
                return Ok(true);
            }
            if matches!(edit_result, TextEditResult::Consumed) {
                self.global_search = Some(search);
                return Ok(true);
            }
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    search.filter.editing = false;
                }
                _ => {}
            }
            let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
            search.selected = search.selected.min(visible.len().saturating_sub(1));
            self.status = global_search_filter_status(&search.filter, visible.len());
            self.global_search = Some(search);
            return Ok(true);
        }

        if self
            .panes
            .get(&search.pane_id)
            .is_some_and(PaneState::is_preview_active)
            && matches!(search.mode, SearchMode::Content)
        {
            match key.code {
                KeyCode::Tab => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                        pane.set_preview_active(false);
                    }
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    self.global_search = Some(search);
                    return Ok(true);
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                        pane.set_preview_active(false);
                    }
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    search.preview_scroll = None;
                    search.preview_current_match = None;
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                        pane.set_preview_active(false);
                    }
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.move_search_preview_match(&mut search, true);
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'p')
                    || key_matches_shifted_letter(&key, 'N') =>
                {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.move_search_preview_match(&mut search, false);
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    search.preview_scroll =
                        Some(search.preview_scroll.unwrap_or(0).saturating_add(1));
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    search.preview_scroll =
                        Some(search.preview_scroll.unwrap_or(0).saturating_sub(1));
                    self.status = self.search_preview_status_for(&search);
                    self.global_search = Some(search);
                    return Ok(true);
                }
                _ => {}
            }
        }

        if self.capture_pending_count_digit(&key) {
            self.global_search = Some(search);
            return Ok(true);
        }

        match key.code {
            KeyCode::Down => {
                let count = self.take_count_or_one();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + count).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'j') => {
                let count = self.take_count_or_one();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + count).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_shifted_letter(&key, 'J') => {
                let step = self.take_large_move_step();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + step).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            KeyCode::Up => {
                let count = self.take_count_or_one();
                search.selected = search.selected.saturating_sub(count);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_shifted_letter(&key, 'K') => {
                let step = self.take_large_move_step();
                search.selected = search.selected.saturating_sub(step);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_ctrl_letter(&key, 'd') => {
                let step = self.take_panel_page_step();
                let visible_len = global_search_visible_len(&search);
                search.selected = (search.selected + step).min(visible_len.saturating_sub(1));
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_ctrl_letter(&key, 'u') => {
                let step = self.take_panel_page_step();
                search.selected = search.selected.saturating_sub(step);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'k') => {
                let count = self.take_count_or_one();
                search.selected = search.selected.saturating_sub(count);
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'g') => {
                let pending_line = self.pending_count;
                if self.pending_g {
                    if let Some(count) = self.take_pending_count() {
                        search.selected = count
                            .saturating_sub(1)
                            .min(global_search_visible_len(&search).saturating_sub(1));
                    } else {
                        search.selected = 0;
                    }
                    search.preview_scroll = None;
                    search.preview_current_match = None;
                    self.pending_g = false;
                } else {
                    self.pending_g = true;
                }
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                if self.pending_g {
                    self.status = if let Some(count) = pending_line {
                        format!("{} (normal): pending {count}g", search.mode.status_label())
                    } else {
                        format!("{} (normal): pending g", search.mode.status_label())
                    };
                }
                self.global_search = Some(search);
            }
            _ if key_matches_shifted_letter(&key, 'G') => {
                if let Some(count) = self.take_pending_count() {
                    if global_search_visible_len(&search) > 0 {
                        search.selected = count
                            .saturating_sub(1)
                            .min(global_search_visible_len(&search).saturating_sub(1));
                    }
                } else if global_search_visible_len(&search) > 0 {
                    search.selected = global_search_visible_len(&search) - 1;
                }
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.pending_g = false;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'i') || key_matches_plain_letter(&key, 's') => {
                self.clear_pending_count();
                search.editing = true;
                self.text_input_mode = RenameMode::Insert;
                self.text_input_cursor = search.buffer.chars().count();
                search.preview_scroll = None;
                search.preview_current_match = None;
                self.pending_g = false;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    true,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'f') => {
                self.clear_pending_count();
                self.pending_g = false;
                if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                    pane.set_preview_active(false);
                }
                search.filter.editing = true;
                self.text_input_mode = RenameMode::Insert;
                self.text_input_cursor = search.filter.buffer.chars().count();
                search.selected = 0;
                self.status =
                    global_search_filter_status(&search.filter, global_search_visible_len(&search));
                self.global_search = Some(search);
            }
            KeyCode::Enter => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_global_search_result(search)?;
            }
            KeyCode::Right => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_global_search_result(search)?;
            }
            KeyCode::Tab if matches!(search.mode, SearchMode::Content) => {
                self.clear_pending_count();
                self.pending_g = false;
                if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                    pane.set_preview_active(true);
                }
                self.status = self.search_preview_status_for(&search);
                self.global_search = Some(search);
            }
            _ if key_matches_plain_letter(&key, 'l') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.open_global_search_result(search)?;
            }
            KeyCode::Esc => {
                self.clear_pending_count();
                self.pending_g = false;
                if search.filter.buffer.is_empty() {
                    self.cancel_global_search();
                } else {
                    search.filter = PanelSearchState::default();
                    search.selected = 0;
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        false,
                        search.searched,
                        search.loading,
                    );
                    self.global_search = Some(search);
                }
            }
            _ if key_matches_plain_letter(&key, 'h') => {
                self.clear_pending_count();
                self.pending_g = false;
                self.cancel_global_search();
            }
            _ => {
                self.clear_pending_count();
                self.pending_g = false;
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    false,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
        }

        Ok(true)
    }

    /// 處理列表內 find-next 輸入框的鍵盤輸入，並在每次輸入後立即更新高亮結果。
    pub(crate) fn handle_list_find_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut search) = self.list_find.take() else {
            return Ok(true);
        };

        let edit_result = self.edit_text_buffer(&mut search.buffer, &key);
        if matches!(edit_result, TextEditResult::Changed) {
            self.apply_list_find_buffer(&search);
            self.status =
                list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
            self.list_find = Some(search);
            return Ok(true);
        }
        if matches!(edit_result, TextEditResult::Consumed) {
            self.list_find = Some(search);
            return Ok(true);
        }

        match key.code {
            KeyCode::Enter => {
                self.apply_list_find_buffer(&search);
                self.status = list_find_locked_status(
                    &search.buffer,
                    self.list_find_match_count(search.pane_id),
                );
            }
            KeyCode::Esc => {
                if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                    pane.clear_list_find();
                }
                self.status = String::from("normal mode");
            }
            _ => {
                self.list_find = Some(search);
            }
        }

        Ok(true)
    }

    /// 處理 filter 輸入框中的鍵盤輸入，並在每次輸入後立即更新列表。
    pub(crate) fn handle_filter_input_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut filter) = self.filter.take() else {
            return Ok(true);
        };

        if key.code == KeyCode::Tab
            || key_matches_ctrl_letter(&key, 'f')
            || key_matches_ctrl_letter(&key, 's')
        {
            filter.mode = match filter.mode {
                FilterMode::Normal => FilterMode::Fuzzy,
                FilterMode::Fuzzy => FilterMode::Normal,
            };
            self.apply_filter_buffer(&filter);
            self.status = format_filter_status(&filter);
            self.filter = Some(filter);
            return Ok(true);
        }

        let edit_result = self.edit_text_buffer(&mut filter.buffer, &key);
        if matches!(edit_result, TextEditResult::Changed) {
            self.apply_filter_buffer(&filter);
            self.status = format_filter_status(&filter);
            self.filter = Some(filter);
            return Ok(true);
        }
        if matches!(edit_result, TextEditResult::Consumed) {
            self.filter = Some(filter);
            return Ok(true);
        }

        match key.code {
            KeyCode::Esc if filter.buffer.trim().is_empty() => {
                if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
                    pane.clear_filter();
                }
                self.status = String::from("normal mode");
            }
            KeyCode::Esc | KeyCode::Enter => {
                filter.editing = false;
                self.status = format_filter_status(&filter);
                self.filter = Some(filter);
            }
            _ => {
                self.filter = Some(filter);
            }
        }

        Ok(true)
    }

    /// 處理目前 pending action 所代表的暫時面板、選單或確認視窗。
    ///
    /// 參數：`key: KeyEvent`，要交給最上層暫時 UI 的按鍵。
    /// 回傳：`Result<bool>`，暫時 UI 一律消耗事件並回傳 `true`；檔案操作失敗則回傳
    /// error。函數開頭先 `take()` action，只有需要保留 UI 的分支才放回去，因此某
    /// 分支若未重設 `pending_action`，語意就是操作完成並關閉面板。
    pub(crate) fn handle_pending_action_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(mut action) = self.pending_action.take() else {
            return Ok(true);
        };

        // 所有具搜尋框的列表先共用文字編輯器。Normal 模式的 h/l 不可落入下面的
        // panel 導航，否則使用者只想移動輸入游標時會意外關閉或執行選項。
        let panel_edit_result = match &mut action {
            PendingAction::TrashPanel {
                selected, search, ..
            }
            | PendingAction::HelpPanel {
                selected, search, ..
            }
            | PendingAction::TaskPanel {
                selected, search, ..
            }
            | PendingAction::BookmarkList {
                selected, search, ..
            }
            | PendingAction::ZoxideList {
                selected, search, ..
            } if search.editing => {
                let result = self.edit_text_buffer(&mut search.buffer, &key);
                if matches!(result, TextEditResult::Changed) {
                    *selected = 0;
                }
                Some(result)
            }
            _ => None,
        };
        if panel_edit_result.is_some_and(|result| {
            matches!(result, TextEditResult::Changed | TextEditResult::Consumed)
        }) {
            self.status = self.status_for_pending_action(&action)?;
            self.pending_action = Some(action);
            return Ok(true);
        }

        match action {
            PendingAction::ToolPanel {
                pane_id,
                mut selected,
            } => {
                let tools = external_tool_statuses();
                let len = tools.len();
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => {
                        self.status = String::from("dependency panel closed");
                    }
                    _ if key_matches_plain_letter(&key, 'j') || key.code == KeyCode::Down => {
                        selected = if len == 0 {
                            0
                        } else {
                            (selected + 1).min(len - 1)
                        };
                        self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
                    }
                    _ if key_matches_plain_letter(&key, 'k') || key.code == KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::ToolPanel { pane_id, selected });
                    }
                }
            }
            PendingAction::ConfirmDelete {
                pane_id,
                target_name,
                permanent,
            } => match key.code {
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.status = if permanent {
                        format!("delete cancelled: {target_name}")
                    } else {
                        format!("trash cancelled: {target_name}")
                    };
                }
                _ if key_matches_letter_any_case(&key, 'y') => {
                    self.confirm_delete(pane_id, &target_name, permanent)?;
                }
                KeyCode::Esc => {
                    self.status = if permanent {
                        format!("delete cancelled: {target_name}")
                    } else {
                        format!("trash cancelled: {target_name}")
                    };
                }
                _ if key_matches_letter_any_case(&key, 'n') => {
                    self.status = if permanent {
                        format!("delete cancelled: {target_name}")
                    } else {
                        format!("trash cancelled: {target_name}")
                    };
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmDelete {
                        pane_id,
                        target_name: target_name.clone(),
                        permanent,
                    });
                    self.status = if permanent {
                        format!("confirm delete {target_name}: y/n")
                    } else {
                        format!("confirm trash {target_name}: y/n")
                    };
                }
            },
            PendingAction::ConfirmPasteOverwrite {
                pane_id,
                target_name,
                entry_count,
                operation,
            } => match key.code {
                _ if key_matches_letter_any_case(&key, 'y') || key.code == KeyCode::Enter => {
                    self.confirm_paste_overwrite(pane_id, target_name, entry_count, operation)?;
                }
                KeyCode::Esc => {
                    self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
                }
                _ if key_matches_letter_any_case(&key, 'n') => {
                    self.status = paste_overwrite_cancelled_status(&target_name, entry_count);
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
                        pane_id,
                        target_name: target_name.clone(),
                        entry_count,
                        operation,
                    });
                    self.status = paste_overwrite_confirm_status(&target_name, entry_count);
                }
            },
            PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                marked_ids,
                visual_anchor,
            } => match key.code {
                _ if key_matches_plain_letter(&key, 'd')
                    && matches!(&action, TrashConfirmAction::DeleteFromPanel { .. }) =>
                {
                    self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                        &action,
                        marked_ids,
                        visual_anchor,
                    ));
                    self.status =
                        trash_confirm_cancelled_status(&action, &target_name, entry_count);
                }
                _ if key_matches_letter_any_case(&key, 'y') || key.code == KeyCode::Enter => {
                    self.confirm_trash_action(action, target_name, entry_count)?;
                }
                KeyCode::Esc => {
                    self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                        &action,
                        marked_ids,
                        visual_anchor,
                    ));
                    self.status =
                        trash_confirm_cancelled_status(&action, &target_name, entry_count);
                }
                _ if key_matches_letter_any_case(&key, 'n') => {
                    self.pending_action = Some(trash_panel_pending_action_from_confirm_action(
                        &action,
                        marked_ids,
                        visual_anchor,
                    ));
                    self.status =
                        trash_confirm_cancelled_status(&action, &target_name, entry_count);
                }
                _ => {
                    self.pending_action = Some(PendingAction::ConfirmTrashAction {
                        action: action.clone(),
                        target_name: target_name.clone(),
                        entry_count,
                        marked_ids,
                        visual_anchor,
                    });
                    self.status = trash_confirm_status(&action, &target_name, entry_count);
                }
            },
            PendingAction::GoPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'g') => {
                    if let Some(count) = self.take_pending_count() {
                        self.current_pane_mut()?
                            .move_to_visible_index(count.saturating_sub(1));
                        self.status = format!("jumped to item {count}");
                    } else {
                        self.current_pane_mut()?.move_top();
                        self.status = String::from("jumped to top");
                    }
                    self.pending_g = false;
                    self.pending_y = false;
                }
                _ if key_matches_plain_letter(&key, 't') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.open_prefilled_command("goto ");
                }
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.go_to_special_directory(GoSpecialDirectory::Documents)?;
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.go_to_special_directory(GoSpecialDirectory::Desktop)?;
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::GoPicker { pane_id });
                    self.status = String::from("go: choose g/t/d/k from the panel");
                }
            },
            PendingAction::ThemeCommandPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 't') => {
                    self.focused_pane = pane_id;
                    self.open_trash_panel()?;
                }
                _ if key_matches_plain_letter(&key, 'u') => {
                    self.focused_pane = pane_id;
                    self.restore_latest_from_trash()?;
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.open_theme_picker();
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.cycle_theme();
                }
                KeyCode::Esc => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ThemeCommandPicker { pane_id });
                    self.status = String::from("theme/trash: choose l/n/t/u from the panel");
                }
            },
            PendingAction::SortPicker { pane_id } => match key.code {
                KeyCode::Char(',') => {
                    self.status = String::from("sort cancelled");
                }
                _ if key_matches_shifted_letter(&key, 'M') => {
                    self.apply_sort_mode(pane_id, SortMode::Modified { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'm') => {
                    self.apply_sort_mode(pane_id, SortMode::Modified { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'B') => {
                    self.apply_sort_mode(pane_id, SortMode::Created { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'b') => {
                    self.apply_sort_mode(pane_id, SortMode::Created { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'A') => {
                    self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'a') => {
                    self.apply_sort_mode(pane_id, SortMode::Alphabetical { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'N') => {
                    self.apply_sort_mode(pane_id, SortMode::Natural { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.apply_sort_mode(pane_id, SortMode::Natural { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'E') => {
                    self.apply_sort_mode(pane_id, SortMode::Extension { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 'e') => {
                    self.apply_sort_mode(pane_id, SortMode::Extension { reverse: false })?
                }
                _ if key_matches_shifted_letter(&key, 'S') => {
                    self.apply_sort_mode(pane_id, SortMode::Size { reverse: true })?
                }
                _ if key_matches_plain_letter(&key, 's') => {
                    self.apply_sort_mode(pane_id, SortMode::Size { reverse: false })?
                }
                _ if key_matches_plain_letter(&key, 'r') => {
                    self.apply_sort_mode(pane_id, SortMode::Random)?
                }
                KeyCode::Esc => {
                    self.status = String::from("sort cancelled");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.status = String::from("sort cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::SortPicker { pane_id });
                    self.status = String::from("sort: choose a key from the panel");
                }
            },
            PendingAction::WindowPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'w') => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Vertical, SplitPlacement::Before)?;
                }
                _ if key_matches_plain_letter(&key, 'j') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Horizontal, SplitPlacement::After)?;
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Horizontal, SplitPlacement::Before)?;
                }
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.split_current_at(SplitDirection::Vertical, SplitPlacement::After)?;
                }
                _ if key_matches_plain_letter(&key, 'c') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    if self.focused_pane == pane_id {
                        self.close_current_pane();
                    } else {
                        self.status = String::from("panel focus changed");
                    }
                }
                _ if key_matches_plain_letter(&key, 'o') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    if self.focused_pane == pane_id {
                        self.only_current_pane();
                    } else {
                        self.status = String::from("panel focus changed");
                    }
                }
                _ if key_matches_plain_letter(&key, 't') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    if self.focused_pane == pane_id {
                        self.open_terminal_in_active_panel()?;
                    } else {
                        self.status = String::from("panel focus changed");
                    }
                }
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.open_diff_matrix(None)?;
                }
                _ if key_matches_shifted_letter(&key, 'D') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_y = false;
                    self.open_prefilled_command("diff ");
                }
                KeyCode::Esc => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') => {
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::WindowPicker { pane_id });
                    self.status = String::from("panel: choose h/j/k/l/c/o/t/d from the panel");
                }
            },
            PendingAction::LineModePicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'm') => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 's') => {
                    self.apply_line_mode(pane_id, LineMode::Size)?;
                }
                _ if key_matches_plain_letter(&key, 'p') => {
                    self.apply_line_mode(pane_id, LineMode::Permissions)?;
                }
                _ if key_matches_plain_letter(&key, 'b') => {
                    self.apply_line_mode(pane_id, LineMode::Btime)?;
                }
                _ if key_matches_plain_letter(&key, 't') => {
                    self.apply_line_mode(pane_id, LineMode::Mtime)?;
                }
                _ if key_matches_plain_letter(&key, 'n') => {
                    self.apply_line_mode(pane_id, LineMode::None)?;
                }
                KeyCode::Esc => {
                    self.status = String::from("normal mode");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.status = String::from("normal mode");
                }
                _ => {
                    self.pending_action = Some(PendingAction::LineModePicker { pane_id });
                    self.status = String::from("linemode: choose a key from the panel");
                }
            },
            PendingAction::ThemePicker {
                mut selected,
                original,
            } => match key.code {
                KeyCode::Down => {
                    selected = (selected + 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_plain_letter(&key, 'j') => {
                    selected = (selected + 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                KeyCode::Up => {
                    selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_plain_letter(&key, 'k') => {
                    selected = (selected + ThemePreset::ALL.len() - 1) % ThemePreset::ALL.len();
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_shifted_letter(&key, 'J') => {
                    selected = (selected + self.take_large_move_step())
                        .min(ThemePreset::ALL.len().saturating_sub(1));
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_shifted_letter(&key, 'K') => {
                    selected = selected.saturating_sub(self.take_large_move_step());
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_ctrl_letter(&key, 'd') => {
                    selected = (selected + self.take_panel_page_step())
                        .min(ThemePreset::ALL.len().saturating_sub(1));
                    self.preview_theme_picker_selection(selected, original);
                }
                _ if key_matches_ctrl_letter(&key, 'u') => {
                    selected = selected.saturating_sub(self.take_panel_page_step());
                    self.preview_theme_picker_selection(selected, original);
                }
                KeyCode::Enter => self.apply_theme(ThemePreset::ALL[selected]),
                _ if key_matches_plain_letter(&key, 'l') => {
                    self.apply_theme(ThemePreset::ALL[selected])
                }
                KeyCode::Esc => {
                    self.theme = original.into();
                    self.status = String::from("theme picker cancelled");
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.theme = original.into();
                    self.status = String::from("theme picker cancelled");
                }
                _ => {
                    self.pending_action = Some(PendingAction::ThemePicker { selected, original });
                    self.status = String::from("theme picker: use j/k preview, l apply, h cancel");
                }
            },
            PendingAction::TrashPanel {
                pane_id,
                mut selected,
                mut search,
                mut marked_ids,
                mut visual_anchor,
            } => {
                let entries = trash_panel_entries(&self.trash_store, &search.buffer)?;
                let len = entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
                    let status = trash_panel_status(
                        &search.buffer,
                        next_len,
                        selected,
                        search.editing,
                        marked_ids.len(),
                    );
                    self.pending_action = Some(PendingAction::TrashPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    self.status = status;
                } else {
                    if self.capture_pending_count_digit(&key) {
                        self.pending_action = Some(PendingAction::TrashPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'G') {
                        if let Some(count) = self.take_pending_count() {
                            if len > 0 {
                                selected = count.saturating_sub(1).min(len.saturating_sub(1));
                            }
                        } else if len > 0 {
                            selected = len - 1;
                        }
                        self.pending_g = false;
                        let search_buffer = search.buffer.clone();
                        let search_editing = search.editing;
                        let marked_count = marked_ids.len();
                        self.pending_action = Some(PendingAction::TrashPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        self.status = trash_panel_status(
                            &search_buffer,
                            len,
                            selected,
                            search_editing,
                            marked_count,
                        );
                        return Ok(true);
                    }
                    if key_matches_plain_letter(&key, 'v') || key_matches_shifted_letter(&key, 'V')
                    {
                        self.pending_g = false;
                        if let Some(anchor) = visual_anchor.take() {
                            let added = self.commit_trash_visual_selection(
                                &entries,
                                &mut marked_ids,
                                anchor,
                                selected,
                            );
                            self.status = if added == 0 {
                                format!("trash: kept {} marked items", marked_ids.len())
                            } else {
                                format!("trash: marked {} items", marked_ids.len())
                            };
                        } else if len > 0 {
                            visual_anchor = Some(selected);
                            self.status = self.trash_visual_status_label(
                                selected,
                                selected,
                                marked_ids.len(),
                            );
                        }
                        self.pending_action = Some(PendingAction::TrashPanel {
                            pane_id,
                            selected,
                            search,
                            marked_ids,
                            visual_anchor,
                        });
                        return Ok(true);
                    }
                    if key_matches_plain_letter(&key, 'u') {
                        self.pending_g = false;
                        self.start_trash_panel_restore_confirmation(
                            pane_id,
                            &entries,
                            selected,
                            search,
                            &marked_ids,
                            visual_anchor,
                        )?;
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'U') {
                        self.pending_g = false;
                        self.start_trash_panel_restore_all_confirmation(
                            pane_id,
                            &entries,
                            selected,
                            search,
                            &marked_ids,
                            visual_anchor,
                        )?;
                        return Ok(true);
                    }
                    if key_matches_plain_letter(&key, 'd') {
                        self.pending_g = false;
                        self.start_trash_panel_delete_confirmation(
                            pane_id,
                            &entries,
                            selected,
                            search,
                            &marked_ids,
                            visual_anchor,
                        )?;
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'D') {
                        self.pending_g = false;
                        self.start_trash_panel_delete_all_confirmation(
                            pane_id,
                            &entries,
                            selected,
                            search,
                            &marked_ids,
                            visual_anchor,
                        )?;
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Down => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        KeyCode::Up => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'k') => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'g') => {
                            if self.pending_g {
                                if let Some(count) = self.take_pending_count() {
                                    selected = count.saturating_sub(1).min(len.saturating_sub(1));
                                } else {
                                    selected = 0;
                                }
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        _ if key_matches_plain_letter(&key, 'f') => {
                            self.clear_pending_count();
                            search.editing = true;
                            self.text_input_mode = RenameMode::Insert;
                            self.text_input_cursor = search.buffer.chars().count();
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'd') => {
                            if len > 0 {
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'K') => {
                            selected = selected.saturating_sub(self.take_large_move_step());
                            self.pending_g = false;
                        }
                        KeyCode::Enter => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.start_trash_panel_restore_confirmation(
                                pane_id,
                                &entries,
                                selected,
                                search,
                                &marked_ids,
                                visual_anchor,
                            )?;
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'l') => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.start_trash_panel_restore_confirmation(
                                pane_id,
                                &entries,
                                selected,
                                search,
                                &marked_ids,
                                visual_anchor,
                            )?;
                            return Ok(true);
                        }
                        KeyCode::Esc => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            if let Some(anchor) = visual_anchor.take() {
                                let added = self.commit_trash_visual_selection(
                                    &entries,
                                    &mut marked_ids,
                                    anchor,
                                    selected,
                                );
                                self.status = if added == 0 {
                                    format!("trash: kept {} marked items", marked_ids.len())
                                } else {
                                    format!("trash: marked {} items", marked_ids.len())
                                };
                            } else if !marked_ids.is_empty() {
                                let cleared = marked_ids.len();
                                marked_ids.clear();
                                self.status = format!("trash: cleared {cleared} marks");
                            } else {
                                self.status = String::from("normal mode");
                                return Ok(true);
                            }
                        }
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ => {
                            self.clear_pending_count();
                            self.pending_g = false;
                        }
                    }
                    let status = if let Some(anchor) = visual_anchor {
                        self.trash_visual_status_label(selected, anchor, marked_ids.len())
                    } else {
                        trash_panel_status(
                            &search.buffer,
                            len,
                            selected,
                            search.editing,
                            marked_ids.len(),
                        )
                    };
                    self.pending_action = Some(PendingAction::TrashPanel {
                        pane_id,
                        selected,
                        search,
                        marked_ids,
                        visual_anchor,
                    });
                    self.status = status;
                }
            }
            PendingAction::HelpPanel {
                pane_id,
                mut selected,
                mut search,
            } => {
                let filtered_entries = help_entries(&search.buffer);
                let filtered_len = filtered_entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = help_entries(&search.buffer).len();
                    let status = help_panel_status(&search.buffer, next_len, search.editing);
                    self.pending_action = Some(PendingAction::HelpPanel {
                        pane_id,
                        selected,
                        search,
                    });
                    self.status = status;
                } else {
                    if self.capture_pending_count_digit(&key) {
                        self.pending_action = Some(PendingAction::HelpPanel {
                            pane_id,
                            selected,
                            search,
                        });
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'G') {
                        if let Some(count) = self.take_pending_count() {
                            if filtered_len > 0 {
                                selected =
                                    count.saturating_sub(1).min(filtered_len.saturating_sub(1));
                            }
                        } else if filtered_len > 0 {
                            selected = filtered_len - 1;
                        }
                        self.pending_g = false;
                        let status = help_panel_status(&search.buffer, filtered_len, false);
                        self.pending_action = Some(PendingAction::HelpPanel {
                            pane_id,
                            selected,
                            search,
                        });
                        self.status = status;
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Down => {
                            if filtered_len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(filtered_len.saturating_sub(1));
                            }
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if filtered_len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(filtered_len.saturating_sub(1));
                            }
                        }
                        KeyCode::Up => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                        }
                        _ if key_matches_plain_letter(&key, 'k') => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                        }
                        _ if key_matches_plain_letter(&key, 'g') => {
                            if self.pending_g {
                                if let Some(count) = self.take_pending_count() {
                                    selected =
                                        count.saturating_sub(1).min(filtered_len.saturating_sub(1));
                                } else {
                                    selected = 0;
                                }
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        _ if key_matches_plain_letter(&key, 'f') => {
                            self.clear_pending_count();
                            search.editing = true;
                            self.text_input_mode = RenameMode::Insert;
                            self.text_input_cursor = search.buffer.chars().count();
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'd') => {
                            if filtered_len > 0 {
                                selected = (selected + self.take_panel_page_step())
                                    .min(filtered_len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if filtered_len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(filtered_len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'K') => {
                            selected = selected.saturating_sub(self.take_large_move_step());
                            self.pending_g = false;
                        }
                        KeyCode::Enter => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.execute_help_entry(&filtered_entries, selected)?;
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'l') => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.execute_help_entry(&filtered_entries, selected)?;
                            return Ok(true);
                        }
                        KeyCode::Esc | KeyCode::F(1) => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.restore_help_return_state(false)?;
                            return Ok(true);
                        }
                        _ if key_matches_tilde(&key) => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.restore_help_return_state(false)?;
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.restore_help_return_state(false)?;
                            return Ok(true);
                        }
                        _ => {
                            self.clear_pending_count();
                            self.pending_g = false;
                        }
                    }
                    let next_count = help_entries(&search.buffer).len();
                    let status = help_panel_status(&search.buffer, next_count, false);
                    self.pending_action = Some(PendingAction::HelpPanel {
                        pane_id,
                        selected,
                        search,
                    });
                    self.status = status;
                }
            }
            PendingAction::TaskPanel {
                pane_id,
                mut selected,
                mut search,
            } => {
                let tasks = self.tasks_for_pane(pane_id);
                let filtered_tasks = filtered_task_entries(&tasks, &search.buffer);
                let len = filtered_tasks.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = filtered_task_entries(&tasks, &search.buffer).len();
                    let status =
                        task_panel_status(&search.buffer, next_len, selected, search.editing);
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected,
                        search,
                    });
                    self.status = status;
                } else {
                    if self.capture_pending_count_digit(&key) {
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                        });
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'G') {
                        if let Some(count) = self.take_pending_count() {
                            if len > 0 {
                                selected = count.saturating_sub(1).min(len.saturating_sub(1));
                            }
                        } else if len > 0 {
                            selected = len - 1;
                        }
                        self.pending_g = false;
                        let status = task_panel_status(&search.buffer, len, selected, false);
                        self.pending_action = Some(PendingAction::TaskPanel {
                            pane_id,
                            selected,
                            search,
                        });
                        self.status = status;
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Down => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        KeyCode::Up => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'k') => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'd') => {
                            if len > 0 {
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'K') => {
                            selected = selected.saturating_sub(self.take_large_move_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'f') => {
                            self.clear_pending_count();
                            search.editing = true;
                            self.text_input_mode = RenameMode::Insert;
                            self.text_input_cursor = search.buffer.chars().count();
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'x')
                            || key_matches_plain_letter(&key, 'c') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            if let Some(task) = filtered_tasks.get(selected) {
                                self.cancel_task_by_id(task.id);
                                let next_tasks = self.tasks_for_pane(pane_id);
                                let next_len =
                                    filtered_task_entries(&next_tasks, &search.buffer).len();
                                selected = selected.min(next_len.saturating_sub(1));
                                let status =
                                    task_panel_status(&search.buffer, next_len, selected, false);
                                self.pending_action = Some(PendingAction::TaskPanel {
                                    pane_id,
                                    selected,
                                    search,
                                });
                                if self.status.is_empty() {
                                    self.status = status;
                                }
                                return Ok(true);
                            }
                        }
                        _ if key_matches_shifted_letter(&key, 'X') => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            let cancelled = self.cancel_running_tasks_for_pane(pane_id);
                            self.status = if cancelled == 0 {
                                String::from("no cancellable running tasks")
                            } else {
                                format!("cancelled {cancelled} tasks")
                            };
                            self.pending_action = Some(PendingAction::TaskPanel {
                                pane_id,
                                selected,
                                search,
                            });
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'g') => {
                            if self.pending_g {
                                if let Some(count) = self.take_pending_count() {
                                    selected = count.saturating_sub(1).min(len.saturating_sub(1));
                                } else {
                                    selected = 0;
                                }
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Enter | KeyCode::Right => {
                            if let Some(task) = filtered_tasks.get(selected) {
                                self.status =
                                    format!("task {} [{}] {}", task.id, task.kind, task.detail);
                            } else {
                                self.status = String::from("tasks: empty");
                            }
                        }
                        _ if key_matches_plain_letter(&key, 'l') => {
                            if let Some(task) = filtered_tasks.get(selected) {
                                self.status =
                                    format!("task {} [{}] {}", task.id, task.kind, task.detail);
                            } else {
                                self.status = String::from("tasks: empty");
                            }
                        }
                        KeyCode::Esc => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 't') => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ => {
                            self.clear_pending_count();
                            self.pending_g = false;
                        }
                    }
                    let status = task_panel_status(&search.buffer, len, selected, false);
                    self.pending_action = Some(PendingAction::TaskPanel {
                        pane_id,
                        selected,
                        search,
                    });
                    if !matches!(key.code, KeyCode::Enter | KeyCode::Right)
                        && !key_matches_plain_letter(&key, 'l')
                    {
                        self.status = status;
                    }
                }
            }
            PendingAction::BookmarkPicker { pane_id } => match key.code {
                _ if key_matches_plain_letter(&key, 'b') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'a') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.add_bookmark_with_auto_key(pane_id)?;
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'g') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.open_bookmark_list_with_mode(pane_id, BookmarkListMode::Jump);
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'd') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.open_bookmark_list_with_mode(pane_id, BookmarkListMode::Delete);
                    return Ok(true);
                }
                _ if key_matches_shifted_letter(&key, 'D') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.delete_all_bookmarks()?;
                    return Ok(true);
                }
                KeyCode::Esc => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
                }
                _ if key_matches_plain_letter(&key, 'q') || key_matches_plain_letter(&key, 'h') => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.status = String::from("normal mode");
                    return Ok(true);
                }
                _ => {
                    self.clear_pending_count();
                    self.pending_g = false;
                    self.pending_action = Some(PendingAction::BookmarkPicker { pane_id });
                    self.status = String::from("bookmark: choose a/g/d/D from the panel");
                    return Ok(true);
                }
            },
            PendingAction::BookmarkList {
                pane_id,
                mut selected,
                mode,
                mut search,
            } => {
                let entries = self.bookmark_store.list();
                let filtered_entries = filtered_bookmark_entries(entries.clone(), &search.buffer);
                let len = filtered_entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len =
                        filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer).len();
                    let status = bookmark_list_status(
                        &search.buffer,
                        next_len,
                        selected,
                        mode,
                        search.editing,
                    );
                    self.pending_action = Some(PendingAction::BookmarkList {
                        pane_id,
                        selected,
                        mode,
                        search,
                    });
                    self.status = status;
                } else {
                    if self.capture_pending_count_digit(&key) {
                        self.pending_action = Some(PendingAction::BookmarkList {
                            pane_id,
                            selected,
                            mode,
                            search,
                        });
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'G') {
                        if let Some(count) = self.take_pending_count() {
                            if len > 0 {
                                selected = count.saturating_sub(1).min(len.saturating_sub(1));
                            }
                        } else if len > 0 {
                            selected = len - 1;
                        }
                        self.pending_g = false;
                        let status =
                            bookmark_list_status(&search.buffer, len, selected, mode, false);
                        self.pending_action = Some(PendingAction::BookmarkList {
                            pane_id,
                            selected,
                            mode,
                            search,
                        });
                        self.status = status;
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Char(bookmark_key)
                            if matches!(mode, BookmarkListMode::Delete)
                                && filtered_entries
                                    .iter()
                                    .any(|entry| entry.key == bookmark_key) =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.delete_bookmark(bookmark_key)?;
                            return Ok(true);
                        }
                        KeyCode::Down => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        KeyCode::Up => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'k') => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'd') => {
                            if len > 0 {
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'K') => {
                            selected = selected.saturating_sub(self.take_large_move_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'f') => {
                            self.clear_pending_count();
                            search.editing = true;
                            self.text_input_mode = RenameMode::Insert;
                            self.text_input_cursor = search.buffer.chars().count();
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'g') => {
                            if self.pending_g {
                                if let Some(count) = self.take_pending_count() {
                                    selected = count.saturating_sub(1).min(len.saturating_sub(1));
                                } else {
                                    selected = 0;
                                }
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Enter => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            match mode {
                                BookmarkListMode::Jump => {
                                    self.open_bookmark_from_list(
                                        pane_id,
                                        &filtered_entries,
                                        selected,
                                    )?;
                                }
                                BookmarkListMode::Delete => {
                                    self.delete_bookmark_from_list(&filtered_entries, selected)?;
                                }
                            }
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'l') => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            if matches!(mode, BookmarkListMode::Jump) {
                                self.open_bookmark_from_list(pane_id, &filtered_entries, selected)?;
                            } else {
                                let status = bookmark_list_status(
                                    &search.buffer,
                                    len,
                                    selected,
                                    mode,
                                    false,
                                );
                                self.pending_action = Some(PendingAction::BookmarkList {
                                    pane_id,
                                    selected,
                                    mode,
                                    search,
                                });
                                self.status = status;
                            }
                            return Ok(true);
                        }
                        KeyCode::Esc => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ => {
                            self.clear_pending_count();
                            self.pending_g = false;
                        }
                    }
                    let status = bookmark_list_status(&search.buffer, len, selected, mode, false);
                    self.pending_action = Some(PendingAction::BookmarkList {
                        pane_id,
                        selected,
                        mode,
                        search,
                    });
                    self.status = status;
                }
            }
            PendingAction::ZoxideList {
                pane_id,
                mut selected,
                entries,
                mut search,
            } => {
                let filtered_entries = filtered_zoxide_entries(&entries, &search.buffer);
                let len = filtered_entries.len();
                if search.editing {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            search.editing = false;
                        }
                        _ => {}
                    }
                    let next_len = filtered_zoxide_entries(&entries, &search.buffer).len();
                    let status =
                        zoxide_list_status(&search.buffer, next_len, selected, search.editing);
                    self.pending_action = Some(PendingAction::ZoxideList {
                        pane_id,
                        selected,
                        entries,
                        search,
                    });
                    self.status = status;
                } else {
                    if self.capture_pending_count_digit(&key) {
                        self.pending_action = Some(PendingAction::ZoxideList {
                            pane_id,
                            selected,
                            entries,
                            search,
                        });
                        return Ok(true);
                    }
                    if key_matches_shifted_letter(&key, 'G') {
                        if let Some(count) = self.take_pending_count() {
                            if len > 0 {
                                selected = count.saturating_sub(1).min(len.saturating_sub(1));
                            }
                        } else if len > 0 {
                            selected = len - 1;
                        }
                        self.pending_g = false;
                        let status = zoxide_list_status(&search.buffer, len, selected, false);
                        self.pending_action = Some(PendingAction::ZoxideList {
                            pane_id,
                            selected,
                            entries,
                            search,
                        });
                        self.status = status;
                        return Ok(true);
                    }
                    match key.code {
                        KeyCode::Down => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'j') => {
                            if len > 0 {
                                selected = (selected + self.take_count_or_one())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        KeyCode::Up => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'k') => {
                            selected = selected.saturating_sub(self.take_count_or_one());
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'd') => {
                            if len > 0 {
                                selected = (selected + self.take_panel_page_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_ctrl_letter(&key, 'u') => {
                            selected = selected.saturating_sub(self.take_panel_page_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'J') => {
                            if len > 0 {
                                selected = (selected + self.take_large_move_step())
                                    .min(len.saturating_sub(1));
                            }
                            self.pending_g = false;
                        }
                        _ if key_matches_shifted_letter(&key, 'K') => {
                            selected = selected.saturating_sub(self.take_large_move_step());
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'f') => {
                            self.clear_pending_count();
                            search.editing = true;
                            self.text_input_mode = RenameMode::Insert;
                            self.text_input_cursor = search.buffer.chars().count();
                            self.pending_g = false;
                        }
                        _ if key_matches_plain_letter(&key, 'g') => {
                            if self.pending_g {
                                if let Some(count) = self.take_pending_count() {
                                    selected = count.saturating_sub(1).min(len.saturating_sub(1));
                                } else {
                                    selected = 0;
                                }
                                self.pending_g = false;
                            } else {
                                self.pending_g = true;
                            }
                        }
                        KeyCode::Enter => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.open_zoxide_from_list(pane_id, &filtered_entries, selected)?;
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'l') => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.open_zoxide_from_list(pane_id, &filtered_entries, selected)?;
                            return Ok(true);
                        }
                        KeyCode::Esc => {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ if key_matches_plain_letter(&key, 'q')
                            || key_matches_plain_letter(&key, 'h') =>
                        {
                            self.clear_pending_count();
                            self.pending_g = false;
                            self.status = String::from("normal mode");
                            return Ok(true);
                        }
                        _ => {
                            self.clear_pending_count();
                            self.pending_g = false;
                        }
                    }
                    let status = zoxide_list_status(&search.buffer, len, selected, false);
                    self.pending_action = Some(PendingAction::ZoxideList {
                        pane_id,
                        selected,
                        entries,
                        search,
                    });
                    self.status = status;
                }
            }
            PendingAction::CopyPicker {
                pane_id,
                target,
                mut selected,
            } => {
                let options = copy_picker_options();
                if self.capture_pending_count_digit(&key) {
                    self.pending_action = Some(PendingAction::CopyPicker {
                        pane_id,
                        target: target.clone(),
                        selected,
                    });
                    self.status = format!("copy to clipboard: {}", target.display_name);
                    return Ok(true);
                }
                match key.code {
                    _ if key_matches_plain_letter(&key, 'c') => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_plain_letter(&key, 'u') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(target.clone(), CopyAction::FileUrl)?;
                    }
                    _ if key_matches_plain_letter(&key, 'd') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(
                            target.clone(),
                            CopyAction::DirectoryUrl,
                        )?;
                    }
                    _ if key_matches_plain_letter(&key, 'f') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(target.clone(), CopyAction::Filename)?;
                    }
                    _ if key_matches_plain_letter(&key, 'n') => {
                        self.clear_pending_count();
                        self.copy_target_to_system_clipboard(
                            target.clone(),
                            CopyAction::FilenameWithoutExtension,
                        )?;
                    }
                    KeyCode::Down => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'j') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'k') => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'd') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_panel_page_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'u') => {
                        selected = selected.saturating_sub(self.take_panel_page_step());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'J') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_large_move_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'K') => {
                        selected = selected.saturating_sub(self.take_large_move_step());
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                    KeyCode::Enter => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.copy_target_to_system_clipboard(target.clone(), option.action)?;
                        } else {
                            self.status = String::from("copy picker: no option selected");
                        }
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.copy_target_to_system_clipboard(target.clone(), option.action)?;
                        } else {
                            self.status = String::from("copy picker: no option selected");
                        }
                    }
                    KeyCode::Esc => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_plain_letter(&key, 'q')
                        || key_matches_plain_letter(&key, 'h') =>
                    {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ => {
                        self.clear_pending_count();
                        self.pending_action = Some(PendingAction::CopyPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                        });
                        self.status = format!("copy to clipboard: {}", target.display_name);
                    }
                }
            }
            PendingAction::OpenPicker {
                pane_id,
                target,
                mut selected,
                options,
            } => {
                if self.capture_pending_count_digit(&key) {
                    self.pending_action = Some(PendingAction::OpenPicker {
                        pane_id,
                        target: target.clone(),
                        selected,
                        options: options.clone(),
                    });
                    self.status = format!("open with: {}", target.display_name);
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_shifted_letter(&key, 'O') => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    KeyCode::Down => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'j') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_count_or_one())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_plain_letter(&key, 'k') => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'd') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_panel_page_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_ctrl_letter(&key, 'u') => {
                        selected = selected.saturating_sub(self.take_panel_page_step());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'J') => {
                        if !options.is_empty() {
                            selected = (selected + self.take_large_move_step())
                                .min(options.len().saturating_sub(1));
                        }
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    _ if key_matches_shifted_letter(&key, 'K') => {
                        selected = selected.saturating_sub(self.take_large_move_step());
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                    KeyCode::Enter => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.queue_open_picker_action(target.clone(), option.action.clone())?;
                        } else {
                            self.status = String::from("open with: no option selected");
                        }
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        self.clear_pending_count();
                        if let Some(option) = options.get(selected) {
                            self.queue_open_picker_action(target.clone(), option.action.clone())?;
                        } else {
                            self.status = String::from("open with: no option selected");
                        }
                    }
                    KeyCode::Esc => {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ if key_matches_plain_letter(&key, 'q')
                        || key_matches_plain_letter(&key, 'h') =>
                    {
                        self.clear_pending_count();
                        self.status = String::from("normal mode");
                    }
                    _ => {
                        self.clear_pending_count();
                        self.pending_action = Some(PendingAction::OpenPicker {
                            pane_id,
                            target: target.clone(),
                            selected,
                            options: options.clone(),
                        });
                        self.status = format!("open with: {}", target.display_name);
                    }
                }
            }
            PendingAction::Rename {
                pane_id,
                original_name,
                mut buffer,
                mut cursor,
                mut mode,
            } => match mode {
                RenameMode::Insert => match key.code {
                    KeyCode::Char(_) => {
                        if let Some(c) = typed_char_from_key(&key) {
                            insert_char(&mut buffer, &mut cursor, c);
                        }
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Backspace => {
                        backspace_char(&mut buffer, &mut cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Enter => {
                        self.confirm_rename(pane_id, &original_name, &buffer)?;
                    }
                    KeyCode::Esc => {
                        if buffer.trim().is_empty() {
                            self.status = format!("rename cancelled: {original_name}");
                        } else {
                            mode = RenameMode::Normal;
                            self.pending_action = Some(PendingAction::Rename {
                                pane_id,
                                original_name,
                                buffer,
                                cursor,
                                mode,
                            });
                            self.status = String::from("rename: normal");
                        }
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
                RenameMode::Normal if key_matches_shifted_letter(&key, 'A') => {
                    cursor = buffer.chars().count();
                    mode = RenameMode::Insert;
                    self.pending_action = Some(PendingAction::Rename {
                        pane_id,
                        original_name,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = String::from("rename: insert");
                }
                RenameMode::Normal => match key.code {
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'h') => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'w') => {
                        cursor = rename_next_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'b') => {
                        cursor = rename_previous_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'e') => {
                        cursor = rename_word_end(&buffer, cursor);
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'i') => {
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    _ if key_matches_plain_letter(&key, 'a') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = String::from("rename: insert");
                    }
                    KeyCode::Enter => {
                        self.confirm_rename(pane_id, &original_name, &buffer)?;
                    }
                    KeyCode::Esc => {
                        self.status = format!("rename cancelled: {original_name}");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::Rename {
                            pane_id,
                            original_name,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
            },
            PendingAction::CreateEntry {
                pane_id,
                mut buffer,
                mut cursor,
                mut mode,
            } => match mode {
                RenameMode::Insert => match key.code {
                    KeyCode::Char(_) => {
                        if let Some(c) = typed_char_from_key(&key) {
                            insert_char(&mut buffer, &mut cursor, c);
                        }
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Backspace => {
                        backspace_char(&mut buffer, &mut cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Enter => {
                        self.confirm_create_entry(pane_id, &buffer)?;
                    }
                    KeyCode::Esc => {
                        if buffer.trim().is_empty() {
                            self.status = String::from("create cancelled");
                        } else {
                            mode = RenameMode::Normal;
                            self.pending_action = Some(PendingAction::CreateEntry {
                                pane_id,
                                buffer,
                                cursor,
                                mode,
                            });
                            self.status = create_status_label("normal");
                        }
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
                RenameMode::Normal if key_matches_shifted_letter(&key, 'A') => {
                    cursor = buffer.chars().count();
                    mode = RenameMode::Insert;
                    self.pending_action = Some(PendingAction::CreateEntry {
                        pane_id,
                        buffer,
                        cursor,
                        mode,
                    });
                    self.status = create_status_label("insert");
                }
                RenameMode::Normal => match key.code {
                    KeyCode::Left => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'h') => {
                        cursor = cursor.saturating_sub(1);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Right => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('0') => {
                        cursor = 0;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    KeyCode::Char('$') => {
                        cursor = rename_line_end_cursor(&buffer);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'w') => {
                        cursor = rename_next_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'b') => {
                        cursor = rename_previous_word_start(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'e') => {
                        cursor = rename_word_end(&buffer, cursor);
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'i') => {
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    _ if key_matches_plain_letter(&key, 'a') => {
                        cursor = move_cursor_right(&buffer, cursor);
                        mode = RenameMode::Insert;
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                        self.status = create_status_label("insert");
                    }
                    KeyCode::Enter => {
                        self.confirm_create_entry(pane_id, &buffer)?;
                    }
                    KeyCode::Esc => {
                        self.status = String::from("create cancelled");
                    }
                    _ => {
                        self.pending_action = Some(PendingAction::CreateEntry {
                            pane_id,
                            buffer,
                            cursor,
                            mode,
                        });
                    }
                },
            },
            PendingAction::RegexRename {
                pane_id,
                pattern,
                replacement,
                mut selected,
                previews,
            } => {
                let len = previews.len();
                if self.capture_pending_count_digit(&key) {
                    self.pending_action = Some(PendingAction::RegexRename {
                        pane_id,
                        pattern,
                        replacement,
                        selected,
                        previews,
                    });
                    if let Some(action) = self.pending_action.as_ref() {
                        self.status = self.status_for_pending_action(action)?;
                    }
                    return Ok(true);
                }
                match key.code {
                    KeyCode::Down => {
                        if len > 0 {
                            selected =
                                (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                        }
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'j') => {
                        if len > 0 {
                            selected =
                                (selected + self.take_count_or_one()).min(len.saturating_sub(1));
                        }
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'k') => {
                        selected = selected.saturating_sub(self.take_count_or_one());
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_ctrl_letter(&key, 'd') => {
                        if len > 0 {
                            selected =
                                (selected + self.take_panel_page_step()).min(len.saturating_sub(1));
                        }
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_ctrl_letter(&key, 'u') => {
                        selected = selected.saturating_sub(self.take_panel_page_step());
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_shifted_letter(&key, 'J') => {
                        if len > 0 {
                            selected =
                                (selected + self.take_large_move_step()).min(len.saturating_sub(1));
                        }
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_shifted_letter(&key, 'K') => {
                        selected = selected.saturating_sub(self.take_large_move_step());
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_plain_letter(&key, 'g') => {
                        if self.pending_g {
                            if let Some(count) = self.take_pending_count() {
                                selected = count.saturating_sub(1).min(len.saturating_sub(1));
                            } else {
                                selected = 0;
                            }
                            self.pending_g = false;
                        } else {
                            self.pending_g = true;
                        }
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    _ if key_matches_shifted_letter(&key, 'G') => {
                        if let Some(count) = self.take_pending_count() {
                            if len > 0 {
                                selected = count.saturating_sub(1).min(len.saturating_sub(1));
                            }
                        } else if len > 0 {
                            selected = len - 1;
                        }
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                    KeyCode::Enter => {
                        self.clear_pending_count();
                        self.pending_g = false;
                        self.apply_regex_rename_preview(pane_id, &previews)?;
                    }
                    _ if key_matches_plain_letter(&key, 'l') => {
                        self.clear_pending_count();
                        self.pending_g = false;
                        self.apply_regex_rename_preview(pane_id, &previews)?;
                    }
                    KeyCode::Esc => {
                        self.clear_pending_count();
                        self.pending_g = false;
                        self.status = String::from("regex rename cancelled");
                    }
                    _ if key_matches_plain_letter(&key, 'q')
                        || key_matches_plain_letter(&key, 'h') =>
                    {
                        self.clear_pending_count();
                        self.pending_g = false;
                        self.status = String::from("regex rename cancelled");
                    }
                    _ => {
                        self.clear_pending_count();
                        self.pending_g = false;
                        self.pending_action = Some(PendingAction::RegexRename {
                            pane_id,
                            pattern,
                            replacement,
                            selected,
                            previews,
                        });
                    }
                }
                if let Some(action) = self.pending_action.as_ref() {
                    self.status = self.status_for_pending_action(action)?;
                }
            }
            PendingAction::DiffMatrix(mut state) => {
                if state.search_active {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => {
                            state.search_active = false;
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        KeyCode::Backspace => {
                            state.search_query.pop();
                            state.refresh_filtered_indices();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        KeyCode::Char(c) => {
                            state.search_query.push(c);
                            state.refresh_filtered_indices();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ => {
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                    }
                } else if state.loading {
                    match key.code {
                        KeyCode::Esc => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ if key_matches_plain_letter(&key, 'q') => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ => {
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ if key_matches_plain_letter(&key, 'q') => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            self.diff_job_rx = None;
                            self.status = String::from("diff matrix closed");
                        }
                        _ if key_matches_plain_letter(&key, 'j') || key.code == KeyCode::Down => {
                            state.move_down();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'k') || key.code == KeyCode::Up => {
                            state.move_up();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'g') || key.code == KeyCode::Home => {
                            state.move_to_top();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_letter_any_case(&key, 'G') || key.code == KeyCode::End => {
                            state.move_to_bottom();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'f') => {
                            state.cycle_filter_mode();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key.code == KeyCode::Char('/') => {
                            state.search_active = true;
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'i') => {
                            state.git_ignore = !state.git_ignore;
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            let cancelled = Arc::new(AtomicBool::new(false));
                            self.diff_job_cancelled = Some(cancelled.clone());
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.diff_job_rx = Some(rx);
                            spawn_background_diff(state.panel_roots.clone(), state.git_ignore, state.include_hidden, cancelled, tx);
                            state.loading = true;
                            state.rows.clear();
                            state.filtered_indices.clear();
                            self.status = format!(
                                "diff matrix: .gitignore rules {}",
                                if state.git_ignore { "enabled" } else { "disabled (scanning target/build dirs)" }
                            );
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key.code == KeyCode::Char('.') => {
                            state.include_hidden = !state.include_hidden;
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            let cancelled = Arc::new(AtomicBool::new(false));
                            self.diff_job_cancelled = Some(cancelled.clone());
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.diff_job_rx = Some(rx);
                            spawn_background_diff(state.panel_roots.clone(), state.git_ignore, state.include_hidden, cancelled, tx);
                            state.loading = true;
                            state.rows.clear();
                            state.filtered_indices.clear();
                            self.status = format!(
                                "diff matrix: hidden files {}",
                                if state.include_hidden { "included" } else { "excluded" }
                            );
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ if key_matches_plain_letter(&key, 'r') => {
                            if let Some(cancelled) = self.diff_job_cancelled.take() {
                                cancelled.store(true, Ordering::Relaxed);
                            }
                            let cancelled = Arc::new(AtomicBool::new(false));
                            self.diff_job_cancelled = Some(cancelled.clone());
                            let (tx, rx) = std::sync::mpsc::channel();
                            self.diff_job_rx = Some(rx);
                            spawn_background_diff(state.panel_roots.clone(), state.git_ignore, state.include_hidden, cancelled, tx);
                            state.loading = true;
                            state.rows.clear();
                            state.filtered_indices.clear();
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        KeyCode::Enter => {
                            if let Some(row) = state.selected_row() {
                                if let Some(launch) = launch_content_diff_spec(&state.panel_roots, row) {
                                    let detail = format!("{} {}", launch.program, launch.args.join(" "));
                                    let sources = state
                                        .panel_roots
                                        .iter()
                                        .map(|p| p.join(&row.relative_path).display().to_string())
                                        .collect();
                                    let task_id = self.push_task(
                                        self.focused_pane,
                                        "diff",
                                        format!("diff: {}", row.relative_path.display()),
                                        detail,
                                        sources,
                                        None,
                                    );
                                    self.pending_launch = Some(QueuedLaunch { task_id, launch });
                                    self.status = format!("opening diff for {}", row.relative_path.display());
                                } else if row.is_dir {
                                    self.status = format!("{} is a directory", row.relative_path.display());
                                }
                            }
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                        _ => {
                            self.pending_action = Some(PendingAction::DiffMatrix(state));
                        }
                    }
                }
                if let Some(action) = self.pending_action.as_ref() {
                    self.status = self.status_for_pending_action(action)?;
                }
            }
        }

        Ok(true)
    }

    /// 處理 command mode 中的按鍵編輯、候選切換與送出行為。
    pub(crate) fn handle_command_key(&mut self, key: KeyEvent) -> Result<bool> {
        let suggestions = self.command_suggestions();
        let path_completion_active =
            command_path_completion_context(self.current_pane_cwd(), &self.command_buffer)
                .is_some();
        if self.text_input_mode == RenameMode::Insert
            && path_completion_active
            && (key.code == KeyCode::Tab || key.code == KeyCode::BackTab)
            && !suggestions.is_empty()
        {
            self.apply_path_completion_tab_cycle(key.code == KeyCode::BackTab, &suggestions);
            return Ok(true);
        }
        if self.text_input_mode == RenameMode::Insert
            && !path_completion_active
            && (key.code == KeyCode::Tab || key.code == KeyCode::BackTab)
            && !suggestions.is_empty()
        {
            self.apply_command_completion_tab(key.code == KeyCode::BackTab, &suggestions);
            return Ok(true);
        }
        if self.text_input_mode == RenameMode::Insert
            && let Some(direction) = command_suggestion_navigation(&key)
        {
            if !suggestions.is_empty() {
                match direction {
                    SuggestionNavigation::Next => {
                        self.command_suggestion_selected =
                            (self.command_suggestion_selected + 1) % suggestions.len();
                    }
                    SuggestionNavigation::Previous => {
                        self.command_suggestion_selected =
                            (self.command_suggestion_selected + suggestions.len() - 1)
                                % suggestions.len();
                    }
                }
            }
            return Ok(true);
        }

        let mut buffer = std::mem::take(&mut self.command_buffer);
        let edit_result = self.edit_text_buffer(&mut buffer, &key);
        self.command_buffer = buffer;
        match edit_result {
            TextEditResult::Changed => {
                self.command_suggestion_selected = 0;
                self.command_completion_cycle = None;
                return Ok(true);
            }
            TextEditResult::Consumed => return Ok(true),
            TextEditResult::PassThrough => {}
        }

        match key.code {
            KeyCode::Esc => {
                self.command_mode = false;
                self.command_buffer.clear();
                self.command_suggestion_selected = 0;
                self.command_completion_cycle = None;
                self.status = String::from("normal mode");
            }
            KeyCode::Enter => {
                let selected_suggestion = suggestions
                    .get(
                        self.command_suggestion_selected
                            .min(suggestions.len().saturating_sub(1)),
                    )
                    .map(|entry| entry.command.trim_start_matches(':').to_string());
                let current = self.command_buffer.trim();
                let has_arguments = current.contains(char::is_whitespace);
                let path_like_input = looks_like_navigation_path(current);
                if let Some(suggestion) = selected_suggestion
                    && !suggestion.is_empty()
                    && suggestion != current
                    && !has_arguments
                    && !path_like_input
                {
                    self.command_buffer = suggestion;
                    self.text_input_cursor = self.command_buffer.chars().count();
                } else {
                    let command = std::mem::take(&mut self.command_buffer);
                    self.command_mode = false;
                    self.command_suggestion_selected = 0;
                    self.command_completion_cycle = None;
                    self.execute_command(command.trim())?;
                }
            }
            _ => {}
        }
        Ok(true)
    }

    /// 根據目前焦點 pane 與 command buffer，取得 command palette 應顯示的候選。
    fn command_suggestions(&self) -> Vec<CommandSuggestionLine> {
        self.command_completion_cycle.clone().map_or_else(
            || command_suggestions_for_buffer(self.current_pane_cwd(), &self.command_buffer),
            |cycle| cycle.suggestions,
        )
    }

    /// 取得目前焦點 pane 的工作目錄，供 command palette 的路徑補全使用。
    fn current_pane_cwd(&self) -> Option<&Path> {
        self.panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.as_path())
    }

    /// 在路徑補全模式下處理 `Tab` / `Shift+Tab`，提供共同前綴補齊與候選輪詢。
    fn apply_path_completion_tab_cycle(
        &mut self,
        reverse: bool,
        suggestions: &[CommandSuggestionLine],
    ) {
        if suggestions.is_empty() {
            return;
        }

        if suggestions.len() == 1 {
            self.command_buffer = suggestions[0].command.clone();
            self.text_input_cursor = self.command_buffer.chars().count();
            self.command_suggestion_selected = 0;
            self.command_completion_cycle = None;
            return;
        }

        let current = self.command_buffer.clone();
        let common_prefix = longest_common_prefix(
            &suggestions
                .iter()
                .map(|line| line.command.as_str())
                .collect::<Vec<_>>(),
        );
        if common_prefix.chars().count() > current.chars().count() {
            self.command_buffer = common_prefix;
            self.text_input_cursor = self.command_buffer.chars().count();
            self.command_suggestion_selected = 0;
            self.command_completion_cycle = None;
            return;
        }

        let len = suggestions.len();
        let selected = match &self.command_completion_cycle {
            Some(cycle) if cycle.suggestions == suggestions => {
                if reverse {
                    (self.command_suggestion_selected + len - 1) % len
                } else {
                    (self.command_suggestion_selected + 1) % len
                }
            }
            _ => {
                if reverse {
                    len - 1
                } else {
                    0
                }
            }
        };

        self.command_suggestion_selected = selected;
        self.command_buffer = suggestions[selected].command.clone();
        self.text_input_cursor = self.command_buffer.chars().count();
        self.command_completion_cycle = Some(CommandCompletionCycle {
            suggestions: suggestions.to_vec(),
        });
    }

    /// 在一般 command 提示模式下處理 `Tab` / `Shift+Tab`，直接採用最接近的候選。
    ///
    /// `Tab` 會使用目前選取的提示；若使用者尚未手動切換，預設採用排序後的第一筆。
    /// `Shift+Tab` 則會先往上一筆，再套用該提示，方便快速反向挑選。
    fn apply_command_completion_tab(
        &mut self,
        reverse: bool,
        suggestions: &[CommandSuggestionLine],
    ) {
        if suggestions.is_empty() {
            return;
        }

        let len = suggestions.len();
        let selected = if reverse {
            (self.command_suggestion_selected + len - 1) % len
        } else {
            self.command_suggestion_selected.min(len - 1)
        };

        self.command_suggestion_selected = selected;
        self.command_buffer = suggestions[selected]
            .command
            .trim_start_matches(':')
            .to_string();
        self.text_input_cursor = self.command_buffer.chars().count();
        self.command_completion_cycle = None;
    }

    /// 在一般模式按下 `Esc` 時，優先處理 filter 的兩段式離開流程。
    fn handle_escape_in_normal_mode(&mut self) {
        if let Some(filter) = self.filter.take() {
            if filter.editing {
                self.filter = Some(FilterState {
                    editing: false,
                    ..filter
                });
                self.status = String::from("filter active");
            } else {
                if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
                    pane.clear_filter();
                }
                self.status = String::from("normal mode");
            }
            return;
        }

        if self.clear_list_find_if_active() {
            return;
        }

        if self.has_any_marks() {
            self.clear_all_marks();
            return;
        }

        self.status = String::from("normal mode");
    }

    /// 執行 command mode 送出的命令字串。
    pub(crate) fn execute_command(&mut self, command: &str) -> Result<()> {
        match command {
            "q" => self.status = String::from("use q in normal mode to quit"),
            "goto" => self.status = String::from("usage: goto <path>"),
            "rename" => self.start_rename(),
            "create" => self.start_create_entry(),
            "copy" => self.copy_selected(),
            "mark-toggle" => self.toggle_mark_selected_in_focused_pane()?,
            "cancel-copy" => self.clear_clipboard(ClipboardOperation::Copy),
            "cancel-cut" => self.clear_clipboard(ClipboardOperation::Cut),
            "cut" => self.cut_selected(),
            "paste" => self.paste_into_focused_pane()?,
            "paste!" => self.paste_into_focused_pane_with_overwrite()?,
            "undo" => self.undo_latest_file_operation()?,
            "terminal" => self.open_terminal_in_active_panel()?,
            "compress" => self.compress_selected_entries()?,
            "extract" => self.extract_selected_archives()?,
            "jump" => self.open_fzf_jump(),
            "open" => self.open_selected_with_default()?,
            "open-picker" => self.open_selected_with_picker()?,
            "copy-picker" => self.open_copy_picker()?,
            "mark-all" | "select-all" => self.mark_all_in_focused_pane()?,
            "mark-invert" | "invert-selection" => self.invert_marks_in_focused_pane()?,
            "split" => self.split_current(SplitDirection::Horizontal)?,
            "vsplit" => self.split_current(SplitDirection::Vertical)?,
            "split-up" => {
                self.split_current_at(SplitDirection::Horizontal, SplitPlacement::Before)?
            }
            "split-left" => {
                self.split_current_at(SplitDirection::Vertical, SplitPlacement::Before)?
            }
            "vim" => {
                let Some(target) = self.selected_open_target() else {
                    self.status = String::from("nothing selected to open");
                    return Ok(());
                };
                self.queue_open_action(target, OpenAction::Vim)?;
            }
            "reveal" => {
                let Some(target) = self.selected_open_target() else {
                    self.status = String::from("nothing selected to reveal");
                    return Ok(());
                };
                self.queue_open_action(target, OpenAction::Reveal)?;
            }
            "unmark" | "unmark-all" => self.clear_marks_in_focused_pane()?,
            "preview" => self.open_preview_focus(),
            "preview-search" => self.open_preview_search_input(),
            "search" => self.open_global_search()?,
            "filter" => self.open_filter_input(FilterMode::Normal),
            "filter-fuzzy" | "filter fuzzy" | "fuzzy" => self.open_filter_input(FilterMode::Fuzzy),
            "linemode" => self.open_linemode_picker(),
            "bookmark" => self.open_bookmark_picker(),
            "bookmark add" => self.add_bookmark_with_auto_key(self.focused_pane)?,
            "bookmark jump" => self.open_bookmark_list(),
            "bookmark delete" => {
                self.open_bookmark_list_with_mode(self.focused_pane, BookmarkListMode::Delete);
            }
            "bookmark clear" => self.delete_all_bookmarks()?,
            "trash" => self.open_trash_panel()?,
            "tasks" => self.open_task_panel(),
            "status" => self.open_tool_panel(),
            "help" => self.open_help_panel(),
            "bookmark list" => self.open_bookmark_list(),
            "zoxide" => self.open_zoxide_list(),
            "trash undo" => self.restore_latest_from_trash()?,
            "delete" => self.start_delete_confirmation(false),
            "delete!" | "delete-permanently" => self.start_delete_confirmation(true),
            "theme list" => self.open_theme_picker(),
            "theme next" => self.cycle_theme(),
            "panel" | "pane" => {
                self.status = format!(
                    "usage: panel <panel-id>. available: {}",
                    self.available_pane_ids_label()
                );
            }
            "close" => self.close_current_pane(),
            "only" => self.only_current_pane(),
            "diff" | "df" | "d" => self.open_diff_matrix(None)?,
            "rename-regex" | "reg" => {
                self.status = String::from("usage: rename-regex <pattern> <replace>");
            }
            "" => self.status = String::from("normal mode"),
            other => {
                if let Some(args) = other
                    .strip_prefix("diff ")
                    .or_else(|| other.strip_prefix("df "))
                    .or_else(|| other.strip_prefix("d "))
                {
                    let ids = args
                        .split_whitespace()
                        .filter_map(|s| s.parse::<usize>().ok())
                        .collect::<Vec<_>>();
                    self.open_diff_matrix(Some(ids))?;
                } else if let Some(name) = other.strip_prefix("theme ") {
                    self.set_theme_by_name(name.trim());
                } else if let Some(path) = other.strip_prefix("goto ") {
                    self.change_directory_from_command(path.trim())?;
                } else if let Some(name) = other.strip_prefix("create ") {
                    self.create_entry_from_command(name)?;
                } else if let Some(args) = other
                    .strip_prefix("rename-regex ")
                    .or_else(|| other.strip_prefix("reg "))
                {
                    self.start_regex_rename_from_command(args)?;
                } else if let Some(args) = other.strip_prefix("move-panel ") {
                    self.move_selected_to_pane_id(args.trim())?;
                } else if let Some(args) = other.strip_prefix("panel ") {
                    self.focus_pane_by_id_argument(args.trim());
                } else if let Some(args) = other.strip_prefix("pane ") {
                    self.focus_pane_by_id_argument(args.trim());
                } else if let Some(path) = other.strip_prefix("move ") {
                    self.move_selected_to_path(path.trim())?;
                } else if let Some(args) = other.strip_prefix("linemode ") {
                    self.apply_line_mode_from_command(args.trim())?;
                } else if let Some(args) = other.strip_prefix("bookmark jump ") {
                    self.jump_to_bookmark_from_command(args.trim())?;
                } else if let Some(args) = other.strip_prefix("bookmark delete ") {
                    let Some(key) = parse_bookmark_argument(args) else {
                        self.status = String::from("usage: bookmark delete <key>");
                        return Ok(());
                    };
                    self.delete_bookmark(key)?;
                } else if looks_like_navigation_path(other) {
                    self.change_directory_from_command(other)?;
                } else {
                    self.status = format!("unknown command: {other}");
                }
            }
        }
        Ok(())
    }

    /// 取得目前有焦點的 pane 可變參考。
    pub(crate) fn current_pane_mut(&mut self) -> io::Result<&mut PaneState> {
        self.panes
            .get_mut(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused panel"))
    }

    /// 將目前焦點 pane 依指定方向分割成兩個 pane。
    pub(crate) fn split_current(&mut self, direction: SplitDirection) -> io::Result<()> {
        self.split_current_at(direction, SplitPlacement::After)
    }

    /// 將目前焦點 pane 依指定方向與位置分割成兩個 pane。
    pub(crate) fn split_current_at(
        &mut self,
        direction: SplitDirection,
        placement: SplitPlacement,
    ) -> io::Result<()> {
        let source_pane = self
            .panes
            .get(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused panel"))?;
        let cwd = source_pane.cwd.clone();
        let show_hidden = source_pane.show_hidden;
        let sort_mode = source_pane.sort_mode;

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let mut pane = PaneState::new(cwd)?;
        pane.set_show_hidden(show_hidden);
        pane.set_sort_mode(sort_mode);
        self.panes.insert(new_id, pane);
        self.layout =
            self.layout
                .clone()
                .split_leaf(self.focused_pane, direction, placement, new_id);
        self.focused_pane = new_id;
        self.status = match (direction, placement) {
            (SplitDirection::Horizontal, SplitPlacement::Before) => String::from("split up"),
            (SplitDirection::Horizontal, SplitPlacement::After) => String::from("split down"),
            (SplitDirection::Vertical, SplitPlacement::Before) => String::from("split left"),
            (SplitDirection::Vertical, SplitPlacement::After) => String::from("split right"),
        };
        Ok(())
    }

    /// 依照目前布局順序取得所有 pane id。
    pub(crate) fn ordered_pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.layout.pane_ids(&mut ids);
        ids
    }

    /// 將焦點直接切到指定 pane 編號。
    pub(crate) fn focus_pane_by_id(&mut self, target_pane_id: usize) {
        if self.panes.contains_key(&target_pane_id) {
            self.focused_pane = target_pane_id;
            self.status = format!("focused panel {target_pane_id}");
        } else {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
        }
    }

    /// 讓指定 panel 切換到目標路徑，並在成功後同步把最新目錄寫進 zoxide。
    ///
    /// 參數：
    /// - `pane_id: usize`，要操作的 panel 編號。
    /// - `target_path: &Path`，要切換或定位到的目標路徑。
    ///
    /// 回傳：`io::Result<()>`。
    fn go_to_path_and_track(&mut self, pane_id: usize, target_path: &Path) -> io::Result<()> {
        let Some(previous_cwd) = self.panes.get(&pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        if let Some((task_id, title, progress)) = self.active_file_job_for_path(target_path) {
            let pct_str = progress.map(|p| format!(" ({p}%)")).unwrap_or_default();
            self.status = format!(
                "cannot enter '{}': transfer in progress [task #{task_id}: {title}{pct_str}]",
                target_path.display()
            );
            return Ok(());
        }
        self.cancel_directory_load(pane_id);
        let current_cwd = {
            let pane = self.panes.get_mut(&pane_id).expect("panel checked above");
            pane.go_to_path(target_path)?;
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
            pane.cwd.clone()
        };
        self.restart_directory_size_scan_after_navigation(pane_id, &previous_cwd);
        self.zoxide_tracker.track(&current_cwd);
        Ok(())
    }

    /// 讓指定 panel 定位到某個檔案或目錄，並在成功後同步把結果目錄寫進 zoxide。
    ///
    /// 參數：
    /// - `pane_id: usize`，要操作的 panel 編號。
    /// - `target_path: &Path`，要被 reveal 的目標。
    ///
    /// 回傳：`io::Result<()>`。
    fn reveal_path_and_track(&mut self, pane_id: usize, target_path: &Path) -> io::Result<()> {
        let Some(previous_cwd) = self.panes.get(&pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        self.cancel_directory_load(pane_id);
        let current_cwd = {
            let pane = self.panes.get_mut(&pane_id).expect("panel checked above");
            pane.reveal_path(target_path)?;
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
            pane.cwd.clone()
        };
        self.restart_directory_size_scan_after_navigation(pane_id, &previous_cwd);
        self.zoxide_tracker.track(&current_cwd);
        Ok(())
    }

    /// 把目前 focus 的 panel 工作目錄寫進 zoxide，供一般瀏覽操作完成後同步學習。
    ///
    /// 這個 helper 專門給 `h/l` 與方向鍵這類直接操作 pane 的流程使用，
    /// 因為它們不會經過 `go_to_path_and_track()` 這類包裝函式。
    fn track_focused_pane_cwd_in_zoxide(&self) {
        if let Some(pane) = self.panes.get(&self.focused_pane) {
            self.zoxide_tracker.track(&pane.cwd);
        }
    }

    /// 從 `:panel <id>` 的參數解析目標 panel 編號並切換焦點。
    fn focus_pane_by_id_argument(&mut self, target: &str) {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return;
        };
        self.focus_pane_by_id(target_pane_id);
    }

    /// 關閉目前有焦點的 pane。
    pub(crate) fn close_current_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if ids.len() <= 1 {
            self.status = String::from("cannot close the last panel");
            return;
        }

        let old_focus = self.focused_pane;
        if let Some(index) = ids.iter().position(|id| *id == old_focus) {
            let fallback = if index > 0 {
                ids[index - 1]
            } else {
                ids[index + 1]
            };
            if let Some(layout) = self.layout.clone().close_pane(old_focus) {
                self.layout = layout;
                self.cancel_directory_load(old_focus);
                self.cancel_directory_size_scan(old_focus);
                self.panes.remove(&old_focus);
                if self
                    .global_search
                    .as_ref()
                    .is_some_and(|search| search.pane_id == old_focus)
                {
                    self.global_search = None;
                }
                self.focused_pane = fallback;
                self.status = format!("closed panel {old_focus}");
            }
        }
    }

    /// 僅保留目前有焦點的 pane，其餘全部關閉。
    pub(crate) fn only_current_pane(&mut self) {
        let focused = self.focused_pane;
        self.panes.retain(|id, _| *id == focused);
        self.layout = LayoutNode::Leaf { pane_id: focused };
        if self
            .global_search
            .as_ref()
            .is_some_and(|search| search.pane_id != focused)
        {
            self.global_search = None;
        }
        self.status = String::from("kept only focused panel");
    }

    /// 將主題切換到下一個內建預設值。
    pub(crate) fn cycle_theme(&mut self) {
        let next = self.theme_preset.next();
        self.apply_theme(next);
    }

    /// 打開主題選擇視窗，並將選項焦點設在目前主題。
    pub(crate) fn open_theme_picker(&mut self) {
        let original = self.theme_preset;
        let selected = ThemePreset::ALL
            .iter()
            .position(|preset| *preset == original)
            .unwrap_or(0);
        self.pending_action = Some(PendingAction::ThemePicker { selected, original });
        self.status = String::from("theme picker: use j/k preview, l apply, h cancel");
    }

    /// 即時預覽主題列表目前選到的色盤，但不會寫入設定檔。
    ///
    /// 參數：
    /// - `selected: usize`，`ThemePreset::ALL` 中目前選取的索引。
    /// - `original: ThemePreset`，開啟列表前使用的主題，供取消操作時還原。
    ///
    /// 回傳：`()`，函數會更新畫面主題並保留主題列表狀態。
    fn preview_theme_picker_selection(&mut self, selected: usize, original: ThemePreset) {
        let preset = ThemePreset::ALL[selected];
        self.theme = preset.into();
        self.pending_action = Some(PendingAction::ThemePicker { selected, original });
        self.status = format!("theme preview: {}", preset.name());
    }

    /// 打開底部排序面板，等待使用者輸入排序快捷鍵。
    pub(crate) fn open_sort_picker(&mut self) {
        self.pending_action = Some(PendingAction::SortPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("sort: choose a key from the panel");
    }

    /// 打開底部 `g` 系列命令面板，供 `gg`、`gt` 等 leader 指令共用。
    pub(crate) fn open_go_picker(&mut self) {
        self.pending_action = Some(PendingAction::GoPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("go: choose g/t/d/k from the panel");
    }

    /// 打開底部 panel 操作面板，讓使用者可視化選擇 `w` 的第二個按鍵。
    pub(crate) fn open_window_picker(&mut self) {
        self.pending_action = Some(PendingAction::WindowPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("panel: choose h/j/k/l/c/o/t/d from the panel");
    }

    /// 打開底部 linemode 面板，等待使用者輸入右側欄位顯示模式。
    pub(crate) fn open_linemode_picker(&mut self) {
        self.pending_action = Some(PendingAction::LineModePicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("linemode: choose a key from the panel");
    }

    /// 打開書籤功能面板，列出目前可用的書籤操作。
    pub(crate) fn open_bookmark_picker(&mut self) {
        self.pending_action = Some(PendingAction::BookmarkPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("bookmark: choose a/g/d/D from the panel");
    }

    /// 打開 trash 面板，列出目前可還原的項目。
    pub(crate) fn open_trash_panel(&mut self) -> io::Result<()> {
        self.pending_action = Some(PendingAction::TrashPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
            marked_ids: Vec::new(),
            visual_anchor: None,
        });
        self.help_return = None;
        self.status = trash_panel_status("", self.trash_store.list_entries()?.len(), 0, false, 0);
        Ok(())
    }

    /// 打開 `t` 系列命令面板，讓使用者選擇主題或 Trash 功能。
    ///
    /// 參數：無，功能固定作用於目前取得焦點的 panel。
    /// 回傳：`()`, 只更新目前的互動狀態與提示文字。
    pub(crate) fn open_theme_command_picker(&mut self) {
        self.pending_action = Some(PendingAction::ThemeCommandPicker {
            pane_id: self.focused_pane,
        });
        self.status = String::from("theme/trash: choose l/n/t/u from the panel");
    }

    /// 打開 F1 功能說明面板，支援 Vim 式滾動與面板內搜尋。
    pub(crate) fn open_help_panel(&mut self) {
        self.help_return = None;
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 打開 task 面板，查看目前 pane 最近執行過的任務與狀態。
    pub(crate) fn open_task_panel(&mut self) {
        let count = self.tasks_for_pane(self.focused_pane).len();
        self.pending_action = Some(PendingAction::TaskPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = task_panel_status("", count, 0, false);
    }

    /// 打開全螢幕 N 路目錄與檔案差異比對工作區 (Diff Matrix)。
    pub(crate) fn open_diff_matrix(&mut self, target_pane_ids: Option<Vec<usize>>) -> Result<()> {
        let pane_ids = match target_pane_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => self.panes.keys().copied().collect::<Vec<_>>(),
        };

        if pane_ids.len() < 2 {
            self.status = String::from("diff requires at least 2 open panels (e.g. use Ctrl+s / Ctrl+v to split first)");
            return Ok(());
        }

        let mut valid_ids = Vec::new();
        let mut roots = Vec::new();
        let mut labels = Vec::new();

        for id in pane_ids {
            if let Some(pane) = self.panes.get(&id) {
                valid_ids.push(id);
                roots.push(pane.cwd.clone());
                let tail = pane
                    .cwd
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| pane.cwd.display().to_string());
                labels.push(tail);
            }
        }

        if roots.len() < 2 {
            self.status = String::from("diff requires at least 2 valid panels");
            return Ok(());
        }

        // 取消任何先前的 diff background job
        if let Some(cancelled) = self.diff_job_cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }

        let cancelled = Arc::new(AtomicBool::new(false));
        self.diff_job_cancelled = Some(cancelled.clone());

        let (tx, rx) = std::sync::mpsc::channel();
        self.diff_job_rx = Some(rx);

        spawn_background_diff(roots.clone(), true, true, cancelled, tx);

        let diff_state = DiffMatrixState::new_loading(valid_ids, roots, labels);
        self.pending_action = Some(PendingAction::DiffMatrix(diff_state));
        self.status = format!("diff matrix: scanning {} panels...", self.panes.len());
        Ok(())
    }

    /// 輪詢背景目錄比對工作的接收端，非阻塞更新差異矩陣。
    fn poll_diff_job(&mut self) {
        let Some(receiver) = &self.diff_job_rx else {
            return;
        };
        let messages: Vec<DiffJobEvent> = receiver.try_iter().collect();
        if messages.is_empty() {
            return;
        }

        for message in messages {
            match message {
                DiffJobEvent::Discovered(count) => {
                    if let Some(PendingAction::DiffMatrix(state)) = &mut self.pending_action {
                        state.discovered_count = count;
                    }
                }
                DiffJobEvent::Done(rows) => {
                    let count = rows.len();
                    if let Some(PendingAction::DiffMatrix(state)) = &mut self.pending_action {
                        state.set_completed_rows(rows);
                        self.status = format!("diff matrix: compared {} items (press q to exit)", count);
                    }
                    self.diff_job_rx = None;
                    self.diff_job_cancelled = None;
                    break;
                }
                DiffJobEvent::Error(err) => {
                    self.status = format!("diff error: {err}");
                    self.diff_job_rx = None;
                    self.diff_job_cancelled = None;
                    break;
                }
            }
        }
    }

    /// 打開書籤列表彈窗，讓使用者可以用列表方式跳轉既有書籤。
    pub(crate) fn open_bookmark_list(&mut self) {
        self.open_bookmark_list_with_mode(self.focused_pane, BookmarkListMode::Jump);
    }

    /// 以指定模式打開書籤列表彈窗，供跳轉或刪除流程共用。
    pub(crate) fn open_bookmark_list_with_mode(&mut self, pane_id: usize, mode: BookmarkListMode) {
        self.pending_action = Some(PendingAction::BookmarkList {
            pane_id,
            selected: 0,
            mode,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = bookmark_list_status("", self.bookmark_store.list().len(), 0, mode, false);
    }

    /// 打開 zoxide 目錄列表，讓目前 panel 可依 frecency 快速跳到常用目錄。
    ///
    /// 這個面板是 `:zoxide` 的正式入口，`Z` 也會走這裡。
    pub(crate) fn open_zoxide_list(&mut self) {
        match query_zoxide_directories() {
            Ok(entries) => {
                let count = entries.len();
                self.pending_action = Some(PendingAction::ZoxideList {
                    pane_id: self.focused_pane,
                    selected: 0,
                    entries,
                    search: PanelSearchState {
                        buffer: String::new(),
                        editing: false,
                    },
                });
                self.status = zoxide_list_status("", count, 0, false);
            }
            Err(error) => {
                self.status = format!("zoxide failed: {error}");
                self.open_tool_panel();
            }
        }
    }

    /// 在目前 focus panel 顯示外部工具安裝狀態，讓使用者知道缺少哪些依賴。
    pub(crate) fn open_tool_panel(&mut self) {
        self.pending_action = Some(PendingAction::ToolPanel {
            pane_id: self.focused_pane,
            selected: 0,
        });
        self.status = String::from("dependencies: j/k move, Esc close");
    }

    /// 處理等待書籤按鍵時的輸入。
    pub(crate) fn handle_bookmark_key(&mut self, key: KeyEvent) -> Result<bool> {
        let Some(prompt) = self.pending_bookmark.take() else {
            return Ok(true);
        };

        match key.code {
            KeyCode::Esc => {
                self.status = String::from("normal mode");
            }
            KeyCode::Char(bookmark) => match prompt {
                BookmarkPrompt::Jump => self.jump_to_bookmark(bookmark)?,
            },
            _ => {
                self.pending_bookmark = Some(prompt);
                self.status = match prompt {
                    BookmarkPrompt::Jump => String::from("bookmark: choose an existing key"),
                };
            }
        }

        Ok(true)
    }

    /// 將目前焦點 pane 的目錄存成書籤。
    fn set_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let target = pane.bookmark_target.clone();
        match &target {
            BookmarkTarget::LocalPath(path) => {
                self.bookmark_store
                    .set(key, path.clone())
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.status = format!("bookmark [{key}] = {}", path.display());
            }
            BookmarkTarget::SmbLocation(location) => {
                self.bookmark_store
                    .set_smb(key, location.clone())
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.status = format!("bookmark [{key}] = {}", target.display_text());
            }
        }
        Ok(())
    }

    /// 自動挑選下一個可用書籤代號，並把指定 pane 目前位置存成書籤。
    ///
    /// 參數：
    /// - `pane_id: usize`，要儲存位置的 pane 編號。
    ///
    /// 回傳：`io::Result<()>`。
    fn add_bookmark_with_auto_key(&mut self, pane_id: usize) -> io::Result<()> {
        let Some(key) = self.bookmark_store.next_available_key() else {
            self.status = String::from("bookmark: no available auto key");
            return Ok(());
        };

        self.focused_pane = pane_id;
        self.set_bookmark(key)
    }

    /// 跳到指定書籤對應的路徑。
    fn jump_to_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(target) = self.bookmark_store.get(key).cloned() else {
            self.status = format!("bookmark [{key}] not found");
            return Ok(());
        };

        self.jump_to_bookmark_target(self.focused_pane, key, &target)
    }

    /// 讓 `:bookmark jump <key>` 可以直接跳到指定書籤。
    fn jump_to_bookmark_from_command(&mut self, args: &str) -> io::Result<()> {
        let Some(key) = parse_bookmark_argument(args) else {
            self.status = String::from("usage: bookmark jump <key>");
            return Ok(());
        };
        self.jump_to_bookmark(key)
    }

    /// 刪除指定代號的單一書籤，並同步更新狀態列。
    ///
    /// 參數：
    /// - `key: char`，要刪除的書籤代號。
    ///
    /// 回傳：`io::Result<()>`。
    fn delete_bookmark(&mut self, key: char) -> io::Result<()> {
        let removed = self
            .bookmark_store
            .remove(key)
            .map_err(|error| io::Error::other(error.to_string()))?;

        if removed {
            self.status = format!("bookmark [{key}] deleted");
        } else {
            self.status = format!("bookmark [{key}] not found");
        }
        Ok(())
    }

    /// 根據目前書籤列表選取位置刪除單一書籤。
    ///
    /// 參數：
    /// - `entries: &[BookmarkEntry]`，目前列表中可見的書籤資料。
    /// - `selected: usize`，目前游標指到的列索引。
    ///
    /// 回傳：`io::Result<()>`。
    fn delete_bookmark_from_list(
        &mut self,
        entries: &[BookmarkEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("bookmark delete: empty");
            return Ok(());
        };

        self.delete_bookmark(entry.key)
    }

    /// 清空全部書籤，並同步寫回 `bookmark.toml`。
    ///
    /// 回傳：`io::Result<()>`。
    fn delete_all_bookmarks(&mut self) -> io::Result<()> {
        self.bookmark_store
            .clear()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.status = String::from("all bookmarks deleted");
        Ok(())
    }

    /// 讓 `:linemode <mode>` 可以直接切換目前 pane 的右側欄位顯示模式。
    ///
    /// 支援：
    /// - `size`
    /// - `permissions`
    /// - `btime`
    /// - `mtime`
    /// - `none`
    fn apply_line_mode_from_command(&mut self, args: &str) -> io::Result<()> {
        let line_mode = match args {
            "size" => LineMode::Size,
            "permissions" => LineMode::Permissions,
            "btime" => LineMode::Btime,
            "mtime" => LineMode::Mtime,
            "none" => LineMode::None,
            _ => {
                self.status = String::from("usage: linemode <size|permissions|btime|mtime|none>");
                return Ok(());
            }
        };

        self.apply_line_mode(self.focused_pane, line_mode)
    }

    /// 讓目前焦點 pane 依 `goto smb://...` 進入指定的 SMB share；若尚未掛載則先請求系統掛載。
    fn goto_smb_location(&mut self, target: &str) -> io::Result<()> {
        self.goto_smb_location_with_mount_root(target, std::path::Path::new("/Volumes"))
    }

    /// 用指定掛載根目錄測試或進入 SMB share，方便在測試中模擬 macOS 的掛載點。
    fn goto_smb_location_with_mount_root(
        &mut self,
        target: &str,
        mount_root: &std::path::Path,
    ) -> io::Result<()> {
        #[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
        let _ = mount_root;

        let location = match parse_smb_location(target) {
            Ok(location) => location,
            Err(error) => {
                self.status = error.to_string();
                return Ok(());
            }
        };

        #[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
        let resolved = resolve_smb_location(&location);

        #[cfg(any(all(not(target_os = "windows"), not(target_os = "macos")), test))]
        let resolved = resolve_smb_location_with_mount_root(&location, mount_root);

        match resolved {
            ResolvedSmbLocation::Ready(path) => {
                if !path.exists() {
                    self.status = format!("smb path missing: {}", path.display());
                    return Ok(());
                }
                if !self.panes.contains_key(&self.focused_pane) {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
                self.go_to_path_and_track(self.focused_pane, &path)?;
                let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                };
                pane.set_bookmark_target(BookmarkTarget::SmbLocation(location.url.clone()));
                self.full_redraw_requested = true;
                self.status = format!("jumped to smb: {}", location.url);
            }
            ResolvedSmbLocation::NeedsMount { local_path } => {
                let launch = build_smb_mount_launch(&location);
                let task_id = self.push_task(
                    self.focused_pane,
                    "smb",
                    format!("mount {}", location.url),
                    format!("expected mount path: {}", local_path.display()),
                    vec![location.url.clone()],
                    Some(local_path.display().to_string()),
                );
                self.pending_launch = Some(QueuedLaunch { task_id, launch });
                self.status = format!(
                    "已請求系統掛載 SMB：{}；若系統連線失敗，請檢查主機、share 名稱、網路與權限，成功後再重試。預期掛載位置：{}",
                    location.url,
                    local_path.display()
                );
            }
        }
        Ok(())
    }

    /// 依書籤目標型別跳到本機目錄，或自動發起 SMB 掛載／進入流程。
    fn jump_to_bookmark_target(
        &mut self,
        pane_id: usize,
        key: char,
        target: &BookmarkTarget,
    ) -> io::Result<()> {
        self.jump_to_bookmark_target_with_mount_root(
            pane_id,
            key,
            target,
            std::path::Path::new("/Volumes"),
        )
    }

    /// 依書籤目標型別跳到本機目錄，或用指定掛載根目錄自動發起 SMB 掛載／進入流程。
    fn jump_to_bookmark_target_with_mount_root(
        &mut self,
        pane_id: usize,
        key: char,
        target: &BookmarkTarget,
        mount_root: &std::path::Path,
    ) -> io::Result<()> {
        self.focused_pane = pane_id;

        match target {
            BookmarkTarget::LocalPath(path) => {
                if !self.panes.contains_key(&pane_id) {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
                if !path.exists() {
                    self.status = format!("bookmark [{key}] missing: {}", path.display());
                    return Ok(());
                }
                self.go_to_path_and_track(pane_id, path)?;
                self.status = format!("jumped to bookmark [{key}]");
            }
            BookmarkTarget::SmbLocation(location) => {
                self.goto_smb_location_with_mount_root(location, mount_root)?;
                if self.status.starts_with("jumped to smb:") {
                    self.status = format!("jumped to bookmark [{key}]");
                } else if self.status.starts_with("已請求系統掛載 SMB：") {
                    self.status = format!("bookmark [{key}] 正在連線：{location}");
                }
            }
        }

        Ok(())
    }

    /// 從書籤列表彈窗中打開目前選取的書籤。
    fn open_bookmark_from_list(
        &mut self,
        pane_id: usize,
        entries: &[BookmarkEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("bookmark jump: empty");
            return Ok(());
        };
        let Some(target) = self.bookmark_store.get(entry.key).cloned() else {
            self.status = format!("bookmark [{}] not found", entry.key);
            return Ok(());
        };
        self.jump_to_bookmark_target(pane_id, entry.key, &target)
    }

    /// 從 zoxide 列表中打開目前選取的目錄。
    fn open_zoxide_from_list(
        &mut self,
        pane_id: usize,
        entries: &[PathBuf],
        selected: usize,
    ) -> io::Result<()> {
        let Some(target_path) = entries.get(selected).cloned() else {
            self.status = String::from("zoxide: empty");
            return Ok(());
        };

        self.go_to_path_and_track(pane_id, &target_path)?;
        self.focused_pane = pane_id;
        self.status = format!("jumped via zoxide: {}", target_path.display());
        Ok(())
    }

    /// 取得目前選取項目的外部開啟目標資訊。
    fn selected_open_target(&self) -> Option<OpenTarget> {
        let pane = self.panes.get(&self.focused_pane)?;
        let entry = pane.selected_entry()?;
        Some(OpenTarget {
            path: entry.path.clone(),
            display_name: entry.display_name(),
            is_dir: entry.is_dir,
        })
    }

    /// 使用預設動作開啟目前選取項目。
    pub(crate) fn open_selected_with_default(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to open");
            return Ok(());
        };
        self.queue_open_action(target, default_open_action())
    }

    /// 打開 `Open with` 面板，讓使用者選擇外部開啟方式。
    pub(crate) fn open_selected_with_picker(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to open");
            return Ok(());
        };
        let options = self.open_picker_options_for_target(&target);

        self.pending_action = Some(PendingAction::OpenPicker {
            pane_id: self.focused_pane,
            target: target.clone(),
            selected: 0,
            options: options.clone(),
        });
        self.status = format!("open with: {}", target.display_name);
        Ok(())
    }

    /// 打開 `Copy` 面板，讓使用者把不同格式的文字直接複製到系統剪貼簿。
    pub(crate) fn open_copy_picker(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to copy");
            return Ok(());
        };

        self.pending_action = Some(PendingAction::CopyPicker {
            pane_id: self.focused_pane,
            target: target.clone(),
            selected: 0,
        });
        self.status = format!("copy to clipboard: {}", target.display_name);
        Ok(())
    }

    /// 根據選擇的複製動作，把文字寫進系統剪貼簿。
    fn copy_target_to_system_clipboard(
        &mut self,
        target: OpenTarget,
        action: CopyAction,
    ) -> io::Result<()> {
        let text = build_copy_text(&target, action)
            .map_err(|error| io::Error::other(error.to_string()))?;
        write_text_to_system_clipboard(&text)?;
        self.status = format!(
            "{}: {}",
            copy_action_status_label(action),
            target.display_name
        );
        Ok(())
    }

    /// 檢查指定路徑是否目前正由背景檔案工作（複製、貼上、移動、刪除、壓縮、解壓）寫入或修改中。
    /// 若正在處理，回傳該工作的資訊（工作編號、標題、完成百分比）。
    pub(crate) fn active_file_job_for_path(
        &self,
        path: &Path,
    ) -> Option<(usize, String, Option<u8>)> {
        if self.active_file_job_busy_paths.is_empty() {
            return None;
        }
        let canon_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        for (task_id, busy_paths) in &self.active_file_job_busy_paths {
            if busy_paths.iter().any(|busy| {
                let canon_busy = busy.canonicalize().unwrap_or_else(|_| busy.to_path_buf());
                canon_path == canon_busy
                    || canon_path.starts_with(&canon_busy)
                    || path == busy
                    || path.starts_with(busy)
            }) {
                let task = self.task_log.iter().find(|t| t.id == *task_id);
                let title = task
                    .map(|t| t.title.clone())
                    .unwrap_or_else(|| String::from("task in progress"));
                let progress = task.and_then(|t| match (t.completed_bytes, t.total_bytes) {
                    (Some(c), Some(tot)) if tot > 0 => {
                        Some(((c as f64 / tot as f64) * 100.0).min(100.0) as u8)
                    }
                    _ => t.progress_percent,
                });
                return Some((*task_id, title, progress));
            }
        }
        None
    }

    /// 回傳指定路徑目前正處於背景工作中的狀態標籤（例如 `[copying 99%]` 或 `[deleting...]`），若無進行中工作則回傳 `None`。
    pub(crate) fn active_job_badge_for_path(&self, path: &Path) -> Option<String> {
        // 一般瀏覽狀態沒有背景工作時是最常見路徑，必須在 canonicalize 前返回。
        // 否則大型目錄即使完全沒有 task，畫一幀仍會對每個可見路徑做同步磁碟查詢。
        if self.active_file_job_busy_paths.is_empty() {
            return None;
        }
        for (task_id, busy_paths) in &self.active_file_job_busy_paths {
            // UI badge 的 busy path 與 pane entry 都由 PaneFM 內部產生，已使用同一套完整
            // 路徑；render 熱路徑只做字面前綴比較，不能再觸發同步檔案系統 I/O。
            if busy_paths
                .iter()
                .any(|busy| path == busy || path.starts_with(busy))
            {
                let task = self.task_log.iter().find(|t| t.id == *task_id)?;
                let action = match task.kind.as_str() {
                    "paste" => {
                        if task.title.starts_with("move") {
                            "moving"
                        } else {
                            "copying"
                        }
                    }
                    "extract" => "extracting",
                    "compress" => "compressing",
                    "delete" => "deleting",
                    _ => "busy",
                };
                let progress = match (task.completed_bytes, task.total_bytes) {
                    (Some(c), Some(tot)) if tot > c && tot > 0 => {
                        Some(((c as f64 / tot as f64) * 100.0).min(100.0) as u8)
                    }
                    _ => task.progress_percent,
                };
                return match progress {
                    Some(pct) => Some(format!("[{action} {pct}%]")),
                    None => match task.completed_bytes {
                        Some(c) if c > 0 => Some(format!("[{action} {}]", format_task_bytes(c))),
                        _ => Some(format!("[{action}...]")),
                    },
                };
            }
        }
        None
    }

    /// 將外部開啟動作排入待執行佇列。
    fn queue_open_action(&mut self, target: OpenTarget, action: OpenAction) -> io::Result<()> {
        if target.is_dir {
            if let Some((task_id, title, progress)) = self.active_file_job_for_path(&target.path) {
                let pct_str = progress.map(|p| format!(" ({p}%)")).unwrap_or_default();
                self.status = format!(
                    "cannot open '{}': transfer in progress [task #{task_id}: {title}{pct_str}]",
                    target.display_name
                );
                return Ok(());
            }
        }
        let launch = build_launch_spec(&target, action)?;
        let title = match action {
            OpenAction::Editor => format!("open {} with editor", target.display_name),
            OpenAction::Vim => format!("open {} with vim", target.display_name),
            OpenAction::Open => format!("open {}", target.display_name),
            OpenAction::Reveal => format!("reveal {}", target.display_name),
        };
        let detail = format!("{} {}", launch.program, launch.args.join(" "));
        let task_id = self.push_task(
            self.focused_pane,
            "open",
            title,
            detail,
            vec![target.path.display().to_string()],
            None,
        );
        self.pending_launch = Some(QueuedLaunch { task_id, launch });
        self.status = match action {
            OpenAction::Editor => format!("opening {} with editor", target.display_name),
            OpenAction::Vim => format!("opening {} with vim", target.display_name),
            OpenAction::Open => format!("opening {}", target.display_name),
            OpenAction::Reveal => format!("revealing {}", target.display_name),
        };
        Ok(())
    }

    /// 以目前 active panel 的目錄開啟新終端，並排入統一外部程序佇列。
    ///
    /// 多 panel 時只讀取 `focused_pane` 的 cwd。若 plugins.toml 有 `[terminal]`，會使用
    /// 公司環境指定的 TrustView 等入口；否則 Windows 直接建立繼承權杖的新 console，
    /// macOS 則優先延續目前終端 App，無法辨識時才交給 Terminal.app。
    ///
    /// 參數：`self: &mut App`，目前應用程式狀態。
    /// 回傳：`io::Result<()>`；active panel 不存在或命令無法建立時回傳錯誤。
    fn open_terminal_in_active_panel(&mut self) -> io::Result<()> {
        let cwd = self
            .panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "panel no longer exists"))?;
        let launch = build_terminal_launch_spec(&cwd, self.config.actions.terminal.as_ref())?;
        let detail = format!("{} {}", launch.program, launch.args.join(" "));
        let task_id = self.push_task(
            self.focused_pane,
            "terminal",
            format!("open terminal: {}", cwd.display()),
            detail,
            vec![cwd.display().to_string()],
            None,
        );
        self.pending_launch = Some(QueuedLaunch { task_id, launch });
        self.status = format!("opening terminal: {}", cwd.display());
        Ok(())
    }

    /// 根據目前選取目標與設定檔，組出 Open with 面板應顯示的完整選項。
    ///
    /// `plugins.toml` 是使用者客製化層；若自訂動作與內建選項同名（例如 `Vim`
    /// 或 `Reveal`），自訂動作會在原本的位置覆寫內建動作。不同名的外掛才追加到
    /// 選單尾端。名稱比較不區分大小寫，也會移除前後空白。
    ///
    /// 參數：`target: &OpenTarget`，目前 active panel 選取的檔案或目錄。
    /// 回傳：`Vec<OpenPickerOption>`，已套用外掛覆寫且名稱不重複的選項。
    fn open_picker_options_for_target(&self, target: &OpenTarget) -> Vec<OpenPickerOption> {
        let mut options = open_picker_options(target);
        let mut label_positions = options
            .iter()
            .enumerate()
            .map(|(index, option)| (option.label.trim().to_lowercase(), index))
            .collect::<BTreeMap<_, _>>();

        for action in self
            .config
            .actions
            .open_with
            .iter()
            .filter(|action| custom_action_applies_to_target(action, target))
        {
            let normalized_label = action.name.trim().to_lowercase();
            if normalized_label.is_empty() {
                continue;
            }
            let option = OpenPickerOption {
                label: action.name.clone(),
                action: OpenPickerAction::Custom(action.clone()),
            };
            if let Some(index) = label_positions.get(&normalized_label).copied() {
                options[index] = option;
            } else {
                let index = options.len();
                options.push(option);
                label_positions.insert(normalized_label, index);
            }
        }
        options
    }

    /// 依照 Open with 面板中的選項類型，排入內建或自訂的外部動作。
    fn queue_open_picker_action(
        &mut self,
        target: OpenTarget,
        action: OpenPickerAction,
    ) -> io::Result<()> {
        match action {
            OpenPickerAction::Builtin(action) => self.queue_open_action(target, action),
            OpenPickerAction::Custom(action) => {
                let launch = build_custom_launch_spec(&target, &action)?;
                let title = format!("run {} on {}", action.name, target.display_name);
                let detail = format!("{} {}", launch.program, launch.args.join(" "));
                let task_id = self.push_task(
                    self.focused_pane,
                    "open",
                    title,
                    detail,
                    vec![target.path.display().to_string()],
                    None,
                );
                self.pending_launch = Some(QueuedLaunch { task_id, launch });
                self.status = format!("running {} on {}", action.name, target.display_name);
                Ok(())
            }
        }
    }

    /// 取出目前排隊中的外部開啟請求，交給主事件迴圈處理。
    pub(crate) fn take_pending_launch(&mut self) -> Option<QueuedLaunch> {
        self.pending_launch.take()
    }

    /// 根據外部開啟結果，更新 task manager 中對應任務的最終狀態。
    pub(crate) fn finish_launch_task(&mut self, task_id: usize, result: io::Result<()>) {
        match result {
            Ok(()) => self.finish_task(task_id, TaskState::Done, String::from("completed")),
            Err(error) => {
                let detail = error.to_string();
                self.finish_task(task_id, TaskState::Failed, detail.clone());
                self.status = format!("open failed: {detail}");
            }
        }
    }

    /// 取出目前排隊中的 `fzf` 跳轉請求，交給主事件迴圈處理。
    pub(crate) fn take_pending_fzf_jump(&mut self) -> Option<FzfJumpRequest> {
        self.pending_fzf_jump.take()
    }

    /// 套用 `fzf` 選取結果；取消時保留原本列表狀態。
    pub(crate) fn apply_fzf_jump_selection(
        &mut self,
        request: FzfJumpRequest,
        selected_line: Option<&str>,
    ) {
        let Some(line) = selected_line.map(str::trim).filter(|line| !line.is_empty()) else {
            self.finish_task(
                request.task_id,
                TaskState::Cancelled,
                String::from("fzf cancelled"),
            );
            self.status = String::from("jump cancelled");
            return;
        };

        if !self.panes.contains_key(&request.pane_id) {
            self.finish_task(
                request.task_id,
                TaskState::Failed,
                String::from("panel no longer exists"),
            );
            self.status = String::from("jump failed: panel no longer exists");
            return;
        }

        let target_path = jump_selection_to_path(&request.root_dir, line);
        let go_to_path_started_at = Instant::now();
        match self.go_to_path_and_track(request.pane_id, &target_path) {
            Ok(()) => {
                debug_timing_message(&format!("jump target path: {}", target_path.display()));
                debug_timing_log("jump go_to_path", go_to_path_started_at);
                self.finish_task(request.task_id, TaskState::Done, format!("opened {line}"));
                self.status = format!("jumped: {line}");
            }
            Err(error) => {
                debug_timing_log("jump go_to_path (failed)", go_to_path_started_at);
                self.finish_task(request.task_id, TaskState::Failed, error.to_string());
                self.status = format!("jump failed for {line}: {error}");
            }
        }
    }

    /// 以目前正在操作的上下文為返回點，打開 help 面板。
    pub(crate) fn open_help_from_current(&mut self) {
        self.help_return = self.capture_help_return_state();
        self.pending_action = Some(PendingAction::HelpPanel {
            pane_id: self.focused_pane,
            selected: 0,
            search: PanelSearchState {
                buffer: String::new(),
                editing: false,
            },
        });
        self.status = help_panel_status("", help_entries("").len(), false);
    }

    /// 擷取目前互動狀態，供 help 面板關閉後回復。
    fn capture_help_return_state(&mut self) -> Option<HelpReturnState> {
        if let Some(action) = self.pending_action.take() {
            return Some(HelpReturnState::Pending(action));
        }
        if let Some(filter) = self.filter.take() {
            return Some(HelpReturnState::Filter(filter));
        }
        if let Some(search) = self.preview_search.take() {
            return Some(HelpReturnState::PreviewSearch(search));
        }
        if let Some(search) = self.list_find.take() {
            return Some(HelpReturnState::ListFind(search));
        }
        if let Some(search) = self.global_search.take() {
            self.cancel_global_search();
            return Some(HelpReturnState::GlobalSearch(search));
        }
        if let Some(selection) = self.visual_selection.take() {
            return Some(HelpReturnState::VisualSelection(selection));
        }
        if self.command_mode {
            self.command_mode = false;
            self.command_completion_cycle = None;
            return Some(HelpReturnState::CommandMode(std::mem::take(
                &mut self.command_buffer,
            )));
        }
        if let Some(prompt) = self.pending_bookmark.take() {
            return Some(HelpReturnState::PendingBookmark(prompt));
        }
        if let Some(pane) = self.panes.get_mut(&self.focused_pane)
            && pane.is_preview_active()
        {
            pane.set_preview_active(false);
            return Some(HelpReturnState::PreviewFocus(self.focused_pane));
        }
        None
    }

    /// 從 help 面板回到先前的互動上下文。
    fn restore_help_return_state(&mut self, preserve_status: bool) -> io::Result<()> {
        let previous_status = self.status.clone();
        let Some(state) = self.help_return.take() else {
            self.status = String::from("normal mode");
            return Ok(());
        };

        match state {
            HelpReturnState::Pending(action) => {
                self.status = self.status_for_pending_action(&action)?;
                self.pending_action = Some(action);
            }
            HelpReturnState::Filter(filter) => {
                self.status = if filter.editing {
                    if filter.buffer.is_empty() {
                        String::from("filter: all")
                    } else {
                        format!("filter: {}", filter.buffer)
                    }
                } else if filter.buffer.is_empty() {
                    String::from("filter active")
                } else {
                    format!("filter locked: {}", filter.buffer)
                };
                self.filter = Some(filter);
            }
            HelpReturnState::PreviewSearch(search) => {
                self.status =
                    preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
                self.preview_search = Some(search);
            }
            HelpReturnState::ListFind(search) => {
                self.status =
                    list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
                self.list_find = Some(search);
            }
            HelpReturnState::GlobalSearch(search) => {
                self.status = global_search_status(
                    search.mode,
                    &search.buffer,
                    search.results.len(),
                    search.editing,
                    search.searched,
                    search.loading,
                );
                self.global_search = Some(search);
            }
            HelpReturnState::VisualSelection(selection) => {
                self.visual_selection = Some(selection);
                self.status = self.visual_status_label();
            }
            HelpReturnState::CommandMode(buffer) => {
                self.open_prefilled_command(buffer);
            }
            HelpReturnState::PendingBookmark(prompt) => {
                self.pending_bookmark = Some(prompt);
                self.status = match prompt {
                    BookmarkPrompt::Jump => String::from("bookmark: press a key to jump"),
                };
            }
            HelpReturnState::PreviewFocus(pane_id) => {
                if let Some(pane) = self.panes.get_mut(&pane_id) {
                    pane.set_preview_active(true);
                    self.status = String::from("preview mode");
                } else {
                    self.status = String::from("panel no longer exists");
                }
            }
        }

        if preserve_status {
            self.status = previous_status;
        }

        Ok(())
    }

    /// 依 pending action 類型回傳適合顯示的狀態文字。
    fn status_for_pending_action(&self, action: &PendingAction) -> io::Result<String> {
        Ok(match action {
            PendingAction::ConfirmDelete {
                target_name,
                permanent,
                ..
            } => {
                if *permanent {
                    format!("confirm delete {target_name}: y/n")
                } else {
                    format!("confirm trash {target_name}: y/n")
                }
            }
            PendingAction::ConfirmPasteOverwrite {
                target_name,
                entry_count,
                ..
            } => paste_overwrite_confirm_status(target_name, *entry_count),
            PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                ..
            } => trash_confirm_status(action, target_name, *entry_count),
            PendingAction::GoPicker { .. } => String::from("go: choose g/t/d/k from the panel"),
            PendingAction::ThemeCommandPicker { .. } => {
                String::from("theme/trash: choose l/n/t/u from the panel")
            }
            PendingAction::SortPicker { .. } => String::from("sort: choose a key from the panel"),
            PendingAction::WindowPicker { .. } => {
                String::from("panel: choose h/j/k/l/c/o/t/d from the panel")
            }
            PendingAction::LineModePicker { .. } => {
                String::from("linemode: choose a key from the panel")
            }
            PendingAction::ThemePicker { selected, .. } => {
                format!("theme picker: {}", ThemePreset::ALL[*selected].name())
            }
            PendingAction::TaskPanel {
                pane_id,
                selected,
                search,
            } => {
                let filtered =
                    filtered_task_entries(&self.tasks_for_pane(*pane_id), &search.buffer);
                task_panel_status(&search.buffer, filtered.len(), *selected, search.editing)
            }
            PendingAction::BookmarkPicker { .. } => {
                String::from("bookmark: choose a/g/d/D from the panel")
            }
            PendingAction::TrashPanel {
                selected,
                search,
                marked_ids,
                ..
            } => {
                let visible = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
                trash_panel_status(
                    &search.buffer,
                    visible,
                    *selected,
                    search.editing,
                    marked_ids.len(),
                )
            }
            PendingAction::HelpPanel { search, .. } => help_panel_status(
                &search.buffer,
                help_entries(&search.buffer).len(),
                search.editing,
            ),
            PendingAction::ToolPanel { .. } => String::from("dependencies: j/k move, Esc close"),
            PendingAction::BookmarkList {
                selected,
                mode,
                search,
                ..
            } => {
                let filtered =
                    filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer);
                bookmark_list_status(
                    &search.buffer,
                    filtered.len(),
                    *selected,
                    *mode,
                    search.editing,
                )
            }
            PendingAction::ZoxideList {
                entries,
                selected,
                search,
                ..
            } => {
                let filtered = filtered_zoxide_entries(entries, &search.buffer);
                zoxide_list_status(&search.buffer, filtered.len(), *selected, search.editing)
            }
            PendingAction::CopyPicker { target, .. } => {
                format!("copy to clipboard: {}", target.display_name)
            }
            PendingAction::OpenPicker { target, .. } => {
                format!("open with: {}", target.display_name)
            }
            PendingAction::Rename { mode, .. } => match mode {
                RenameMode::Insert => String::from("rename: insert"),
                RenameMode::Normal => String::from("rename: normal"),
            },
            PendingAction::CreateEntry { mode, .. } => match mode {
                RenameMode::Insert => create_status_label("insert"),
                RenameMode::Normal => create_status_label("normal"),
            },
            PendingAction::RegexRename {
                previews,
                pattern,
                replacement,
                ..
            } => regex_rename_status(pattern, replacement, previews),
            PendingAction::DiffMatrix(state) => {
                format!(
                    "diff matrix: {} items (filter: {})",
                    state.filtered_indices.len(),
                    state.filter_mode.label()
                )
            }
        })
    }

    /// 依照主題名稱字串套用指定主題。
    pub(crate) fn set_theme_by_name(&mut self, name: &str) {
        match ThemePreset::from_name(name) {
            Some(preset) => self.apply_theme(preset),
            None => {
                let available = ThemePreset::ALL
                    .iter()
                    .map(|preset| preset.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                self.status = format!("unknown theme: {name}. available: {available}");
            }
        }
    }

    /// 直接套用指定的主題預設值。
    pub(crate) fn apply_theme(&mut self, preset: ThemePreset) {
        self.theme_preset = preset;
        self.theme = preset.into();
        self.config.ui.theme_preset = preset;
        match persist_theme(&self.config_source, preset) {
            Ok(()) => {
                self.status = format!("theme: {}", preset.name());
            }
            Err(error) => {
                self.status = format!("theme: {} (save failed: {error})", preset.name());
            }
        }
    }

    /// 開始重新命名流程，建立一個待輸入的新名稱互動。
    pub(crate) fn start_rename(&mut self) {
        let Some(entry) = self
            .panes
            .get(&self.focused_pane)
            .and_then(PaneState::selected_entry)
            .cloned()
        else {
            self.status = String::from("nothing selected to rename");
            return;
        };

        self.pending_action = Some(PendingAction::Rename {
            pane_id: self.focused_pane,
            original_name: entry.display_name(),
            cursor: rename_basename_cursor(&entry.name),
            buffer: entry.name,
            mode: RenameMode::Insert,
        });
        self.status = String::from("rename: insert");
    }

    /// 開始建立新檔案流程，打開一個可直接輸入名稱的 inline 編輯器。
    ///
    /// 參數：無。
    /// 回傳：`()`
    /// 打開一個可直接輸入建立路徑的 inline 編輯器。
    ///
    /// 回傳：`()`
    pub(crate) fn start_create_entry(&mut self) {
        self.pending_action = Some(PendingAction::CreateEntry {
            pane_id: self.focused_pane,
            buffer: String::new(),
            cursor: 0,
            mode: RenameMode::Insert,
        });
        self.status = create_status_label("insert");
    }

    /// 解析 `:rename-regex` 指令參數，並建立批次改名預覽。
    ///
    /// 參數：
    /// - `args: &str`，使用者輸入在 `rename-regex` 後面的原始參數字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn start_regex_rename_from_command(&mut self, args: &str) -> io::Result<()> {
        let parsed = shlex::split(args).unwrap_or_default();
        if parsed.len() != 2 {
            self.status = String::from("usage: rename-regex <pattern> <replace>");
            return Ok(());
        }
        self.open_regex_rename_preview(&parsed[0], &parsed[1])
    }

    /// 依照 regex 規則建立目前選取項目的批次改名預覽。
    ///
    /// 參數：
    /// - `pattern: &str`，要用來匹配檔名的 regex。
    /// - `replacement: &str`，匹配後要替換成的新文字，支援 `$1` 這類群組語法。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn open_regex_rename_preview(
        &mut self,
        pattern: &str,
        replacement: &str,
    ) -> io::Result<()> {
        let regex = match Regex::new(pattern) {
            Ok(regex) => regex,
            Err(error) => {
                self.status = format!("invalid regex: {error}");
                return Ok(());
            }
        };
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to rename");
            return Ok(());
        }

        let selected_paths = entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>();
        let existing_names = pane
            .entries
            .iter()
            .filter(|entry| !selected_paths.contains(&entry.path))
            .map(|entry| entry.name.clone())
            .collect::<BTreeSet<_>>();

        let mut previews = entries
            .into_iter()
            .map(|entry| {
                let new_name = regex.replace_all(&entry.name, replacement).into_owned();
                let outcome = classify_regex_rename_preview(&entry.name, &new_name);
                RegexRenamePreview {
                    source_path: entry.path,
                    original_name: entry.name,
                    new_name,
                    outcome,
                }
            })
            .collect::<Vec<_>>();

        let mut target_counts = BTreeMap::new();
        for preview in previews
            .iter()
            .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
        {
            *target_counts
                .entry(preview.new_name.clone())
                .or_insert(0usize) += 1;
        }

        for preview in &mut previews {
            if !matches!(preview.outcome, RegexRenameOutcome::Ready) {
                continue;
            }
            if target_counts.get(&preview.new_name).copied().unwrap_or(0) > 1
                || existing_names.contains(&preview.new_name)
            {
                preview.outcome = RegexRenameOutcome::Conflict;
            }
        }

        self.pending_action = Some(PendingAction::RegexRename {
            pane_id: self.focused_pane,
            pattern: pattern.to_string(),
            replacement: replacement.to_string(),
            selected: 0,
            previews,
        });
        if let Some(action) = self.pending_action.as_ref() {
            self.status = self.status_for_pending_action(action)?;
        }
        Ok(())
    }

    /// 打開 filter 輸入框，並以目前焦點 pane 作為過濾目標。
    pub(crate) fn open_filter_input(&mut self, mode: FilterMode) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let filter = FilterState {
            pane_id: self.focused_pane,
            buffer: String::new(),
            editing: true,
            mode,
        };
        self.apply_filter_buffer(&filter);
        self.status = format_filter_status(&filter);
        self.filter = Some(filter);
    }

    /// 打開 global search 面板，遞迴建立目前目錄下的搜尋候選資料集。
    pub(crate) fn open_global_search(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = GlobalSearchState {
            pane_id: self.focused_pane,
            root_dir: pane.cwd.clone(),
            mode: SearchMode::Path,
            buffer: String::new(),
            editing: true,
            loading: false,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        };
        self.status = global_search_status(search.mode, "", 0, true, false, false);
        self.global_search = Some(search);
        self.cancel_global_search_worker();
        Ok(())
    }

    /// 打開內容搜尋面板，遞迴搜尋目前目錄下所有檔案內容。
    pub(crate) fn open_content_search(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = GlobalSearchState {
            pane_id: self.focused_pane,
            root_dir: pane.cwd.clone(),
            mode: SearchMode::Content,
            buffer: String::new(),
            editing: true,
            loading: false,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        };
        self.status = global_search_status(search.mode, "", 0, true, false, false);
        self.global_search = Some(search);
        self.cancel_global_search_worker();
        Ok(())
    }

    /// 切換目前焦點 panel 自己的 preview mode。
    ///
    /// 參數：
    /// - `self: &mut App`，包含 panel 集合與目前焦點的應用程式狀態。
    ///
    /// 回傳：`()`；只切換 `focused_pane` 對應的 `PaneState::preview_active`，其他
    /// panel 已開啟的 preview 會保持原狀。若焦點 panel 已不存在，會在狀態列顯示錯誤。
    pub(crate) fn open_preview_focus(&mut self) {
        let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return;
        };
        let preview_active = pane.toggle_preview_active();
        self.pending_g = false;
        self.pending_y = false;
        self.status = if preview_active {
            String::from("preview mode")
        } else {
            String::from("normal mode")
        };
    }

    /// 打開 preview search 輸入框，並清空上一次的搜尋字串。
    pub(crate) fn open_preview_search_input(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = PreviewSearchState {
            pane_id: self.focused_pane,
            buffer: String::new(),
            editing: true,
        };
        self.apply_preview_search_buffer(&search);
        self.status =
            preview_search_status(&search.buffer, self.preview_match_count(search.pane_id));
        self.preview_search = Some(search);
    }

    /// 打開目前 pane 的列表內 find-next 輸入框，並沿用目前已存在的查詢字串。
    pub(crate) fn open_list_find_input(&mut self) {
        self.text_input_mode = RenameMode::Insert;
        self.text_input_cursor = 0;
        let search = ListFindState {
            pane_id: self.focused_pane,
            buffer: String::new(),
        };
        self.apply_list_find_buffer(&search);
        self.status = list_find_status(&search.buffer, self.list_find_match_count(search.pane_id));
        self.list_find = Some(search);
    }

    /// 使用 `fzf` 遞迴掃描目前 pane 的目錄樹，快速挑選任意深度的目標。
    pub(crate) fn open_fzf_jump(&mut self) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("jump failed: panel not found");
            return;
        };

        if !pane.cwd.is_dir() {
            self.status = String::from("jump failed: current root is not a directory");
            return;
        }

        let root_dir = pane.cwd.clone();
        let task_id = self.push_task(
            self.focused_pane,
            "jump",
            format!("fzf jump in {}", root_dir.display()),
            String::from("waiting for fzf"),
            vec![root_dir.display().to_string()],
            None,
        );
        self.pending_fzf_jump = Some(FzfJumpRequest {
            pane_id: self.focused_pane,
            root_dir,
            show_hidden: true,
            follow_links: self.config.search.fzf_follow_links,
            task_id,
        });
        self.status = String::from("jump: fzf loading");
    }

    /// 進入 visual selection 模式，準備用移動游標的方式框選一段範圍。
    fn open_visual_selection(&mut self) -> io::Result<()> {
        let pane_id = self.focused_pane;
        let selected = self.current_pane_mut()?.selected;
        self.visual_selection = Some(VisualSelectionState {
            pane_id,
            anchor: selected,
            current: selected,
        });
        self.pending_g = false;
        self.pending_y = false;
        self.status = String::from("visual: range selection");
        Ok(())
    }

    /// 將目前 visual selection 範圍加入已標記清單，並回到一般模式。
    fn commit_visual_selection(&mut self) -> io::Result<()> {
        let Some(selection) = self.visual_selection.take() else {
            return Ok(());
        };
        let Some(pane) = self.panes.get_mut(&selection.pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let added = pane.mark_range(selection.anchor, selection.current);
        self.status = format!("marked {added} items");
        self.pending_g = false;
        Ok(())
    }

    /// 當 visual selection 模式中游標移動後，更新目前選取範圍的尾端。
    fn sync_visual_selection_cursor(&mut self) {
        if let Some(selection) = &mut self.visual_selection
            && let Some(pane) = self.panes.get(&selection.pane_id)
        {
            selection.current = pane.selected;
        }
    }

    /// 回傳 visual selection 狀態列文字，包含目前暫時框選的項目數量。
    fn visual_status_label(&self) -> String {
        match &self.visual_selection {
            Some(selection) => {
                let count = selection.anchor.abs_diff(selection.current) + 1;
                format!("visual: selecting {count} items")
            }
            None => String::from("normal mode"),
        }
    }

    /// 將目前 trash 視覺選取範圍加入已標記清單。
    fn commit_trash_visual_selection(
        &self,
        entries: &[TrashListEntry],
        marked_ids: &mut Vec<String>,
        anchor: usize,
        current: usize,
    ) -> usize {
        let start = anchor.min(current);
        let end = anchor.max(current);
        let mut added = 0usize;

        for entry in entries
            .iter()
            .skip(start)
            .take(end.saturating_sub(start) + 1)
        {
            if !marked_ids.iter().any(|id| id == &entry.id) {
                marked_ids.push(entry.id.clone());
                added += 1;
            }
        }

        added
    }

    /// 從目前可見 trash 項目中，挑出批次操作真正要套用的目標清單。
    ///
    /// 規則：
    /// - 若已經有 `V` 選到的標記，就只處理那些標記。
    /// - 若目前沒有標記，則直接以搜尋結果中的全部項目當作目標。
    fn trash_panel_batch_entries<'a>(
        &self,
        entries: &'a [TrashListEntry],
        marked_ids: &[String],
    ) -> Vec<&'a TrashListEntry> {
        if marked_ids.is_empty() {
            return entries.iter().collect();
        }

        entries
            .iter()
            .filter(|entry| marked_ids.iter().any(|id| id == &entry.id))
            .collect()
    }

    /// 為一批 trash 項目產生確認視窗要顯示的名稱摘要。
    fn trash_confirm_target_name(entries: &[&TrashListEntry]) -> String {
        if entries.len() == 1 {
            entries[0].display_name.clone()
        } else {
            format!("{} items", entries.len())
        }
    }

    /// 回傳 trash 視覺選取狀態列文字。
    fn trash_visual_status_label(
        &self,
        selected: usize,
        anchor: usize,
        marked_count: usize,
    ) -> String {
        let range_count = anchor.abs_diff(selected) + 1;
        if marked_count == 0 {
            format!("trash visual: selecting {range_count} items")
        } else {
            format!("trash visual: selecting {range_count} items ({marked_count} marked)")
        }
    }

    /// 為目前 `trash` 面板挑出還原目標，並進入確認視窗。
    ///
    /// 規則：
    /// - 若已有 `V` 標記，`u` 會直接還原全部標記項目。
    /// - 若沒有標記，`u` 才會只還原游標所在的單筆項目。
    fn start_trash_panel_restore_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if !marked_ids.is_empty() {
            if selected_entries.is_empty() {
                self.status = String::from("trash is empty");
                return Ok(());
            }
            let target_name = Self::trash_confirm_target_name(&selected_entries);
            let entry_count = selected_entries.len();
            let action = TrashConfirmAction::RestoreFromPanel {
                pane_id,
                target_ids: selected_entries
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect(),
                search,
                selected,
            };
            self.pending_action = Some(PendingAction::ConfirmTrashAction {
                action: action.clone(),
                target_name: target_name.clone(),
                entry_count,
                marked_ids: marked_ids.to_vec(),
                visual_anchor,
            });
            self.status = trash_confirm_status(&action, &target_name, entry_count);
            return Ok(());
        }

        let Some(entry) = entries.get(selected) else {
            self.status = String::from("trash is empty");
            return Ok(());
        };
        let action = TrashConfirmAction::RestoreFromPanel {
            pane_id,
            target_ids: vec![entry.id.clone()],
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: entry.display_name.clone(),
            entry_count: 1,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &entry.display_name, 1);
        Ok(())
    }

    /// 為目前 `trash` 面板挑出批次還原目標，並進入確認視窗。
    ///
    /// 若已有 `V` 標記，會只操作標記的項目；否則會操作目前可見結果的全部項目。
    fn start_trash_panel_restore_all_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if selected_entries.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }
        let target_name = Self::trash_confirm_target_name(&selected_entries);
        let entry_count = selected_entries.len();
        let action = TrashConfirmAction::RestoreFromPanel {
            pane_id,
            target_ids: selected_entries
                .iter()
                .map(|entry| entry.id.clone())
                .collect(),
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: target_name.clone(),
            entry_count,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &target_name, entry_count);
        Ok(())
    }

    /// 為目前 `trash` 面板挑出永久刪除目標，並進入確認視窗。
    ///
    /// 規則：
    /// - 若已有 `V` 標記，`d` 會直接刪除全部標記項目。
    /// - 若沒有標記，`d` 才會只刪除游標所在的單筆項目。
    fn start_trash_panel_delete_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if !marked_ids.is_empty() {
            if selected_entries.is_empty() {
                self.status = String::from("trash is empty");
                return Ok(());
            }
            let target_name = Self::trash_confirm_target_name(&selected_entries);
            let entry_count = selected_entries.len();
            let action = TrashConfirmAction::DeleteFromPanel {
                pane_id,
                target_ids: selected_entries
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect(),
                search,
                selected,
            };
            self.pending_action = Some(PendingAction::ConfirmTrashAction {
                action: action.clone(),
                target_name: target_name.clone(),
                entry_count,
                marked_ids: marked_ids.to_vec(),
                visual_anchor,
            });
            self.status = trash_confirm_status(&action, &target_name, entry_count);
            return Ok(());
        }

        let Some(entry) = entries.get(selected) else {
            self.status = String::from("trash is empty");
            return Ok(());
        };
        let action = TrashConfirmAction::DeleteFromPanel {
            pane_id,
            target_ids: vec![entry.id.clone()],
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: entry.display_name.clone(),
            entry_count: 1,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &entry.display_name, 1);
        Ok(())
    }

    /// 為目前 `trash` 面板挑出批次永久刪除目標，並進入確認視窗。
    ///
    /// 若已有 `V` 標記，會只刪除標記項目；否則會刪除目前搜尋結果中的全部項目。
    fn start_trash_panel_delete_all_confirmation(
        &mut self,
        pane_id: usize,
        entries: &[TrashListEntry],
        selected: usize,
        search: PanelSearchState,
        marked_ids: &[String],
        visual_anchor: Option<usize>,
    ) -> io::Result<()> {
        let selected_entries = self.trash_panel_batch_entries(entries, marked_ids);
        if selected_entries.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }
        let target_name = Self::trash_confirm_target_name(&selected_entries);
        let entry_count = selected_entries.len();
        let action = TrashConfirmAction::DeleteFromPanel {
            pane_id,
            target_ids: selected_entries
                .iter()
                .map(|entry| entry.id.clone())
                .collect(),
            search,
            selected,
        };
        self.pending_action = Some(PendingAction::ConfirmTrashAction {
            action: action.clone(),
            target_name: target_name.clone(),
            entry_count,
            marked_ids: marked_ids.to_vec(),
            visual_anchor,
        });
        self.status = trash_confirm_status(&action, &target_name, entry_count);
        Ok(())
    }

    /// 清除目前焦點 pane 中所有標記。
    fn clear_marks_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let count = pane.marked_count();
        pane.clear_marks();
        self.status = if count == 0 {
            String::from("no marks to clear")
        } else {
            format!("cleared {count} marks")
        };
        Ok(())
    }

    /// 將目前焦點 pane 中所有可見項目全部標記，方便後續做批次操作。
    fn mark_all_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let added = pane.mark_all_visible();
        let total = pane.marked_count();
        self.status = if total == 0 {
            String::from("nothing to mark")
        } else {
            format!("marked all visible items (+{added}, total {total})")
        };
        Ok(())
    }

    /// 切換目前焦點項目的標記狀態，讓單項多選操作更直接。
    fn toggle_mark_selected_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let selected_name = pane.selected_entry().map(|entry| entry.display_name());
        match pane.toggle_mark_selected() {
            Some(true) => {
                let name = selected_name.unwrap_or_else(|| String::from("item"));
                self.status = format!("marked {name}");
            }
            Some(false) => {
                let name = selected_name.unwrap_or_else(|| String::from("item"));
                self.status = format!("unmarked {name}");
            }
            None => {
                self.status = String::from("nothing selected to mark");
            }
        }
        Ok(())
    }

    /// 反轉目前焦點 pane 所有可見項目的標記狀態。
    fn invert_marks_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let (added, removed) = pane.invert_visible_marks();
        let total = pane.marked_count();
        self.status = if added == 0 && removed == 0 {
            String::from("nothing to invert")
        } else {
            format!("inverted visible marks (+{added}, -{removed}, total {total})")
        };
        Ok(())
    }

    /// 判斷目前是否仍有任何 pane 保留已提交的標記。
    fn has_any_marks(&self) -> bool {
        self.panes.values().any(|pane| pane.marked_count() > 0)
    }

    /// 清掉所有 pane 的已提交標記，讓整個畫面回到一般模式。
    fn clear_all_marks(&mut self) {
        let mut cleared = 0usize;
        for pane in self.panes.values_mut() {
            cleared += pane.marked_count();
            pane.clear_marks();
        }

        self.pending_g = false;
        self.pending_y = false;
        self.status = if cleared == 0 {
            String::from("normal mode")
        } else {
            format!("cleared {cleared} marks")
        };
    }

    /// 建立新的 task 紀錄並加入 task log，回傳這筆任務的 id。
    ///
    /// 參數：
    /// - `pane_id: usize`：啟動工作的 panel 編號。
    /// - `kind: &'static str`：供取消、搜尋與診斷使用的穩定工作種類。
    /// - `title: String`：使用者可閱讀的操作說明，例如 `copy 2 item(s)`。
    /// - `detail: String`：執行中說明，完成時可由 [`Self::finish_task`] 改成結果或錯誤。
    /// - `source_locations: Vec<String>`：工作實際讀取或修改的所有來源路徑／URI。
    /// - `destination_location: Option<String>`：工作寫入或跳轉的目的位置。
    ///
    /// 回傳：`usize`，新 task 的唯一 id。來源與目的地會獨立持久化，因此即使完成時
    /// `detail` 被結果覆寫，task 面板仍能完整說明檔案操作方向。
    fn push_task(
        &mut self,
        pane_id: usize,
        kind: &'static str,
        title: String,
        detail: String,
        source_locations: Vec<String>,
        destination_location: Option<String>,
    ) -> usize {
        let id = self.next_task_id;
        self.next_task_id += 1;
        self.task_log.push(TaskRecord {
            id,
            pane_id,
            kind: kind.to_string(),
            title,
            detail,
            source_locations,
            destination_location,
            state: TaskState::Running,
            progress_percent: None,
            completed_bytes: None,
            total_bytes: None,
            started_at_unix_ms: unix_time_ms_now(),
            finished_at_unix_ms: None,
        });
        if self.task_log.len() > 200 {
            let overflow = self.task_log.len() - 200;
            self.task_log.drain(0..overflow);
        }
        self.persist_task_history_best_effort();
        id
    }

    /// 更新指定 task 的最終狀態與說明文字。
    fn finish_task(&mut self, task_id: usize, state: TaskState, detail: String) {
        let mut changed = false;
        if let Some(task) = self.task_log.iter_mut().find(|task| task.id == task_id) {
            task.state = state;
            task.detail = detail;
            if state == TaskState::Done && task.progress_percent.is_some() {
                task.progress_percent = Some(100);
            }
            if state == TaskState::Done
                && let Some(total_bytes) = task.total_bytes
            {
                task.completed_bytes = Some(total_bytes.max(task.completed_bytes.unwrap_or(0)));
                task.total_bytes = task.completed_bytes;
            }
            task.finished_at_unix_ms = Some(unix_time_ms_now());
            changed = true;
        }
        if changed {
            self.persist_task_history_best_effort();
        }
    }

    /// 更新背景檔案工作的原始 byte 進度，並允許走訪期間依新發現的總量校正。
    ///
    /// 參數：`task_id: usize` 為 task 編號；`completed_bytes: u64` 為已完成量；
    /// `total_bytes: u64` 為預估總量。
    /// 回傳：`() `；找不到 task 或總量為零時不修改狀態。
    fn update_task_progress(&mut self, task_id: usize, completed_bytes: u64, total_bytes: u64) {
        let mut changed = false;
        if let Some(task) = self.task_log.iter_mut().find(|task| task.id == task_id) {
            let total_bytes = total_bytes.max(completed_bytes);
            if task.completed_bytes != Some(completed_bytes)
                || task.total_bytes != Some(total_bytes)
            {
                task.completed_bytes = Some(completed_bytes);
                task.total_bytes = Some(total_bytes);
                task.progress_percent = if total_bytes == 0 {
                    Some(0)
                } else {
                    Some(
                        completed_bytes
                            .saturating_mul(100)
                            .checked_div(total_bytes)
                            .unwrap_or(0)
                            .min(99) as u8,
                    )
                };
                changed = true;
            }
        }
        if changed {
            self.persist_task_history_best_effort();
        }
    }

    /// 立即保存目前 task 歷史；失敗時保留應用程式運作並把原因顯示在狀態列。
    ///
    /// 參數：無。
    /// 回傳：`() `。持久化錯誤不應讓進行中的檔案工作崩潰，但必須讓使用者知道
    /// 關閉後可能看不到最新歷史。
    fn persist_task_history_best_effort(&mut self) {
        if let Err(error) = save_task_history(&self.task_history_path, &self.task_log) {
            self.status = format!("task history save failed: {error}");
        }
    }

    /// 執行正常關閉前的 task 收尾與持久化。
    ///
    /// 尚在背景 thread 執行的工作不能跨 process 真正暫停；PaneFM 會把它們標記為
    /// `Interrupted`，保留當下 byte 進度與診斷資料，但不在下次啟動時自動覆寫目的地。
    /// 即使程式被強制關閉、來不及執行本函數，啟動載入也會把磁碟上的 `Running`
    /// 紀錄轉成 `Interrupted`。
    ///
    /// 參數：無。
    /// 回傳：`io::Result<()>`，確保離開主迴圈前最後一份歷史已寫入磁碟。
    pub(crate) fn prepare_for_shutdown(&mut self) -> io::Result<()> {
        let finished_at = unix_time_ms_now();
        for task in &mut self.task_log {
            if task.state == TaskState::Running {
                task.state = TaskState::Interrupted;
                task.finished_at_unix_ms = Some(finished_at);
                task.detail = format!("{}; interrupted when PaneFM closed", task.detail);
            }
        }
        save_task_history(&self.task_history_path, &self.task_log)
    }

    /// 取消指定 task；目前支援 search worker 與尚未執行的 queued open / fzf jump。
    fn cancel_task_by_id(&mut self, task_id: usize) {
        if self.active_global_search_task_id == Some(task_id) {
            self.cancel_global_search();
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if self.active_network_goto_task_id == Some(task_id) {
            self.cancel_network_goto("cancelled from task panel");
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if self
            .pending_fzf_jump
            .as_ref()
            .map(|request| request.task_id)
            == Some(task_id)
        {
            self.pending_fzf_jump = None;
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("cancelled before fzf"),
            );
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if self.pending_launch.as_ref().map(|queued| queued.task_id) == Some(task_id) {
            self.pending_launch = None;
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("cancelled before launch"),
            );
            self.status = format!("cancelled task {task_id}");
            return;
        }

        if let Some(task) = self.task_log.iter().find(|task| task.id == task_id) {
            if matches!(task.state, TaskState::Running) {
                self.status = format!("task {task_id} cannot be cancelled now");
            } else {
                self.status = format!("task {task_id} is already {}", task_state_label(task.state));
            }
        } else {
            self.status = format!("task {task_id} not found");
        }
    }

    /// 取消目前 panel 中所有仍可取消的背景任務。
    ///
    /// 參數：
    /// - `pane_id: usize`，要清理任務的 panel 編號。
    ///
    /// 回傳：`usize`，實際送出取消要求的任務數量。
    fn cancel_running_tasks_for_pane(&mut self, pane_id: usize) -> usize {
        let task_ids = self
            .task_log
            .iter()
            .filter(|task| task.pane_id == pane_id && matches!(task.state, TaskState::Running))
            .map(|task| task.id)
            .collect::<Vec<_>>();

        let mut cancelled = 0;
        for task_id in task_ids {
            let was_running = self
                .task_log
                .iter()
                .any(|task| task.id == task_id && matches!(task.state, TaskState::Running));
            self.cancel_task_by_id(task_id);
            if was_running
                && self
                    .task_log
                    .iter()
                    .any(|task| task.id == task_id && matches!(task.state, TaskState::Cancelled))
            {
                cancelled += 1;
            }
        }
        cancelled
    }

    /// 取得目前 pane 對應的任務清單，最新的排在最上面。
    fn tasks_for_pane(&self, pane_id: usize) -> Vec<TaskRecord> {
        self.task_log
            .iter()
            .filter(|task| task.pane_id == pane_id)
            .cloned()
            .rev()
            .collect()
    }

    /// 將目前選取或標記的項目壓成單一 zip 檔，並在完成後刷新所有 pane。
    fn compress_selected_entries(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("nothing selected to compress");
            return Ok(());
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to compress");
            return Ok(());
        }

        let target_dir = pane.cwd.clone();
        if entries_should_run_in_background(&entries) {
            return self.start_background_compress(self.focused_pane, target_dir, entries);
        }
        let archive_path = compress_entries_to_zip(&target_dir, &entries)?;
        self.reload_all_panes()?;
        let _ = self.reveal_path_and_track(self.focused_pane, &archive_path);

        self.status = if entries.len() == 1 {
            format!(
                "compressed {} -> {}",
                entries[0].display_name(),
                archive_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive.zip")
            )
        } else {
            format!(
                "compressed {} items -> {}",
                entries.len(),
                archive_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("archive.zip")
            )
        };
        Ok(())
    }

    /// 解開目前選取或標記的壓縮檔，並盡量把游標帶到第一個輸出結果。
    fn extract_selected_archives(&mut self) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("nothing selected to extract");
            return Ok(());
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to extract");
            return Ok(());
        }

        let target_dir = pane.cwd.clone();
        if entries_should_run_in_background(&entries) {
            return self.start_background_extract(self.focused_pane, target_dir, entries);
        }
        let (extracted, skipped) = extract_entries(&target_dir, &entries)?;
        if extracted.is_empty() {
            self.status = if skipped == 0 {
                String::from("nothing selected to extract")
            } else {
                format!("no supported archives selected (skipped {skipped})")
            };
            return Ok(());
        }

        self.reload_all_panes()?;
        self.reveal_first_extracted_output(&extracted)?;

        self.status = extraction_status_label(&extracted, skipped);
        Ok(())
    }

    /// 把大型壓縮工作排入背景執行，避免 ZIP deflate 長時間占住 TUI 主執行緒。
    ///
    /// 參數：`pane_id: usize` 為來源 panel；`target_dir: PathBuf` 為輸出目錄；
    /// `entries: Vec<FileEntry>` 為本次要壓縮的項目。
    /// 回傳：`io::Result<()>`；成功代表工作已排入 task manager。
    fn start_background_compress(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        entries: Vec<super::entry::FileEntry>,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let first_name = entries
            .first()
            .map(|entry| entry.display_name())
            .unwrap_or_else(|| String::from("item"));
        let task_id = self.push_task(
            pane_id,
            "compress",
            format!("compress {entry_count} item(s)"),
            format!("output: {}", target_dir.display()),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let busy_paths = entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
        self.active_file_job_busy_paths.insert(task_id, busy_paths);
        let (sender, receiver) = std::sync::mpsc::channel();
        thread::spawn(move || {
            // 計算大型目錄的內容總量也會遞迴碰觸檔案系統，必須跟壓縮本體一起留在
            // worker，否則「已使用背景壓縮」仍會在按下 C 時先凍結 TUI。
            let total_bytes = entries
                .iter()
                .filter_map(|entry| path_content_size(&entry.path).ok())
                .fold(0u64, u64::saturating_add);
            let _ = sender.send(FileJobEvent::Progress {
                task_id,
                completed_bytes: 0,
                total_bytes,
            });
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result =
                compress_entries_to_zip_with_progress(&target_dir, &entries, &mut progress);
            let _ = sender.send(FileJobEvent::Compress {
                task_id,
                pane_id,
                entry_count,
                first_name,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("compressing {entry_count} item(s) in background [task {task_id}]");
        Ok(())
    }

    /// 把大型解壓工作排入背景執行，讓使用者可在其他 panel 繼續操作。
    ///
    /// 參數：`pane_id: usize` 為來源 panel；`target_dir: PathBuf` 為輸出目錄；
    /// `entries: Vec<FileEntry>` 為選取的壓縮檔。
    /// 回傳：`io::Result<()>`；成功代表工作已排入 task manager。
    fn start_background_extract(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        entries: Vec<super::entry::FileEntry>,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let task_id = self.push_task(
            pane_id,
            "extract",
            format!("extract {entry_count} item(s)"),
            format!("output: {}", target_dir.display()),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let busy_paths = vec![target_dir.clone()];
        self.active_file_job_busy_paths.insert(task_id, busy_paths);
        let (sender, receiver) = mpsc::channel();
        // 解壓後資料通常比壓縮檔大，因此這是估算分母；完成事件會校正成最終 byte。
        let total_bytes = entries
            .iter()
            .map(|entry| entry.size)
            .fold(0u64, u64::saturating_add);
        self.update_task_progress(task_id, 0, total_bytes);
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result = extract_entries_with_progress(&target_dir, &entries, &mut progress);
            let _ = sender.send(FileJobEvent::Extract {
                task_id,
                pane_id,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("extracting {entry_count} item(s) in background [task {task_id}]");
        Ok(())
    }

    /// 開始刪除確認流程，建立一個待確認的刪除互動。
    pub(crate) fn start_delete_confirmation(&mut self, permanent: bool) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = if permanent {
                String::from("nothing selected to delete")
            } else {
                String::from("nothing selected to trash")
            };
            return;
        };

        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = if permanent {
                String::from("nothing selected to delete")
            } else {
                String::from("nothing selected to trash")
            };
            return;
        }

        let target_name = if entries.len() == 1 {
            entries[0].display_name()
        } else {
            format!("{} items", entries.len())
        };

        self.pending_action = Some(PendingAction::ConfirmDelete {
            pane_id: self.focused_pane,
            target_name: target_name.clone(),
            permanent,
        });
        self.status = if permanent {
            format!("confirm delete {target_name}: y/n")
        } else {
            format!("confirm trash {target_name}: y/n")
        };
    }

    /// 把永久刪除工作排入背景 task 執行，避免刪除大型目錄（數萬筆檔案）時凍結主 UI 執行緒。
    fn start_background_delete(
        &mut self,
        pane_id: usize,
        entries: Vec<super::entry::FileEntry>,
        target_name: String,
    ) -> io::Result<()> {
        let entry_count = entries.len();
        let task_id = self.push_task(
            pane_id,
            "delete",
            format!("delete {entry_count} item(s)"),
            format!("target: {target_name}"),
            entries
                .iter()
                .map(|entry| entry.path.display().to_string())
                .collect(),
            None,
        );
        let mut busy_paths = Vec::new();
        let mut delete_targets = Vec::new();
        let deleted_names = entries.iter().map(|e| e.display_name()).collect::<Vec<_>>();
        let pid = std::process::id();
        let now_ms = unix_time_ms_now();

        for (idx, entry) in entries.iter().enumerate() {
            busy_paths.push(entry.path.clone());
            if entry.is_dir {
                if let Some(parent) = entry.path.parent() {
                    let staging_path =
                        parent.join(format!(".panefm-del-{pid}-{task_id}-{idx}-{now_ms}"));
                    if fs::rename(&entry.path, &staging_path).is_ok() {
                        busy_paths.push(staging_path.clone());
                        delete_targets.push((staging_path, entry.path.clone(), true));
                        continue;
                    }
                }
            }
            delete_targets.push((entry.path.clone(), entry.path.clone(), entry.is_dir));
        }
        self.active_file_job_busy_paths.insert(task_id, busy_paths);

        // 資料夾移入暫存刪除路徑後，立即重載面板讓使用者看到目錄已瞬間移除
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.marked_paths.clear();
            let _ = pane.reload();
        }

        let total_bytes = entries
            .iter()
            .map(|e| e.directory_size.unwrap_or(e.size))
            .fold(0u64, u64::saturating_add);
        self.update_task_progress(task_id, 0, total_bytes.max(1));
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let mut completed_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now();
            let mut progress = |increment: u64| {
                completed_bytes = completed_bytes.saturating_add(increment);
                if last_progress_update.elapsed() >= Duration::from_millis(200) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes.max(completed_bytes),
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };

            let mut failed_error = None;
            for (path, original_path, is_dir) in delete_targets {
                let res = if is_dir {
                    remove_dir_all_parallel_with_progress(&path, &mut progress)
                } else {
                    match remove_file_or_symlink_with_retry(&path) {
                        Ok(size) => {
                            progress(size);
                            Ok(())
                        }
                        Err(err) => Err(err),
                    }
                };
                if let Err(error) = res {
                    // 若失敗且曾改名為暫存檔，且原路徑目前不存在，嘗試改名還原讓使用者在目錄看見
                    if path != original_path && path.exists() && !original_path.exists() {
                        let _ = fs::rename(&path, &original_path);
                    }
                    failed_error = Some(error);
                    break;
                }
            }
            send_progress_if_changed(
                &progress_sender,
                task_id,
                completed_bytes,
                total_bytes.max(completed_bytes),
                &mut last_progress,
            );
            let result = match failed_error {
                Some(error) => Err(error),
                None => Ok(deleted_names),
            };
            let _ = sender.send(FileJobEvent::Delete {
                task_id,
                target_name,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!("deleting {entry_count} item(s) in background [task #{task_id}]");
        Ok(())
    }

    /// 真正執行將目前待確認項目移到 trash 或永久刪除的檔案系統操作。
    pub(crate) fn confirm_delete(
        &mut self,
        pane_id: usize,
        target_name: &str,
        permanent: bool,
    ) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        if permanent {
            let entries = {
                let pane = self
                    .panes
                    .get_mut(&pane_id)
                    .expect("checked pane existence before delete");
                pane.selected_or_marked_entries()
            };
            if entries.is_empty() {
                self.status = String::from("nothing selected to delete");
                return Ok(());
            }
            self.start_background_delete(pane_id, entries, target_name.to_string())?;
        } else {
            let trash_store = self.trash_store.clone();
            let trash_result = {
                let pane = self
                    .panes
                    .get_mut(&pane_id)
                    .expect("checked pane existence before trash");
                pane.trash_selected_or_marked(&trash_store)
            };
            match trash_result {
                Ok(trashed_names) if trashed_names.is_empty() => {
                    self.status = String::from("nothing selected to trash");
                }
                Ok(trashed_names) if trashed_names.len() == 1 => {
                    self.reload_all_panes()?;
                    self.status = format!("trashed {}", trashed_names[0]);
                }
                Ok(trashed_names) => {
                    self.reload_all_panes()?;
                    self.status = format!("trashed {} items", trashed_names.len());
                }
                Err(error) => self.status = format!("failed to trash {target_name}: {error}"),
            }
        }

        Ok(())
    }

    /// 還原最近一次放進 trash 的項目，並盡量在目前 pane 對焦到還原結果。
    pub(crate) fn restore_latest_from_trash(&mut self) -> io::Result<()> {
        match self.trash_store.restore_latest()? {
            Some(result) => {
                self.reload_all_panes()?;
                let _ = self.reveal_path_and_track(self.focused_pane, &result.restored_path);
                self.status = format!("restored {}", result.display_name);
            }
            None => {
                self.status = String::from("trash is empty");
            }
        }
        Ok(())
    }

    /// 執行使用者在確認視窗中同意的 trash 操作。
    fn confirm_trash_action(
        &mut self,
        action: TrashConfirmAction,
        target_name: String,
        entry_count: usize,
    ) -> io::Result<()> {
        match action {
            TrashConfirmAction::RestoreFromPanel {
                pane_id,
                target_ids,
                search,
                selected,
            } => self.restore_trash_ids_in_panel(
                pane_id,
                &target_ids,
                search,
                selected,
                &target_name,
                entry_count,
            ),
            TrashConfirmAction::DeleteFromPanel {
                pane_id,
                target_ids,
                search,
                selected,
            } => self.delete_trash_ids_in_panel(
                pane_id,
                &target_ids,
                search,
                selected,
                &target_name,
                entry_count,
            ),
        }
    }

    /// 在 trash 面板中批次還原指定 id 清單，並盡量保留原本搜尋上下文。
    fn restore_trash_ids_in_panel(
        &mut self,
        pane_id: usize,
        target_ids: &[String],
        search: PanelSearchState,
        selected: usize,
        target_name: &str,
        entry_count: usize,
    ) -> io::Result<()> {
        if target_ids.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }

        let results = self.trash_store.restore_many_by_ids(target_ids)?;
        self.reload_all_panes()?;
        if let Some(first) = results.first() {
            let _ = self.reveal_path_and_track(pane_id, &first.restored_path);
        }
        self.reopen_trash_panel_after_mutation(pane_id, search, selected)?;
        if results.is_empty() {
            self.status = format!("trash item no longer exists: {target_name}");
        } else if entry_count <= 1 {
            self.status = format!("restored {target_name}");
        } else {
            self.status = format!("restored {} items", results.len());
        }
        Ok(())
    }

    /// 在 trash 面板中永久刪除指定 id 清單，並保留目前搜尋上下文。
    fn delete_trash_ids_in_panel(
        &mut self,
        pane_id: usize,
        target_ids: &[String],
        search: PanelSearchState,
        selected: usize,
        target_name: &str,
        entry_count: usize,
    ) -> io::Result<()> {
        if target_ids.is_empty() {
            self.status = String::from("trash is empty");
            return Ok(());
        }

        let deleted_names = self.trash_store.delete_many_by_ids(target_ids)?;
        self.reopen_trash_panel_after_mutation(pane_id, search, selected)?;
        if deleted_names.is_empty() {
            self.status = format!("trash item no longer exists: {target_name}");
        } else if entry_count <= 1 {
            self.status = format!("deleted permanently {target_name}");
        } else {
            self.status = format!("deleted permanently {} items", deleted_names.len());
        }
        Ok(())
    }

    /// 在 trash 異動完成後重建面板狀態，避免游標跳到錯誤位置。
    fn reopen_trash_panel_after_mutation(
        &mut self,
        pane_id: usize,
        search: PanelSearchState,
        selected: usize,
    ) -> io::Result<()> {
        let visible_count = trash_panel_entries(&self.trash_store, &search.buffer)?.len();
        let next_selected = if visible_count == 0 {
            0
        } else {
            selected.min(visible_count.saturating_sub(1))
        };
        self.pending_action = Some(PendingAction::TrashPanel {
            pane_id,
            selected: next_selected,
            search,
            marked_ids: Vec::new(),
            visual_anchor: None,
        });
        Ok(())
    }

    /// 執行 help 面板中選到的功能，直接跳到對應模式或命令。
    fn execute_help_entry(&mut self, entries: &[HelpEntry], selected: usize) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("help: no command selected");
            return Ok(());
        };

        self.pending_action = None;
        let mut should_restore_help_return = true;
        match entry.action {
            HelpAction::Command(command) => {
                if command.ends_with(' ') {
                    self.open_prefilled_command(command);
                    should_restore_help_return = false;
                } else {
                    self.execute_command(command)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                }
            }
            HelpAction::Delete => self.start_delete_confirmation(false),
            HelpAction::Filter => self.open_filter_input(FilterMode::Normal),
            HelpAction::FuzzyFilter => self.open_filter_input(FilterMode::Fuzzy),
            HelpAction::Sort => self.open_sort_picker(),
            HelpAction::Hidden => {
                self.toggle_hidden_files()?;
            }
            HelpAction::Visual => {
                self.open_visual_selection()?;
            }
            HelpAction::QuitHint => {
                self.status = String::from("use q in normal mode to quit");
            }
        }

        if should_restore_help_return && self.pending_action.is_none() && self.help_return.is_some()
        {
            self.restore_help_return_state(true)?;
        }
        Ok(())
    }

    /// 真正執行重新命名目前待確認項目的檔案系統操作。
    pub(crate) fn confirm_rename(
        &mut self,
        pane_id: usize,
        original_name: &str,
        new_name: &str,
    ) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let rename_result = {
            let pane = self
                .panes
                .get_mut(&pane_id)
                .expect("checked pane existence before rename");
            pane.rename_selected(new_name)
        };

        match rename_result {
            Ok(Some(renamed_name)) => {
                self.reload_all_panes()?;
                self.status = format!("renamed {original_name} -> {renamed_name}");
            }
            Ok(None) => {
                self.status = String::from("nothing selected to rename");
            }
            Err(error) => {
                self.status = format!("failed to rename {original_name}: {error}");
            }
        }

        Ok(())
    }

    /// 真正執行 regex 批次改名預覽中所有可套用的項目。
    ///
    /// 參數：
    /// - `pane_id: usize`，發起這次批次改名的 pane。
    /// - `previews: &[RegexRenamePreview]`，先前建立好的預覽結果。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn apply_regex_rename_preview(
        &mut self,
        pane_id: usize,
        previews: &[RegexRenamePreview],
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let cwd = pane.cwd.clone();
        let ready = previews
            .iter()
            .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
            .cloned()
            .collect::<Vec<_>>();

        if previews.iter().any(|preview| {
            matches!(
                preview.outcome,
                RegexRenameOutcome::Conflict | RegexRenameOutcome::Invalid
            )
        }) {
            self.status = String::from("rename-regex: resolve conflicts before apply");
            return Ok(());
        }
        if ready.is_empty() {
            self.status = String::from("rename-regex: nothing to apply");
            return Ok(());
        }

        let staged = ready
            .iter()
            .enumerate()
            .map(|(index, preview)| {
                let temp_path =
                    unique_regex_rename_temp_path(&cwd, &preview.original_name, index, &ready);
                (
                    preview.source_path.clone(),
                    temp_path,
                    cwd.join(&preview.new_name),
                )
            })
            .collect::<Vec<_>>();

        for (source_path, temp_path, _) in &staged {
            fs::rename(source_path, temp_path)?;
        }

        for (_, temp_path, final_path) in &staged {
            if let Err(error) = fs::rename(temp_path, final_path) {
                self.status = format!("rename-regex failed: {error}");
                return Ok(());
            }
        }

        self.reload_all_panes()?;
        self.status = if ready.len() == 1 {
            String::from("rename-regex: renamed 1 item")
        } else {
            format!("rename-regex: renamed {} items", ready.len())
        };
        Ok(())
    }

    /// 真正執行建立新項目的檔案系統操作。
    ///
    /// 參數：
    /// - `pane_id: usize`，要建立項目的目標 pane。
    /// - `path: &str`，新項目的相對路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn confirm_create_entry(&mut self, pane_id: usize, path: &str) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let create_result = {
            let pane = self
                .panes
                .get_mut(&pane_id)
                .expect("checked pane existence before create");
            pane.create_entry(path)
        };

        match create_result {
            Ok(created_name) => {
                self.reload_all_panes()?;
                let item_type = if created_name.ends_with('/') {
                    "directory"
                } else {
                    "file"
                };
                self.status = format!("created {item_type}: {created_name}");
            }
            Err(error) => {
                self.status = format!("failed to create entry: {error}");
            }
        }

        Ok(())
    }

    /// 讓命令模式可以直接建立新項目，而不必再開啟 inline 輸入框。
    ///
    /// 參數：
    /// - `path: &str`，命令列中指定的新路徑。
    ///
    /// 回傳：`io::Result<()>`。
    fn create_entry_from_command(&mut self, path: &str) -> io::Result<()> {
        self.confirm_create_entry(self.focused_pane, path)
    }

    /// 將 filter 文字套用到指定 pane。
    fn apply_filter_buffer(&mut self, filter: &FilterState) {
        if let Some(pane) = self.panes.get_mut(&filter.pane_id) {
            pane.set_filter_query(&filter.buffer, filter.mode);
        }
    }

    /// 判斷目前畫面上是否有正在輸入中的文字框。
    fn is_text_input_active(&self) -> bool {
        if self.command_mode {
            return true;
        }
        if self.filter.as_ref().is_some_and(|filter| filter.editing) {
            return true;
        }
        if self
            .preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
        {
            return true;
        }
        if self.list_find.is_some() {
            return true;
        }
        if self
            .global_search
            .as_ref()
            .is_some_and(|s| s.editing || s.filter.editing)
        {
            return true;
        }
        matches!(
            self.pending_action,
            Some(
                PendingAction::Rename { .. }
                    | PendingAction::CreateEntry { .. }
                    | PendingAction::RegexRename { .. }
                    | PendingAction::TrashPanel {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::HelpPanel {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::TaskPanel {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::BookmarkList {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
                    | PendingAction::ZoxideList {
                        search: PanelSearchState { editing: true, .. },
                        ..
                    }
            )
        )
    }

    /// 將 preview search 文字套用到指定 pane，並讓 preview 跳到命中位置。
    fn apply_preview_search_buffer(&mut self, search: &PreviewSearchState) {
        if let Some(pane) = self.panes.get_mut(&search.pane_id) {
            pane.set_preview_search_query(&search.buffer);
        }
    }

    /// 將列表內 find-next 文字套用到指定 pane，並把游標移到第一個命中項目。
    fn apply_list_find_buffer(&mut self, search: &ListFindState) {
        if let Some(pane) = self.panes.get_mut(&search.pane_id) {
            pane.set_list_find_query(&search.buffer);
            if pane.has_list_find() {
                if let Some(target) = pane.list_find_match_indices().first().copied() {
                    pane.selected = target;
                    pane.list_state.select(Some(target));
                    pane.preview_scroll = 0;
                }
            }
        }
    }

    /// 啟動一個背景 global search 工作，避免在大型目錄中阻塞主介面。
    fn start_global_search(&mut self, search: &mut GlobalSearchState) -> io::Result<()> {
        if let Some(task_id) = search.task_id.take() {
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("replaced by new query"),
            );
        }
        self.cancel_global_search_worker();

        let pane_id = search.pane_id;
        let root_dir = search.root_dir.clone();
        let mode = search.mode;
        let query = search.buffer.clone();
        let show_hidden = self
            .panes
            .get(&pane_id)
            .map(|pane| pane.show_hidden)
            .unwrap_or(false);
        let limit = self.config.search.global_search_limit;
        let chunk_size = self.config.search.global_search_chunk_size;

        let (tx, rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let task_id = self.push_task(
            pane_id,
            "search",
            format!(
                "{}: {}",
                mode.status_label(),
                if query.is_empty() { "<all>" } else { &query }
            ),
            format!("root: {}", root_dir.display()),
            vec![root_dir.display().to_string()],
            None,
        );
        thread::spawn(move || {
            match mode {
                SearchMode::Path => stream_search_entries(
                    pane_id,
                    &root_dir,
                    show_hidden,
                    &query,
                    limit,
                    chunk_size,
                    worker_cancelled,
                    tx,
                ),
                SearchMode::Content => stream_content_search_entries(
                    pane_id,
                    &root_dir,
                    show_hidden,
                    &query,
                    limit,
                    chunk_size,
                    worker_cancelled,
                    tx,
                ),
            };
        });

        search.loading = true;
        search.searched = false;
        search.selected = 0;
        search.results.clear();
        search.task_id = Some(task_id);
        self.global_search_rx = Some(rx);
        self.global_search_cancelled = Some(cancelled);
        self.active_global_search_task_id = Some(task_id);
        Ok(())
    }

    /// 要求目前的 global search 背景工作停止，避免使用者離開畫面後仍持續掃描。
    fn cancel_global_search_worker(&mut self) {
        if let Some(cancelled) = self.global_search_cancelled.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        self.global_search_rx = None;
    }

    /// 關閉 global search 畫面，並同步停止正在進行中的背景搜尋。
    fn cancel_global_search(&mut self) {
        let mut cancelled_task_id = None;
        if let Some(search) = &mut self.global_search
            && let Some(task_id) = search.task_id.take()
        {
            cancelled_task_id = Some(task_id);
            self.finish_task(task_id, TaskState::Cancelled, String::from("cancelled"));
        }
        if let Some(task_id) = self.active_global_search_task_id.take()
            && cancelled_task_id != Some(task_id)
        {
            self.finish_task(task_id, TaskState::Cancelled, String::from("cancelled"));
        }
        self.cancel_global_search_worker();
        self.global_search = None;
        self.status = String::from("normal mode");
    }

    /// 回傳指定 pane 目前 preview 搜尋命中的數量。
    fn preview_match_count(&self, pane_id: usize) -> usize {
        self.panes
            .get(&pane_id)
            .map(PaneState::preview_match_count)
            .unwrap_or(0)
    }

    /// 清除目前 preview 的搜尋狀態；若有清除任何內容則回傳 `true`。
    fn clear_preview_search_if_active(&mut self) -> bool {
        if let Some(search) = self.preview_search.take() {
            if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                pane.clear_preview_search();
            }
            self.status = String::from("preview search cleared");
            return true;
        }

        if let Some(pane) = self.panes.get_mut(&self.focused_pane)
            && pane.is_preview_active()
            && pane.has_preview_search()
        {
            pane.clear_preview_search();
            self.status = String::from("preview search cleared");
            return true;
        }

        false
    }

    /// 清除目前焦點 pane 上的列表內 find-next 結果；若有清除任何內容則回傳 `true`。
    fn clear_list_find_if_active(&mut self) -> bool {
        if let Some(search) = self.list_find.take() {
            if let Some(pane) = self.panes.get_mut(&search.pane_id) {
                pane.clear_list_find();
            }
            self.status = String::from("normal mode");
            return true;
        }

        if let Some(pane) = self.panes.get_mut(&self.focused_pane)
            && pane.has_list_find()
        {
            pane.clear_list_find();
            self.status = String::from("normal mode");
            return true;
        }

        false
    }

    /// 計算指定 pane 目前列表內 find-next 的命中數量。
    fn list_find_match_count(&self, pane_id: usize) -> usize {
        self.panes
            .get(&pane_id)
            .map(|pane| pane.list_find_match_indices().len())
            .unwrap_or(0)
    }

    /// 在目前焦點 pane 中跳到下一個或上一個列表內 find-next 命中結果。
    fn jump_list_find_match(&mut self, forward: bool, count: usize) -> io::Result<String> {
        let pane = self.current_pane_mut()?;
        let Some(query) = pane.list_find_query().map(str::to_string) else {
            return Ok(String::from("normal mode"));
        };

        let mut found = false;
        for _ in 0..count.max(1) {
            found = if forward {
                pane.jump_to_next_list_find_match()
            } else {
                pane.jump_to_previous_list_find_match()
            };
            if !found {
                break;
            }
        }
        let count = pane.list_find_match_indices().len();

        Ok(if found {
            list_find_locked_status(&query, count)
        } else {
            format!("find next: {query} (0)")
        })
    }

    /// 在 preview mode 中跳到下一個或上一個搜尋結果，並回傳狀態訊息。
    fn jump_preview_match(&mut self, forward: bool, count: usize) -> io::Result<String> {
        let Some(pane) = self.panes.get_mut(&self.focused_pane) else {
            return Ok(String::from("panel no longer exists"));
        };
        if !pane.is_preview_active() {
            return Ok(String::from("preview mode is not active"));
        }

        let Some(query) = pane.preview_search_query().map(str::to_string) else {
            return Ok(String::from("preview search is empty"));
        };

        let mut found = false;
        for _ in 0..count.max(1) {
            found = if forward {
                pane.jump_to_next_preview_match()
            } else {
                pane.jump_to_previous_preview_match()
            };
            if !found {
                break;
            }
        }
        let count = pane.preview_match_count();

        Ok(if found {
            format!("preview search: {query} ({count})")
        } else {
            format!("preview search: {query} (0)")
        })
    }

    /// 將目前 global search 選到的結果打開到原 pane 中，並把游標移到該項目。
    fn open_global_search_result(&mut self, search: GlobalSearchState) -> io::Result<()> {
        let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
        let Some(entry) = visible.get(search.selected).cloned() else {
            self.status = String::from("global search: no result selected");
            self.global_search = Some(search);
            return Ok(());
        };

        if !self.panes.contains_key(&search.pane_id) {
            self.status = String::from("panel no longer exists");
            return Ok(());
        }

        self.reveal_path_and_track(search.pane_id, &entry.path)?;
        if let Some(task_id) = search.task_id.or(self.active_global_search_task_id.take()) {
            self.finish_task(
                task_id,
                TaskState::Cancelled,
                String::from("stopped after opening a result"),
            );
        }
        self.cancel_global_search_worker();
        self.global_search = None;
        if let Some(pane) = self.panes.get_mut(&search.pane_id) {
            pane.set_preview_active(false);
        }
        self.status = format!("search opened: {}", entry.relative_path);
        Ok(())
    }

    /// 依照目前內容搜尋選到的檔案，計算搜尋 preview 的狀態列文字。
    fn search_preview_status_for(&self, search: &GlobalSearchState) -> String {
        let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
        let Some(entry) = visible.get(search.selected) else {
            return global_search_status(
                search.mode,
                &search.buffer,
                search.results.len(),
                false,
                search.searched,
                search.loading,
            );
        };
        let matches = PaneState::search_preview_match_positions(&entry.path, &search.buffer);
        if matches.is_empty() {
            return format!("preview search: {} (0)", search.buffer);
        }
        let current_match = search.preview_current_match.unwrap_or(matches[0]);
        let current = matches
            .iter()
            .position(|line| *line == current_match)
            .map(|index| index + 1)
            .unwrap_or(matches.len());
        format!(
            "preview search: {} ({}/{})",
            search.buffer,
            current,
            matches.len()
        )
    }

    /// 在 global search 的 preview focus 模式中，跳到下一個或上一個命中位置。
    fn move_search_preview_match(&self, search: &mut GlobalSearchState, forward: bool) {
        let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
        let Some(entry) = visible.get(search.selected) else {
            search.preview_scroll = None;
            return;
        };
        let matches = PaneState::search_preview_match_positions(&entry.path, &search.buffer);
        if matches.is_empty() {
            search.preview_current_match = None;
            search.preview_scroll = Some(0);
            return;
        }
        let current = search.preview_current_match.unwrap_or(matches[0]);
        let target = if forward {
            matches
                .iter()
                .copied()
                .find(|line| *line > current)
                .unwrap_or(matches[0])
        } else {
            matches
                .iter()
                .rev()
                .copied()
                .find(|line| *line < current)
                .unwrap_or(*matches.last().unwrap_or(&matches[0]))
        };
        search.preview_current_match = Some(target);
        search.preview_scroll = Some(target);
    }

    /// 切換目前焦點 pane 的隱藏檔顯示狀態。
    fn toggle_hidden_files(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        pane.toggle_hidden();
        self.status = if pane.show_hidden {
            String::from("showing hidden files")
        } else {
            String::from("hiding hidden files")
        };
        Ok(())
    }

    /// 將指定 pane 套用某一種排序模式。
    fn apply_sort_mode(&mut self, pane_id: usize, sort_mode: SortMode) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        pane.set_sort_mode(sort_mode);
        let needs_directory_sizes = matches!(pane.active_detail_kind(), SortDetailKind::Size);
        self.status = format!("sort: {}", pane.sort_mode.label());
        if needs_directory_sizes {
            self.start_directory_size_scan(pane_id);
        } else {
            self.cancel_directory_size_scan(pane_id);
        }
        Ok(())
    }

    /// 套用指定 pane 的 linemode，只更新右側欄位顯示方式，不改動原本排序順序。
    ///
    /// 參數：
    /// - `pane_id: usize`，要被套用 linemode 的 pane 編號。
    /// - `line_mode: LineMode`，要切換成的右側欄位模式。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表 linemode 已套用完成。
    /// - 若目標 pane 已不存在，會改寫狀態列並直接結束。
    fn apply_line_mode(&mut self, pane_id: usize, line_mode: LineMode) -> io::Result<()> {
        let Some(pane) = self.panes.get_mut(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        pane.set_line_mode(line_mode);
        let needs_directory_sizes = matches!(pane.active_detail_kind(), SortDetailKind::Size);
        self.status = format!("linemode: {}", line_mode.label());
        if needs_directory_sizes {
            self.start_directory_size_scan(pane_id);
        } else {
            self.cancel_directory_size_scan(pane_id);
        }
        Ok(())
    }

    /// 為指定 panel 啟動非同步遞迴目錄大小掃描。
    ///
    /// 參數：`pane_id: usize`，要顯示 `ms` 大小的 panel 編號。
    /// 回傳：`() `；panel 不存在或已在計算相同目錄時不重複重啟，避免複製或頻繁變更時數字反覆歸零跳動。
    fn start_directory_size_scan(&mut self, pane_id: usize) {
        let (cwd, directories) = {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                return;
            };
            if self
                .directory_size_jobs
                .get(&pane_id)
                .is_some_and(|job| job.cwd == pane.cwd)
            {
                return;
            }
            pane.init_directory_sizes_if_missing();
            let cwd = pane.cwd.clone();
            let directories = pane
                .entries
                .iter()
                .filter(|entry| entry.is_dir)
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>();
            (cwd, directories)
        };
        self.cancel_directory_size_scan(pane_id);
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            scan_directory_sizes(directories, &worker_cancelled, &sender);
            let _ = sender.send(DirectorySizeEvent::Done);
        });
        self.directory_size_jobs.insert(
            pane_id,
            DirectorySizeJob {
                cwd,
                receiver,
                cancelled,
            },
        );
    }

    /// 在 panel 成功切換目錄後，同步目前目錄容量工作的生命週期。
    ///
    /// 參數：
    /// - `pane_id: usize`，剛完成目錄切換的 panel 編號。
    /// - `previous_cwd: &Path`，切換前的工作目錄，用來避免選到一般檔案時無謂重啟。
    ///
    /// 回傳：`() `。只有工作目錄真的改變才處理；`linemode size` 或 size 排序啟用時，
    /// 會取消舊目錄工作並立即替新列表填入 `~0B`、啟動新掃描。其他顯示模式則只取消
    /// 可能殘留的舊工作，避免背景執行緒繼續走訪已離開的目錄。
    fn restart_directory_size_scan_after_navigation(
        &mut self,
        pane_id: usize,
        previous_cwd: &Path,
    ) {
        let is_size_detail = {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                self.cancel_directory_size_scan(pane_id);
                return;
            };
            if pane.cwd == previous_cwd {
                return;
            }
            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                pane.clear_directory_sizes();
                true
            } else {
                false
            }
        };
        self.cancel_directory_size_scan(pane_id);
        if is_size_detail {
            self.start_directory_size_scan(pane_id);
        }
    }

    /// 取消指定 panel 尚未完成的大小掃描並丟棄其接收端。
    ///
    /// 參數：`pane_id: usize`，要停止掃描的 panel 編號。
    /// 回傳：`() `；沒有工作時安全地不做任何事。
    fn cancel_directory_size_scan(&mut self, pane_id: usize) {
        if let Some(job) = self.directory_size_jobs.remove(&pane_id) {
            job.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// 取出並清除下一幀的完整重畫需求。
    ///
    /// 參數：`self: &mut App`，目前應用程式狀態。
    /// 回傳：`bool`；`true` 代表事件迴圈必須先呼叫 `Terminal::clear()`。
    pub(crate) fn take_full_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.full_redraw_requested)
    }

    /// 將目前選取項目放進內部剪貼簿，模式為複製。
    ///
    /// 參數：無。
    /// 回傳：`()`
    pub(crate) fn copy_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Copy);
    }

    /// 將目前選取項目放進內部剪貼簿，模式為剪下。
    ///
    /// 參數：無。
    /// 回傳：`()`
    pub(crate) fn cut_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Cut);
    }

    /// 清除目前內部剪貼簿中指定類型的 yank 狀態。
    ///
    /// 規則：
    /// - 若目前剪貼簿剛好就是指定操作類型，就清掉它。
    /// - 若目前不是該類型，則只更新狀態列，不動既有內容。
    fn clear_clipboard(&mut self, operation: ClipboardOperation) {
        match self.clipboard.as_ref() {
            Some(clipboard) if clipboard.operation == operation => {
                self.clipboard = None;
                self.status = match operation {
                    ClipboardOperation::Copy => String::from("cleared copied items"),
                    ClipboardOperation::Cut => String::from("cleared cut items"),
                };
            }
            _ => {
                self.status = match operation {
                    ClipboardOperation::Copy => String::from("no copied items to clear"),
                    ClipboardOperation::Cut => String::from("no cut items to clear"),
                };
            }
        }
    }

    /// 把目前焦點 pane 的選取項目寫入剪貼簿。
    ///
    /// 參數：
    /// - `operation: ClipboardOperation`，要記錄成複製或剪下。
    ///
    /// 回傳：`()`
    fn store_selected_in_clipboard(&mut self, operation: ClipboardOperation) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        };

        let entries: Vec<ClipboardEntry> = pane
            .selected_or_marked_entries()
            .into_iter()
            .map(|entry| {
                let display_name = entry.display_name();
                ClipboardEntry {
                    source_path: entry.path,
                    display_name,
                }
            })
            .collect();

        if entries.is_empty() {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        }

        let count = entries.len();
        self.clipboard = Some(ClipboardState { entries, operation });

        self.status = match operation {
            ClipboardOperation::Copy if count == 1 => String::from("copied 1 item"),
            ClipboardOperation::Copy => format!("copied {count} items"),
            ClipboardOperation::Cut if count == 1 => String::from("cut 1 item"),
            ClipboardOperation::Cut => format!("cut {count} items"),
        };
    }

    /// 將內部剪貼簿中的項目貼到目前有焦點的 pane 目錄。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表貼上流程已完成，並同步更新所有 pane。
    /// - 失敗時代表目標目錄或檔案系統操作失敗。
    pub(crate) fn paste_into_focused_pane(&mut self) -> io::Result<()> {
        self.paste_into_focused_pane_with_confirmation()
    }

    /// 將內部剪貼簿中的項目以覆蓋模式貼到目前有焦點的 pane 目錄。
    ///
    /// 規則：
    /// - 若目標名稱已存在，會直接覆蓋目標。
    /// - 若來源與目標本來就在同一路徑，會退回一般不覆蓋的行為避免覆寫自己。
    pub(crate) fn paste_into_focused_pane_with_overwrite(&mut self) -> io::Result<()> {
        self.paste_into_focused_pane_impl(true)
    }

    /// 先檢查目前貼上目標是否會和既有項目同名，必要時再打開覆蓋確認視窗。
    ///
    /// 規則：
    /// - 若沒有任何同名衝突，直接沿用一般貼上流程。
    /// - 若有同名衝突，先詢問是否要整批覆蓋。
    /// - 若來源與目標本來就是同一路徑，仍維持原本的 duplicate 命名策略，不視為衝突。
    ///
    /// 參數：
    /// - `self: &mut App`，目前應用程式狀態。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已直接貼上，或已打開確認視窗等待使用者決定。
    /// - 失敗時代表在檢查目標目錄是否衝突時發生 I/O 錯誤。
    fn paste_into_focused_pane_with_confirmation(&mut self) -> io::Result<()> {
        let Some(clipboard) = self.clipboard.as_ref() else {
            self.status = String::from("clipboard is empty");
            return Ok(());
        };
        let conflicts = self.paste_conflict_names()?;
        if conflicts.is_empty() {
            return self.paste_into_focused_pane_impl(false);
        }

        let target_name = if conflicts.len() == 1 {
            conflicts[0].clone()
        } else {
            format!("{} items", conflicts.len())
        };
        self.pending_action = Some(PendingAction::ConfirmPasteOverwrite {
            pane_id: self.focused_pane,
            target_name: target_name.clone(),
            entry_count: clipboard.entries.len(),
            operation: clipboard.operation,
        });
        self.status = paste_overwrite_confirm_status(&target_name, clipboard.entries.len());
        Ok(())
    }

    /// 負責實作一般貼上與覆蓋貼上的共用流程。
    fn paste_into_focused_pane_impl(&mut self, overwrite: bool) -> io::Result<()> {
        let Some(clipboard) = self.clipboard.clone() else {
            self.status = String::from("clipboard is empty");
            return Ok(());
        };

        let target_dir = match self.panes.get(&self.focused_pane) {
            Some(pane) => pane.cwd.clone(),
            None => {
                self.status = String::from("panel no longer exists");
                return Ok(());
            }
        };

        if paste_should_run_in_background(&clipboard, &target_dir) {
            return self.start_background_paste(
                self.focused_pane,
                target_dir,
                clipboard,
                overwrite,
            );
        }

        let mut pasted_count = 0usize;
        let mut history_items = Vec::new();
        for entry in &clipboard.entries {
            if entry.source_path.parent() == Some(target_dir.as_path())
                && clipboard.operation == ClipboardOperation::Cut
            {
                continue;
            }

            // 在真正執行前先保存目標名稱；失敗後檔案可能已被清理，不能再靠目錄內容
            // 推測目的地。這也確保同名複製時能顯示實際的 `copy` 名稱。
            let planned_target = self
                .panes
                .get(&self.focused_pane)
                .and_then(|pane| {
                    pane.planned_paste_target(&entry.source_path, overwrite)
                        .ok()
                })
                .unwrap_or_else(|| target_dir.join(&entry.display_name));

            let paste_result = match self.panes.get_mut(&self.focused_pane) {
                Some(pane) => match clipboard.operation {
                    ClipboardOperation::Copy => {
                        pane.copy_entry_with_history(&entry.source_path, overwrite)
                    }
                    ClipboardOperation::Cut => {
                        pane.move_entry_with_history(&entry.source_path, overwrite)
                    }
                },
                None => {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
            };

            let outcome = match paste_result {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.record_file_operation(clipboard.operation, history_items);
                    self.status =
                        paste_failure_status(&entry.display_name, &planned_target, &error);
                    return Ok(());
                }
            };

            pasted_count += 1;
            history_items.push(OperationItem {
                source_path: entry.source_path.clone(),
                destination_path: outcome.target_path,
                replaced_backup: outcome.backup_path,
            });
        }

        if pasted_count == 0 {
            self.status = String::from("nothing to paste into this directory");
            return Ok(());
        }

        self.reload_all_panes()?;
        self.status = paste_success_status(clipboard.operation, overwrite, pasted_count);

        if clipboard.operation == ClipboardOperation::Cut {
            self.clipboard = None;
        }
        self.record_file_operation(clipboard.operation, history_items);

        Ok(())
    }

    /// 把大型或網路目的地 paste 排入背景 task，避免傳輸期間凍結 TUI。
    ///
    /// 參數：
    /// - `pane_id: usize`，啟動貼上的目標 panel。
    /// - `target_dir: PathBuf`，實際目的目錄，可能是 UNC 或 macOS `/Volumes`。
    /// - `clipboard: ClipboardState`，本次固定使用的來源批次與 copy/cut 模式。
    /// - `overwrite: bool`，是否允許覆蓋同名項目。
    ///
    /// 回傳：`io::Result<()>`；成功表示工作已排入背景，完成結果稍後由主迴圈套用。
    fn start_background_paste(
        &mut self,
        pane_id: usize,
        target_dir: PathBuf,
        clipboard: ClipboardState,
        overwrite: bool,
    ) -> io::Result<()> {
        let entry_count = clipboard.entries.len();
        let operation = clipboard.operation;
        let operation_label = match operation {
            ClipboardOperation::Copy => "copy",
            ClipboardOperation::Cut => "move",
        };
        let task_id = self.push_task(
            pane_id,
            "paste",
            format!("{operation_label} {entry_count} item(s)"),
            format!("destination: {}", target_dir.display()),
            clipboard
                .entries
                .iter()
                .map(|entry| entry.source_path.display().to_string())
                .collect(),
            Some(target_dir.display().to_string()),
        );
        let mut busy = Vec::new();
        for entry in &clipboard.entries {
            let item_name = entry
                .source_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| entry.display_name.trim_end_matches('/').to_string());
            busy.push(target_dir.join(&item_name));
            if clipboard.operation == ClipboardOperation::Cut {
                busy.push(entry.source_path.clone());
            }
        }
        self.active_file_job_busy_paths.insert(task_id, busy);
        // 背景 paste 一排入就顯示初始 byte，避免總大小尚未發現時 task 面板只有
        // RUNNING 而沒有任何進度資訊。後續事件會逐步修正已完成量與總量。
        self.update_task_progress(task_id, 0, 1);
        let (sender, receiver) = mpsc::channel();
        let worker_clipboard = clipboard.clone();
        let worker_target_dir = target_dir.clone();
        thread::spawn(move || {
            let progress_sender = sender.clone();
            let progress_target_dir = worker_target_dir.clone();
            let mut completed_bytes = 0u64;
            let mut total_bytes = 0u64;
            let mut last_progress = None;
            let mut last_progress_update = Instant::now()
                .checked_sub(Duration::from_millis(500))
                .unwrap_or_else(Instant::now);
            let mut progress = |event: TransferProgress| {
                if event == TransferProgress::TargetVisible {
                    let _ = progress_sender.send(FileJobEvent::DestinationVisible {
                        target_dir: progress_target_dir.clone(),
                    });
                    return;
                }

                match event {
                    TransferProgress::BytesDiscovered(increment) => {
                        total_bytes = total_bytes.saturating_add(increment);
                    }
                    TransferProgress::BytesCopied(increment) => {
                        completed_bytes = completed_bytes.saturating_add(increment);
                    }
                    TransferProgress::TargetVisible => unreachable!(),
                }
                // 動態總量可能在相鄰檔案間持續增加；若每次都送到 App，task
                // history 會持續寫檔並反過來拖慢數十萬個小檔案的 copy。固定節流為
                // 每 500ms 最多一次，完成事件前仍會再送最後的精確值。
                if last_progress_update.elapsed() >= Duration::from_millis(500) {
                    send_progress_if_changed(
                        &progress_sender,
                        task_id,
                        completed_bytes,
                        total_bytes,
                        &mut last_progress,
                    );
                    last_progress_update = Instant::now();
                }
            };
            let result = perform_paste_job(
                &worker_clipboard,
                &worker_target_dir,
                overwrite,
                &mut progress,
            );
            drop(progress);

            let _ = sender.send(FileJobEvent::Progress {
                task_id,
                completed_bytes,
                total_bytes: total_bytes.max(completed_bytes),
            });
            let _ = sender.send(FileJobEvent::Paste {
                task_id,
                clipboard: worker_clipboard,
                overwrite,
                result,
            });
        });
        self.file_job_receivers.insert(task_id, receiver);
        self.status = format!(
            "pasting {entry_count} item(s) in background to {} [task {task_id}]",
            target_dir.display()
        );
        Ok(())
    }

    /// 將已成功的貼上項目整理成一筆批次 Undo 歷史。
    ///
    /// 參數：
    /// - `operation: ClipboardOperation`，原操作是 Copy 或 Cut/Move。
    /// - `items: Vec<OperationItem>`，本批次真正成功的項目；部分失敗時可能少於剪貼簿。
    ///
    /// 回傳：`() `；空清單不會建立歷史。
    fn record_file_operation(&mut self, operation: ClipboardOperation, items: Vec<OperationItem>) {
        let kind = match operation {
            ClipboardOperation::Copy => FileOperationKind::Copy,
            ClipboardOperation::Cut => FileOperationKind::Move,
        };
        self.operation_history.push(FileOperation { kind, items });
    }

    /// 復原最近一次成功的 Copy 或 Move 批次，並同步刷新所有 panel。
    ///
    /// Copy 建立物會移到 PaneFM Trash；Move 會搬回原來源。可連續呼叫以依序復原最多
    /// 20 筆記憶體內歷史，重開程式後歷史會清空。
    ///
    /// 參數：`self: &mut App`，目前應用程式狀態。
    /// 回傳：`io::Result<()>`；檔案系統拒絕復原時保留失敗項目供下次重試。
    pub(crate) fn undo_latest_file_operation(&mut self) -> io::Result<()> {
        match self.operation_history.undo_latest(&self.trash_store) {
            Ok(None) => self.status = String::from("nothing to undo"),
            Ok(Some(result)) => {
                self.reload_all_panes()?;
                let action = match result.kind {
                    FileOperationKind::Copy => "copy",
                    FileOperationKind::Move => "move",
                };
                self.status = if result.failed == 0 {
                    format!("undid {action}: {} items", result.restored)
                } else {
                    format!(
                        "undo {action}: {} restored, {} failed; press u to retry",
                        result.restored, result.failed
                    )
                };
            }
            Err(error) => {
                self.status = format!("undo failed: {error}");
            }
        }
        Ok(())
    }

    /// 掃描這次貼上是否會和目前目標目錄中的既有項目同名。
    ///
    /// 規則：
    /// - 只有「直接同名」的目標才算衝突。
    /// - 若來源本身就位於同一個目錄，視為 duplicate copy / move 情境，不算衝突。
    ///
    /// 參數：
    /// - `self: &App`，目前應用程式狀態。
    ///
    /// 回傳：`io::Result<Vec<String>>`。
    /// - 成功時回傳所有會衝突的名稱清單。
    /// - 失敗時回傳取得目標 pane 或檢查檔案資訊時的錯誤。
    fn paste_conflict_names(&self) -> io::Result<Vec<String>> {
        let Some(clipboard) = self.clipboard.as_ref() else {
            return Ok(Vec::new());
        };
        let target_dir = self
            .panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "panel no longer exists"))?;

        let mut conflicts = Vec::new();
        for entry in &clipboard.entries {
            let Some(file_name) = entry.source_path.file_name() else {
                continue;
            };
            let direct_target = target_dir.join(file_name);
            let same_location = entry.source_path.parent() == Some(target_dir.as_path())
                && direct_target == entry.source_path;
            if !same_location && direct_target.exists() {
                conflicts.push(entry.display_name.clone());
            }
        }

        Ok(conflicts)
    }

    /// 在使用者確認後，以覆蓋模式完成這次貼上。
    ///
    /// 參數：
    /// - `self: &mut App`，目前應用程式狀態。
    /// - `pane_id: usize`，當初打開確認視窗時的目標 pane。
    /// - `target_name: String`，顯示在確認視窗中的名稱摘要。
    /// - `entry_count: usize`，這次整批貼上的項目數。
    /// - `operation: ClipboardOperation`，這批項目原本是 copy 還是 cut。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表覆蓋貼上已完成。
    /// - 失敗時代表貼上過程發生 I/O 錯誤。
    fn confirm_paste_overwrite(
        &mut self,
        pane_id: usize,
        target_name: String,
        entry_count: usize,
        operation: ClipboardOperation,
    ) -> io::Result<()> {
        if self.focused_pane != pane_id {
            self.focused_pane = pane_id;
        }
        if self
            .clipboard
            .as_ref()
            .is_none_or(|clipboard| clipboard.operation != operation)
        {
            self.status = format!("paste changed before overwrite: {target_name}");
            return Ok(());
        }
        let _ = entry_count;
        self.paste_into_focused_pane_impl(true)
    }

    /// 將目前選取或已標記的項目直接移動到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，目標目錄，可以是絕對路徑或相對於目前 pane 的路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_path(&mut self, target: &str) -> io::Result<()> {
        let Some(target_dir) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: move <target-dir>");
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 讓目前焦點 pane 直接跳到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，使用者在 command mode 輸入的目標路徑。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表目前 pane 已切到指定目錄，或定位到指定檔案。
    pub(crate) fn change_directory_from_command(&mut self, target: &str) -> io::Result<()> {
        if target.trim_start().starts_with("smb://") {
            return self.goto_smb_location(target.trim());
        }

        let Some(target_path) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: goto <path>");
            return Ok(());
        };

        if is_unc_path(target.trim()) {
            return self.start_network_goto(target_path);
        }

        match self.go_to_path_and_track(self.focused_pane, &target_path) {
            Ok(()) => {
                self.status = format!("jumped to path: {}", target_path.display());
            }
            Err(error) => {
                self.status = format!("path jump failed: {} ({error})", target_path.display());
            }
        }
        Ok(())
    }

    /// 在背景執行 UNC 目錄跳轉，避免 Windows 等待失聯 SMB 主機時凍結 TUI。
    ///
    /// 參數：
    /// - `target_path: PathBuf`，`//server/share` 或 `\\server\share` 形式的目標。
    ///
    /// 回傳：`io::Result<()>`；成功代表工作已排入背景，不代表網路目錄已載入完成。
    fn start_network_goto(&mut self, target_path: PathBuf) -> io::Result<()> {
        self.start_network_goto_with(target_path, |mut pane, target| {
            pane.go_to_path(&target)?;
            Ok(pane)
        })
    }

    /// 以可注入 loader 啟動 UNC 背景跳轉，讓測試能證明主執行緒不會等待網路 I/O。
    ///
    /// 參數：
    /// - `target_path: PathBuf`，要交給背景工作載入的 UNC 路徑。
    /// - `loader: F`，取得目前 panel 副本與目標路徑，回傳載入後的 panel 狀態。
    ///
    /// 回傳：`io::Result<()>`；panel 不存在時回傳正常狀態並顯示錯誤，其餘情況會
    /// 立即回傳，loader 則留在背景執行。
    fn start_network_goto_with<F>(&mut self, target_path: PathBuf, loader: F) -> io::Result<()>
    where
        F: FnOnce(PaneState, PathBuf) -> io::Result<PaneState> + Send + 'static,
    {
        if self.active_network_goto_task_id.is_some() {
            self.cancel_network_goto("replaced by new goto");
        }

        let pane_id = self.focused_pane;
        let Some(pane) = self.panes.get(&pane_id).cloned() else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let task_id = self.push_task(
            pane_id,
            "goto",
            format!("goto {}", target_path.display()),
            String::from("loading UNC path in background"),
            Vec::new(),
            Some(target_path.display().to_string()),
        );
        let (sender, receiver) = mpsc::channel();
        let worker_target = target_path.clone();
        thread::spawn(move || {
            let result = loader(pane, worker_target.clone());
            let _ = sender.send(NetworkGotoEvent {
                task_id,
                pane_id,
                target: worker_target,
                result,
            });
        });

        self.network_goto_rx = Some(receiver);
        self.active_network_goto_task_id = Some(task_id);
        self.status = format!(
            "connecting to {} in background; Esc cancels",
            target_path.display()
        );
        Ok(())
    }

    /// 取消目前 UNC 背景跳轉並捨棄晚到的結果。
    ///
    /// 參數：`reason: &str`，寫入 task log 的取消原因。
    /// 回傳：`()`；作業系統中已開始的阻塞呼叫可能稍後才結束，但不再影響 UI。
    fn cancel_network_goto(&mut self, reason: &str) {
        self.network_goto_rx = None;
        if let Some(task_id) = self.active_network_goto_task_id.take() {
            self.finish_task(task_id, TaskState::Cancelled, reason.to_string());
        }
        self.status = String::from("network goto cancelled");
    }

    /// 讓 `g` 系列快捷鍵可以快速跳到常用的系統目錄。
    ///
    /// 參數：
    /// - `directory: GoSpecialDirectory`，要跳去的預設目錄種類。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表已切到目標目錄。
    /// - 若系統上不存在該目錄，會在狀態列顯示原因。
    fn go_to_special_directory(&mut self, directory: GoSpecialDirectory) -> io::Result<()> {
        let Some(target_path) = special_directory_path(directory) else {
            self.status = format!("{} not available on this system", directory.label());
            return Ok(());
        };

        if !target_path.exists() {
            self.status = format!("{} missing: {}", directory.label(), target_path.display());
            return Ok(());
        }

        match self.go_to_path_and_track(self.focused_pane, &target_path) {
            Ok(()) => {
                self.status = format!("jumped to {}: {}", directory.label(), target_path.display());
            }
            Err(error) => {
                self.status = format!(
                    "{} jump failed: {} ({error})",
                    directory.label(),
                    target_path.display()
                );
            }
        }
        Ok(())
    }

    /// 將目前選取或已標記的項目移到指定 pane 目前所在的目錄。
    ///
    /// 參數：
    /// - `target: &str`，目標 pane 編號字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_pane_id(&mut self, target: &str) -> io::Result<()> {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: move-panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        let Some(target_dir) = self.panes.get(&target_pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 將目前焦點 pane 的選取項目批次移動到目標目錄。
    fn move_selected_entries_into_dir(&mut self, target_dir: &std::path::Path) -> io::Result<()> {
        let Some(source_pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let source_dir = source_pane.cwd.clone();
        let entries = source_pane.selected_or_marked_entries();

        if entries.is_empty() {
            self.status = String::from("nothing selected to move");
            return Ok(());
        }

        if !target_dir.exists() {
            self.status = format!("move target does not exist: {}", target_dir.display());
            return Ok(());
        }
        if !target_dir.is_dir() {
            self.status = format!("move target is not a directory: {}", target_dir.display());
            return Ok(());
        }
        if source_dir == target_dir {
            self.status = String::from("move target is the current directory");
            return Ok(());
        }

        let mut moved_count = 0usize;
        let mut history_items = Vec::new();
        for entry in &entries {
            let outcome = match PaneState::move_path_to_dir_with_history(&entry.path, target_dir) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.operation_history.push(FileOperation {
                        kind: FileOperationKind::Move,
                        items: history_items,
                    });
                    self.status = format!("move failed for {}: {error}", entry.display_name());
                    return Ok(());
                }
            };
            moved_count += 1;
            history_items.push(OperationItem {
                source_path: entry.path.clone(),
                destination_path: outcome.target_path,
                replaced_backup: outcome.backup_path,
            });
        }

        self.operation_history.push(FileOperation {
            kind: FileOperationKind::Move,
            items: history_items,
        });
        self.reload_all_panes()?;
        self.status = if moved_count == 1 {
            format!("moved 1 item -> {}", target_dir.display())
        } else {
            format!("moved {moved_count} items -> {}", target_dir.display())
        };
        Ok(())
    }

    /// 將命令列中的路徑字串解析成實際可用的目標路徑。
    fn resolve_path_argument(&self, target: &str) -> Option<PathBuf> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return None;
        }

        let base_dir = self.panes.get(&self.focused_pane)?.cwd.clone();
        let expanded = expand_tilde_path(trimmed).unwrap_or_else(|| trimmed.to_string());
        let path = PathBuf::from(&expanded);
        Some(
            if path.is_absolute() || is_windows_drive_path(&expanded) || is_unc_path(&expanded) {
                path
            } else {
                base_dir.join(path)
            },
        )
    }

    /// 將目前可用的 pane 編號整理成易讀字串，供錯誤訊息與提示使用。
    fn available_pane_ids_label(&self) -> String {
        self.ordered_pane_ids()
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// 重新整理所有 pane，讓跨目錄操作後的內容保持同步。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<()>`。
    /// - 成功時代表所有 pane 都已重新載入。
    /// - 失敗時代表至少有一個 pane 在重新讀取時發生錯誤。
    fn reload_all_panes(&mut self) -> io::Result<()> {
        for pane in self.panes.values_mut() {
            pane.reload()?;
        }
        for pane in self.panes.values() {
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
        }
        let size_panes = self
            .panes
            .iter()
            .filter(|(_, pane)| matches!(pane.active_detail_kind(), SortDetailKind::Size))
            .map(|(pane_id, _)| *pane_id)
            .collect::<Vec<_>>();
        for pane_id in size_panes {
            self.start_directory_size_scan(pane_id);
        }
        Ok(())
    }

    /// 重新載入正在顯示目的目錄或其子目錄的 panel，供背景貼上逐步更新列表。
    ///
    /// 參數：`directory: &Path`，背景工作開始寫入的目的根目錄。
    /// 回傳：`io::Result<()>`；任一位於目的樹內的 panel 載入失敗時回傳原始 I/O 錯誤。
    fn reload_panes_in_tree(&mut self, directory: &Path) -> io::Result<()> {
        for pane in self
            .panes
            .values_mut()
            .filter(|pane| pane.cwd == directory || pane.cwd.starts_with(directory))
        {
            pane.reload()?;
        }
        for pane in self
            .panes
            .values()
            .filter(|pane| pane.cwd == directory || pane.cwd.starts_with(directory))
        {
            self.directory_entry_cache
                .insert(pane.cwd.clone(), pane.entries.clone());
        }
        // 清理不在目前開啟 panel 中的陳舊子目錄快取
        self.directory_entry_cache.retain(|cached_path, _| {
            !cached_path.starts_with(directory)
                || self.panes.values().any(|pane| &pane.cwd == cached_path)
        });
        Ok(())
    }

    /// 將目前焦點 pane 的游標帶到第一個解壓結果，方便使用者立刻繼續操作。
    fn reveal_first_extracted_output(&mut self, extracted: &[ExtractedArchive]) -> io::Result<()> {
        let Some(first) = extracted.first() else {
            return Ok(());
        };
        let _ = self.reveal_path_and_track(self.focused_pane, &first.output_path);
        Ok(())
    }

    /// 根據目前應用程式狀態繪製整個畫面。
    ///
    /// 繪製前會先依 terminal cell 寬度切割狀態文字，再動態計算 status area 高度。
    /// 這讓一般通知仍只占一行，而貼上失敗的完整 destination 與 OS error 可以依實際
    /// 長度展開成多行，不會因終端視窗較窄而遺失錯誤尾端。
    ///
    /// 參數：
    /// - `self: &mut App`，提供目前 panel、輸入模式、狀態文字及 theme 等畫面狀態。
    /// - `frame: &mut ratatui::Frame<'_>`，ratatui 本次更新可使用的繪圖 frame。
    ///
    /// 回傳：`Option<(u16, u16)>`；畫面需要顯示文字輸入游標時回傳其 cell 座標，
    /// 否則回傳 `None`。畫面內容會直接寫入傳入的 `frame`。
    pub(crate) fn render(&mut self, frame: &mut ratatui::Frame<'_>) -> Option<(u16, u16)> {
        let raw_status_text = if self.command_mode {
            format!(":{}", self.command_buffer)
        } else {
            self.status.clone()
        };
        let status_text = wrap_status_text(&raw_status_text, frame.area().width);
        let status_style = if self.command_mode {
            Style::default()
        } else if status_is_error(&raw_status_text) {
            self.theme.danger_style()
        } else {
            Style::default()
        };
        let status_height = status_area_height(&status_text, frame.area().height.saturating_sub(3));
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(status_height),
            ])
            .split(frame.area());

        let mut pane_rects = BTreeMap::new();
        self.layout.render_rects(outer[0], &mut pane_rects);
        // PATH 掃描只在 dependency 面板真正顯示時執行。一般檔案列表每幀都重查四個
        // 外部命令不但沒有畫面用途，也會讓鍵盤回應時間受磁碟與網路 PATH 影響。
        let tool_statuses = matches!(self.pending_action, Some(PendingAction::ToolPanel { .. }))
            .then(external_tool_statuses)
            .unwrap_or_default();
        let mut cursor_position = None;
        for (&pane_id, &rect) in &pane_rects {
            let trash_overlay_state =
                trash_panel_overlay_state_from_pending_action(&self.pending_action, pane_id);
            let trash_lines = if let Some((selected, search, marked_ids, visual_anchor)) =
                trash_overlay_state.as_ref()
            {
                Some(
                    trash_panel_lines(
                        &self.trash_store,
                        &search.buffer,
                        marked_ids,
                        visual_anchor.map(|anchor| (anchor, *selected)),
                    )
                    .unwrap_or_default(),
                )
            } else {
                None
            };
            let task_records = if matches!(
                &self.pending_action,
                Some(PendingAction::TaskPanel {
                    pane_id: action_pane_id,
                    ..
                }) if *action_pane_id == pane_id
            ) {
                Some(self.tasks_for_pane(pane_id))
            } else {
                None
            };
            let task_lines = if let (
                Some(records),
                Some(PendingAction::TaskPanel {
                    pane_id: action_pane_id,
                    search,
                    ..
                }),
            ) = (task_records.as_ref(), self.pending_action.as_ref())
            {
                if *action_pane_id == pane_id {
                    Some(task_panel_lines(&filtered_task_entries(
                        records,
                        &search.buffer,
                    )))
                } else {
                    None
                }
            } else {
                None
            };
            let help_lines = if let Some(PendingAction::HelpPanel {
                pane_id: action_pane_id,
                search,
                ..
            }) = &self.pending_action
            {
                if *action_pane_id == pane_id {
                    Some(help_panel_lines(&search.buffer))
                } else {
                    None
                }
            } else {
                None
            };
            let regex_rename_lines = if let Some(PendingAction::RegexRename {
                pane_id: action_pane_id,
                previews,
                ..
            }) = &self.pending_action
            {
                if *action_pane_id == pane_id {
                    Some(regex_rename_panel_lines(previews))
                } else {
                    None
                }
            } else {
                None
            };
            let global_search_results = self
                .global_search
                .as_ref()
                .filter(|search| search.pane_id == pane_id)
                .map(|search| {
                    filtered_global_search_entries(&search.results, &search.filter.buffer)
                });
            // 工作標籤只可能出現在目前 viewport。舊實作會對大型目錄的全部項目呼叫
            // canonicalize；`deps` 有六萬筆時，單次 render 就產生六萬次同步 I/O，連
            // j/k、mn、T 都會被阻塞。這裡只檢查畫面實際看得到的數十筆路徑。
            let active_job_badges = if self.active_file_job_busy_paths.is_empty() {
                std::collections::HashMap::new()
            } else {
                self.panes
                    .get(&pane_id)
                    .map(|pane| {
                        visible_job_badge_paths(pane, rect.height.saturating_sub(2) as usize)
                            .into_iter()
                            .filter_map(|path| {
                                self.active_job_badge_for_path(&path)
                                    .map(|badge| (path, badge))
                            })
                            .collect::<std::collections::HashMap<_, _>>()
                    })
                    .unwrap_or_default()
            };
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                let rename_buffer = match &self.pending_action {
                    Some(PendingAction::Rename {
                        pane_id: rename_pane_id,
                        buffer,
                        cursor,
                        mode,
                        ..
                    }) if *rename_pane_id == pane_id => Some(InlineEditorState {
                        buffer: buffer.as_str(),
                        cursor: *cursor,
                        title: match mode {
                            RenameMode::Insert => " Rename (insert): ",
                            RenameMode::Normal => " Rename (normal): ",
                        },
                    }),
                    Some(PendingAction::CreateEntry {
                        pane_id: create_pane_id,
                        buffer,
                        cursor,
                        mode,
                    }) if *create_pane_id == pane_id => Some(InlineEditorState {
                        buffer: buffer.as_str(),
                        cursor: *cursor,
                        title: create_editor_title(*mode),
                    }),
                    _ => None,
                };
                let picker_options = match &self.pending_action {
                    Some(PendingAction::CopyPicker {
                        pane_id: copy_pane_id,
                        ..
                    }) if *copy_pane_id == pane_id => Some(
                        copy_picker_options()
                            .into_iter()
                            .map(|option| format!("{} -> {}", option.shortcut, option.label))
                            .collect::<Vec<_>>(),
                    ),
                    Some(PendingAction::OpenPicker {
                        pane_id: open_pane_id,
                        options,
                        ..
                    }) if *open_pane_id == pane_id => Some(
                        options
                            .iter()
                            .map(|option| option.label.clone())
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                };
                let picker_state = match &self.pending_action {
                    Some(PendingAction::CopyPicker {
                        pane_id: copy_pane_id,
                        selected,
                        ..
                    }) if *copy_pane_id == pane_id => {
                        picker_options.as_ref().map(|options| InlinePickerState {
                            title: " Copy: ",
                            options,
                            selected: *selected,
                        })
                    }
                    Some(PendingAction::OpenPicker {
                        pane_id: open_pane_id,
                        selected,
                        ..
                    }) if *open_pane_id == pane_id => {
                        picker_options.as_ref().map(|options| InlinePickerState {
                            title: " Open with: ",
                            options,
                            selected: *selected,
                        })
                    }
                    _ => None,
                };
                let panel_state = if let Some(search) = self.global_search.as_ref() {
                    (search.pane_id == pane_id && (search.loading || search.searched)).then_some(
                        PaneListState::Search(SearchListState {
                            results: global_search_results.as_deref().unwrap_or(&[]),
                            selected: search.selected,
                            loading: search.loading && self.config.search.show_loading,
                            preview_query: matches!(search.mode, SearchMode::Content)
                                .then_some(search.buffer.as_str()),
                            preview_scroll: search.preview_scroll,
                            preview_current_match: search.preview_current_match,
                        }),
                    )
                } else if let Some((selected, search, ..)) = trash_overlay_state.as_ref() {
                    Some(PaneListState::Trash {
                        lines: trash_lines.as_deref().unwrap_or(&[]),
                        selected: *selected,
                        search: &search.buffer,
                        editing: search.editing,
                        cursor: self.text_input_cursor,
                    })
                } else if let Some(PendingAction::TaskPanel {
                    pane_id: action_pane_id,
                    selected,
                    search,
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Tasks {
                            lines: task_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                            search: &search.buffer,
                            editing: search.editing,
                            cursor: self.text_input_cursor,
                        })
                    } else {
                        None
                    }
                } else if let Some(PendingAction::HelpPanel {
                    pane_id: action_pane_id,
                    selected,
                    search,
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Help {
                            lines: help_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                            search: &search.buffer,
                            editing: search.editing,
                            cursor: self.text_input_cursor,
                        })
                    } else {
                        None
                    }
                } else if let Some(PendingAction::ToolPanel {
                    pane_id: action_pane_id,
                    selected,
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Tools {
                            statuses: &tool_statuses,
                            selected: *selected,
                        })
                    } else {
                        None
                    }
                } else if let Some(PendingAction::RegexRename {
                    pane_id: action_pane_id,
                    selected,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::RegexRename {
                            lines: regex_rename_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let preview_active = pane.is_preview_active();
                let pane_cursor = render_pane(
                    frame,
                    rect,
                    pane_id,
                    pane,
                    pane_id == self.focused_pane,
                    preview_active,
                    self.visual_selection.as_ref().and_then(|selection| {
                        (selection.pane_id == pane_id)
                            .then_some((selection.anchor, selection.current))
                    }),
                    panel_state,
                    self.theme,
                    &self.config,
                    rename_buffer,
                    picker_state,
                    self.list_find
                        .as_ref()
                        .filter(|search| search.pane_id == pane_id)
                        .map(|search| search.buffer.as_str()),
                    self.list_find
                        .as_ref()
                        .is_some_and(|search| search.pane_id == pane_id),
                    self.text_input_cursor,
                    &active_job_badges,
                );
                if cursor_position.is_none() {
                    cursor_position = pane_cursor;
                }
            }
        }

        let help = Paragraph::new(status_shortcut_line(outer[1].width, self.theme))
            .block(Block::default().borders(Borders::TOP));
        frame.render_widget(help, outer[1]);

        frame.render_widget(Paragraph::new(status_text).style(status_style), outer[2]);

        if self.command_mode
            && let Some(area) = pane_rects.get(&self.focused_pane)
        {
            let command_suggestions = self.command_suggestions();
            let command_cursor = render_command_palette(
                frame,
                *area,
                self.theme,
                &self.command_buffer,
                &command_suggestions,
                self.command_suggestion_selected,
                self.text_input_cursor,
            );
            if cursor_position.is_none() {
                cursor_position = Some(command_cursor);
            }
        }

        if let Some(filter) = &self.filter
            && filter.editing
            && let Some(area) = pane_rects.get(&filter.pane_id)
        {
            let title = match filter.mode {
                FilterMode::Normal => " Filter [Normal] (Tab: Fuzzy) ",
                FilterMode::Fuzzy => " Filter [Fuzzy] (Tab: Normal) ",
            };
            let filter_cursor = render_filter_input(
                frame,
                *area,
                self.theme,
                title,
                &filter.buffer,
                self.text_input_cursor,
            );
            if cursor_position.is_none() {
                cursor_position = Some(filter_cursor);
            }
        }

        if let Some(search) = &self.preview_search
            && search.editing
            && let Some(area) = pane_rects.get(&search.pane_id)
        {
            let search_cursor = render_preview_search_input(
                frame,
                *area,
                self.theme,
                &search.buffer,
                self.text_input_cursor,
            );
            if cursor_position.is_none() {
                cursor_position = Some(search_cursor);
            }
        }

        if let Some(search) = &self.global_search
            && let Some(area) = pane_rects.get(&search.pane_id)
        {
            if search.editing {
                let search_cursor = render_global_search_panel(
                    frame,
                    *area,
                    self.theme,
                    search.mode.panel_title(true),
                    &search.buffer,
                    self.text_input_cursor,
                    true,
                );
                if cursor_position.is_none() {
                    cursor_position = Some(search_cursor);
                }
            } else if search.filter.editing {
                let filter_cursor = render_filter_input(
                    frame,
                    *area,
                    self.theme,
                    " Filter Results ",
                    &search.filter.buffer,
                    self.text_input_cursor,
                );
                if cursor_position.is_none() {
                    cursor_position = Some(filter_cursor);
                }
            }
        }

        match &mut self.pending_action {
            Some(PendingAction::ConfirmDelete {
                target_name,
                permanent,
                ..
            }) => {
                render_confirm_dialog(
                    frame,
                    frame.area(),
                    target_name,
                    *permanent,
                    self.theme,
                    &self.config,
                );
            }
            Some(PendingAction::ConfirmPasteOverwrite {
                target_name,
                entry_count,
                ..
            }) => {
                render_paste_overwrite_dialog(
                    frame,
                    frame.area(),
                    target_name,
                    *entry_count,
                    self.theme,
                    &self.config,
                );
            }
            Some(PendingAction::ConfirmTrashAction {
                action,
                target_name,
                entry_count,
                ..
            }) => {
                let confirm_area = trash_confirm_panel_id(action)
                    .and_then(|pane_id| pane_rects.get(&pane_id).copied())
                    .unwrap_or(frame.area());
                render_trash_confirm_dialog(
                    frame,
                    confirm_area,
                    action,
                    target_name,
                    *entry_count,
                    self.theme,
                    &self.config,
                );
            }
            Some(PendingAction::GoPicker { .. }) => {
                render_go_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::ThemeCommandPicker { .. }) => {
                render_theme_command_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::SortPicker { .. }) => {
                super::ui::render_sort_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::WindowPicker { .. }) => {
                render_window_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::LineModePicker { .. }) => {
                render_linemode_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::BookmarkPicker { .. }) => {
                render_bookmark_action_picker(frame, frame.area(), self.theme);
            }
            Some(PendingAction::ThemePicker { selected, .. }) => {
                render_theme_picker(frame, frame.area(), self.theme, *selected, &self.config);
            }
            Some(PendingAction::BookmarkList {
                pane_id,
                selected,
                mode,
                search,
            }) => {
                let filtered =
                    filtered_bookmark_entries(self.bookmark_store.list(), &search.buffer);
                let lines = bookmark_panel_lines(filtered);
                if let Some(area) = pane_rects.get(pane_id) {
                    let (title, empty_message) = bookmark_picker_copy(*mode);
                    let bookmark_cursor = render_bookmark_picker(
                        frame,
                        *area,
                        self.theme,
                        &lines,
                        *selected,
                        title,
                        empty_message,
                        &search.buffer,
                        search.editing,
                        self.text_input_cursor,
                    );
                    if search.editing && cursor_position.is_none() {
                        cursor_position = bookmark_cursor;
                    }
                }
            }
            Some(PendingAction::ZoxideList {
                pane_id,
                selected,
                entries,
                search,
            }) => {
                let filtered = filtered_zoxide_entries(entries, &search.buffer);
                let lines = zoxide_panel_lines(filtered);
                if let Some(area) = pane_rects.get(pane_id) {
                    let zoxide_cursor = render_zoxide_picker(
                        frame,
                        *area,
                        self.theme,
                        &lines,
                        *selected,
                        &search.buffer,
                        search.editing,
                        self.text_input_cursor,
                    );
                    if search.editing && cursor_position.is_none() {
                        cursor_position = zoxide_cursor;
                    }
                }
            }
            Some(PendingAction::DiffMatrix(state)) => {
                render_diff_matrix(frame, frame.area(), state, self.theme);
            }
            Some(PendingAction::TrashPanel { .. })
            | Some(PendingAction::TaskPanel { .. })
            | Some(PendingAction::HelpPanel { .. })
            | Some(PendingAction::ToolPanel { .. })
            | Some(PendingAction::CopyPicker { .. })
            | Some(PendingAction::OpenPicker { .. })
            | Some(PendingAction::RegexRename { .. }) => {}
            Some(PendingAction::Rename { .. }) | Some(PendingAction::CreateEntry { .. }) => {}
            None => {}
        }

        if let Some((x, y)) = cursor_position {
            frame.set_cursor_position((x, y));
        }
        cursor_position
    }

    /// 回傳目前畫面應該呈現的 rename 游標模式。
    ///
    /// 回傳：`Option<RenameMode>`。
    /// - `Some(RenameMode::Insert)` 代表應顯示細線游標。
    /// - `Some(RenameMode::Normal)` 代表應顯示方塊游標。
    /// - `None` 代表目前沒有 rename 輸入框，不需要特別切換。
    pub(crate) fn rename_cursor_mode(&self) -> Option<RenameMode> {
        match &self.pending_action {
            Some(PendingAction::Rename { mode, .. })
            | Some(PendingAction::CreateEntry { mode, .. }) => Some(*mode),
            Some(PendingAction::TrashPanel { search, .. })
            | Some(PendingAction::HelpPanel { search, .. })
            | Some(PendingAction::TaskPanel { search, .. })
            | Some(PendingAction::BookmarkList { search, .. })
            | Some(PendingAction::ZoxideList { search, .. })
                if search.editing =>
            {
                Some(self.text_input_mode)
            }
            _ if self.command_mode
                || self.filter.as_ref().is_some_and(|filter| filter.editing)
                || self
                    .preview_search
                    .as_ref()
                    .is_some_and(|search| search.editing)
                || self.list_find.is_some() =>
            {
                Some(self.text_input_mode)
            }
            _ if self
                .global_search
                .as_ref()
                .is_some_and(|search| search.editing || search.filter.editing) =>
            {
                Some(self.text_input_mode)
            }
            _ => None,
        }
    }

    /// 非阻塞接收背景 global search 的增量結果，並更新目前搜尋 panel 與 task。
    ///
    /// 參數：無；資料由 `global_search_rx` channel 取得。
    /// 回傳：`()`, 每輪最多處理八筆訊息，避免大量結果讓主事件迴圈失去回應。
    /// 每個訊息都核對 panel id 與 query，舊搜尋取消後晚到的 chunk 會被捨棄，不能
    /// 混入使用者後來啟動的新搜尋。
    pub(crate) fn poll_background_tasks(&mut self) {
        self.poll_filesystem_watcher();
        self.poll_network_goto();
        self.poll_file_jobs();
        self.poll_directory_load_jobs();
        self.poll_directory_size_jobs();
        self.poll_diff_job();

        let Some(receiver) = &self.global_search_rx else {
            return;
        };
        let messages: Vec<GlobalSearchEvent> = receiver.try_iter().take(8).collect();
        if messages.is_empty() {
            return;
        }

        let mut finished = false;
        let mut completed_search_task = None;
        for message in messages {
            match message {
                GlobalSearchEvent::Chunk {
                    pane_id,
                    query,
                    mut entries,
                } => {
                    let Some(search) = &mut self.global_search else {
                        continue;
                    };
                    if search.pane_id != pane_id || search.buffer != query {
                        continue;
                    }
                    // 搜尋結果採穩定串流列表：既有內容不重排，新資料只加到尾端。
                    // 這可避免使用者正在移動時，游標所在畫面行被新批次推來推去。
                    search.results.append(&mut entries);
                    search.results.truncate(200);
                    search.selected = search
                        .selected
                        .min(global_search_visible_len(search).saturating_sub(1));
                    search.searched = true;
                    self.status = global_search_status(
                        search.mode,
                        &search.buffer,
                        search.results.len(),
                        search.editing,
                        search.searched,
                        true,
                    );
                }
                GlobalSearchEvent::Done { pane_id, query } => {
                    if let Some(search) = &mut self.global_search {
                        if search.pane_id != pane_id || search.buffer != query {
                            continue;
                        }
                        search.loading = false;
                        search.searched = true;
                        if let Some(task_id) = search.task_id.take() {
                            completed_search_task = Some((task_id, search.results.len()));
                        }
                        self.status = global_search_status(
                            search.mode,
                            &search.buffer,
                            search.results.len(),
                            search.editing,
                            search.searched,
                            search.loading,
                        );
                    } else if let Some(task_id) = self.active_global_search_task_id {
                        completed_search_task = Some((task_id, 0));
                    }
                    finished = true;
                }
                GlobalSearchEvent::MissingTool {
                    pane_id,
                    query,
                    tool,
                } => {
                    let search_context = self
                        .global_search
                        .as_ref()
                        .filter(|search| search.pane_id == pane_id && search.buffer == query)
                        .map(|search| (search.task_id, search.mode));
                    self.global_search = None;
                    self.cancel_global_search_worker();
                    let task_id = search_context
                        .and_then(|(task_id, _)| task_id)
                        .or(self.active_global_search_task_id.take());
                    if let Some(task_id) = task_id {
                        self.finish_task(task_id, TaskState::Failed, format!("missing {tool}"));
                    }
                    self.pending_action = Some(PendingAction::ToolPanel {
                        pane_id,
                        selected: 0,
                    });
                    let mode = search_context
                        .map(|(_, mode)| mode)
                        .unwrap_or(SearchMode::Path);
                    self.status = missing_search_tool_status(mode, &tool);
                }
            }
        }

        if let Some((task_id, result_count)) = completed_search_task {
            self.active_global_search_task_id = None;
            self.finish_task(task_id, TaskState::Done, format!("{result_count} results"));
        }

        if finished {
            self.cancel_global_search_worker();
        }
    }

    /// 啟動單一 panel 的非阻塞目錄讀取，並先套用已有快取。
    ///
    /// 參數：`pane_id` 是導航來源 panel；`cwd` 是新目錄；`selected_path` 是回到父目錄
    /// 時應重新選取的子目錄。回傳：`() `；I/O 結果由 `poll_directory_load_jobs` 套用。
    fn start_directory_load(
        &mut self,
        pane_id: usize,
        cwd: PathBuf,
        selected_path: Option<PathBuf>,
    ) {
        self.cancel_directory_size_scan(pane_id);
        self.cancel_directory_load(pane_id);
        let (sort_mode, random_seed) = self
            .panes
            .get(&pane_id)
            .map(|pane| (pane.sort_mode, pane.random_seed))
            .unwrap_or((SortMode::Natural { reverse: false }, 0));
        if let Some(cached) = self.directory_entry_cache.get(&cwd).cloned()
            && let Some(pane) = self.panes.get_mut(&pane_id)
        {
            pane.replace_entries_presorted(cached, selected_path.as_deref());
        }

        let (sender, receiver) = mpsc::channel();
        let worker_cwd = cwd.clone();
        let worker_selection = selected_path.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            let thread_pane_id = pane_id;
            let thread_cwd = worker_cwd.clone();
            let thread_selection = worker_selection.clone();
            let thread_sender = sender.clone();
            let stream_result = super::pane::stream_dir_entries_with_cancellation(
                &worker_cwd,
                sort_mode,
                random_seed,
                &worker_cancelled,
                move |progress| {
                    thread_sender
                        .send(DirectoryLoadEvent {
                            pane_id: thread_pane_id,
                            cwd: thread_cwd.clone(),
                            selected_path: thread_selection.clone(),
                            result: Ok(progress),
                        })
                        .is_ok()
                },
            );
            if let Err(error) = stream_result {
                let _ = sender.send(DirectoryLoadEvent {
                    pane_id,
                    cwd: worker_cwd,
                    selected_path: worker_selection,
                    result: Err(error),
                });
            }
        });
        self.directory_load_jobs.insert(
            pane_id,
            DirectoryLoadJob {
                cwd: cwd.clone(),
                receiver,
                cancelled,
            },
        );
        self.status = format!("loading directory: {}", cwd.display());
    }

    /// 取消指定 panel 尚未完成的目錄載入工作。
    ///
    /// 參數：`pane_id: usize`，要停止載入的 panel 編號。
    /// 回傳：`() `；沒有工作時安全地不做任何事。
    fn cancel_directory_load(&mut self, pane_id: usize) {
        if let Some(job) = self.directory_load_jobs.remove(&pane_id) {
            job.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// 非阻塞接收大型目錄清單；分批套用快速發現項目，並在完成時更新快取與 size linemode。
    ///
    /// 參數：無。回傳：`() `；第一批（< 1ms）讓 UI 立即畫出列表與響應游標，後續增量批次平滑呈現。
    fn poll_directory_load_jobs(&mut self) {
        let pane_ids = self.directory_load_jobs.keys().copied().collect::<Vec<_>>();
        for pane_id in pane_ids {
            let mut job_done = false;
            let Some((job_cwd, events)) = self
                .directory_load_jobs
                .get(&pane_id)
                .map(|job| (job.cwd.clone(), job.receiver.try_iter().collect::<Vec<_>>()))
            else {
                continue;
            };
            for event in events {
                if event.cwd != job_cwd {
                    continue;
                }
                match event.result {
                    Ok(DirectoryLoadProgress::Batch {
                        entries,
                        is_first_chunk,
                    }) => {
                        if let Some(pane) = self.panes.get_mut(&event.pane_id)
                            && pane.cwd == event.cwd
                        {
                            if is_first_chunk {
                                pane.replace_entries_presorted(
                                    entries,
                                    event.selected_path.as_deref(),
                                );
                            } else {
                                pane.extend_entries(entries);
                            }
                            // `ms` 可能在目錄清單尚未載入完成時就已啟用。每批新加入的
                            // 目錄都要立刻取得部分容量狀態，否則右側欄位會一直空白，直到
                            // 完整清單載入並重新啟動容量掃描後才第一次出現內容。
                            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                                pane.init_directory_sizes_if_missing();
                            }
                            self.status = format!(
                                "loading directory: {} ({} items)",
                                event.cwd.display(),
                                pane.entries.len()
                            );
                        }
                    }
                    Ok(DirectoryLoadProgress::Complete(entries)) => {
                        job_done = true;
                        self.directory_entry_cache
                            .insert(event.cwd.clone(), entries.clone());
                        let mut restart_size_scan = false;
                        if let Some(pane) = self.panes.get_mut(&event.pane_id)
                            && pane.cwd == event.cwd
                        {
                            pane.replace_entries_presorted(entries, event.selected_path.as_deref());
                            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                                pane.init_directory_sizes_if_missing();
                                restart_size_scan = true;
                            }
                            self.status = format!("opened directory: {}", event.cwd.display());
                        }
                        if restart_size_scan {
                            // 載入期間可能已有只包含首批目錄的同 cwd 掃描。先取消舊工作
                            // 才能確保完整清單中的每個直接子目錄都會被納入新一輪計算。
                            self.cancel_directory_size_scan(event.pane_id);
                            self.start_directory_size_scan(event.pane_id);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                        job_done = true;
                    }
                    Err(error) => {
                        job_done = true;
                        if self
                            .panes
                            .get(&event.pane_id)
                            .is_some_and(|pane| pane.cwd == event.cwd)
                        {
                            self.status = format!("open directory failed: {error}");
                        }
                    }
                }
            }
            if job_done {
                self.directory_load_jobs.remove(&pane_id);
            }
        }
    }

    /// 非阻塞套用各 panel 的目錄大小快照。
    ///
    /// 參數：無，資料來自 `directory_size_jobs`。
    /// 回傳：`() `；每個 job 每幀最多處理 64 筆，避免大量小目錄拖慢鍵盤事件。
    fn poll_directory_size_jobs(&mut self) {
        let pane_ids = self.directory_size_jobs.keys().copied().collect::<Vec<_>>();
        for pane_id in pane_ids {
            let mut finished = false;
            let mut updates = Vec::new();
            let Some(job) = self.directory_size_jobs.get(&pane_id) else {
                continue;
            };
            let job_cwd = job.cwd.clone();
            for event in job.receiver.try_iter().take(64) {
                match event {
                    DirectorySizeEvent::Update {
                        path,
                        bytes,
                        complete,
                    } => updates.push((path, bytes, complete)),
                    DirectorySizeEvent::Done => finished = true,
                }
            }
            let cwd_matches = self
                .panes
                .get(&pane_id)
                .is_some_and(|pane| pane.cwd == job_cwd);
            if cwd_matches && let Some(pane) = self.panes.get_mut(&pane_id) {
                for (path, bytes, complete) in updates {
                    pane.update_directory_size(&path, bytes, complete);
                }
            }
            if finished || !cwd_matches {
                self.cancel_directory_size_scan(pane_id);
            }
        }
    }

    /// 接收檔案系統 watcher 事件，去重後刷新所有顯示受影響目錄的 panel。
    ///
    /// 參數：無；監看目錄來自目前 `panes`，事件來自 [`FilesystemWatcher`] channel。
    /// 回傳：`()`；watcher 或單一目錄刷新失敗只寫入狀態列，不會結束主事件迴圈。
    fn poll_filesystem_watcher(&mut self) {
        let directories = self
            .panes
            .values()
            .map(|pane| pane.cwd.clone())
            .collect::<BTreeSet<_>>();
        let (changed, watch_errors) = match self.filesystem_watcher.as_mut() {
            Some(watcher) => {
                let errors = watcher.sync_directories(directories);
                (watcher.changed_directories(), errors)
            }
            None => return,
        };

        if !watch_errors.is_empty() {
            self.status = format!("filesystem watcher failed: {}", watch_errors.join(" | "));
        }
        if !changed.is_empty() {
            self.pending_watched_directories.extend(changed);
            let debounce = if self.file_job_receivers.is_empty() {
                self.config.watcher.debounce
            } else {
                Duration::from_millis(400)
            };
            // 第一個事件設定 deadline，後續同批事件只合併路徑而不無限延後刷新。
            self.filesystem_refresh_deadline
                .get_or_insert_with(|| Instant::now() + debounce);
        }

        if !self
            .filesystem_refresh_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return;
        }

        self.filesystem_refresh_deadline = None;
        let changed = std::mem::take(&mut self.pending_watched_directories);
        if let Err(error) = self.reload_watched_directories(&changed) {
            self.status = format!("automatic directory refresh failed: {error}");
        }
    }

    /// 重新載入目前工作目錄出現在 watcher 變更集合中的所有 panel。
    ///
    /// 參數：`directories: &BTreeSet<PathBuf>`，已經過 debounce 的異動目錄集合。
    /// 回傳：`io::Result<()>`；任一受影響 panel 無法重新讀取時回傳原始 I/O 錯誤。
    fn reload_watched_directories(&mut self, directories: &BTreeSet<PathBuf>) -> io::Result<()> {
        let affected_panes = self
            .panes
            .iter()
            .filter(|(_, pane)| directories.contains(&pane.cwd))
            .map(|(pane_id, _)| *pane_id)
            .collect::<Vec<_>>();
        for pane_id in &affected_panes {
            let pane = self.panes.get_mut(pane_id).expect("pane id came from map");
            pane.reload()?;
        }
        for pane_id in &affected_panes {
            if let Some(pane) = self.panes.get(pane_id) {
                self.directory_entry_cache
                    .insert(pane.cwd.clone(), pane.entries.clone());
            }
        }
        for pane_id in &affected_panes {
            if self.file_job_receivers.is_empty()
                && self
                    .panes
                    .get(pane_id)
                    .is_some_and(|pane| matches!(pane.active_detail_kind(), SortDetailKind::Size))
            {
                self.start_directory_size_scan(*pane_id);
            }
        }
        for dir in directories {
            if !self.panes.values().any(|pane| &pane.cwd == dir) {
                self.directory_entry_cache.remove(dir);
            }
        }
        if !directories.is_empty() {
            self.full_redraw_requested = true;
        }
        Ok(())
    }

    /// 非阻塞接收 UNC `goto` 的背景載入結果，完成後才替換指定 panel。
    ///
    /// 參數：無；資料來自 `network_goto_rx`。
    /// 回傳：`()`；尚未完成時立即返回，取消後晚到的結果也不會被套用。
    fn poll_network_goto(&mut self) {
        let Some(receiver) = self.network_goto_rx.take() else {
            return;
        };

        let event = match receiver.try_recv() {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => {
                self.network_goto_rx = Some(receiver);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(task_id) = self.active_network_goto_task_id.take() {
                    self.finish_task(
                        task_id,
                        TaskState::Failed,
                        String::from("network goto worker disconnected"),
                    );
                }
                self.status = String::from("network goto failed: worker disconnected");
                return;
            }
        };

        if self.active_network_goto_task_id != Some(event.task_id) {
            return;
        }
        self.active_network_goto_task_id = None;

        match event.result {
            Ok(pane) if self.panes.contains_key(&event.pane_id) => {
                let cwd = pane.cwd.clone();
                self.panes.insert(event.pane_id, pane);
                self.zoxide_tracker.track(&cwd);
                self.finish_task(
                    event.task_id,
                    TaskState::Done,
                    format!("opened {}", event.target.display()),
                );
                self.full_redraw_requested = true;
                self.status = format!("jumped to path: {}", event.target.display());
            }
            Ok(_) => {
                self.finish_task(
                    event.task_id,
                    TaskState::Cancelled,
                    String::from("panel no longer exists"),
                );
                self.status = String::from("network goto cancelled: panel no longer exists");
            }
            Err(error) => {
                self.finish_task(event.task_id, TaskState::Failed, error.to_string());
                self.status = format!("path jump failed: {} ({error})", event.target.display());
            }
        }
    }

    /// 非阻塞接收大型 paste/compress/extract 工作，並在主執行緒更新 UI 與 Undo。
    ///
    /// 參數：無；接收端保存於 `file_job_receivers`。
    /// 回傳：`()`；每一輪只檢查現有 task，不等待尚未完成的 worker。
    fn poll_file_jobs(&mut self) {
        let task_ids = self.file_job_receivers.keys().copied().collect::<Vec<_>>();
        for task_id in task_ids {
            let Some(receiver) = self.file_job_receivers.remove(&task_id) else {
                continue;
            };
            let mut completed = false;
            let mut refresh_target = None;
            loop {
                match receiver.try_recv() {
                    Ok(FileJobEvent::DestinationVisible { target_dir }) => {
                        refresh_target = Some(target_dir);
                    }
                    Ok(FileJobEvent::Progress {
                        task_id,
                        completed_bytes,
                        total_bytes,
                    }) => {
                        self.update_task_progress(task_id, completed_bytes, total_bytes);
                    }
                    Ok(event) => {
                        self.apply_file_job_event(event);
                        completed = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        completed = true;
                        if self
                            .task_log
                            .iter()
                            .any(|task| task.id == task_id && task.state == TaskState::Running)
                        {
                            self.finish_task(
                                task_id,
                                TaskState::Failed,
                                String::from("file worker disconnected"),
                            );
                            self.status =
                                format!("file task {task_id} failed: worker disconnected");
                        }
                        break;
                    }
                }
            }
            if let Some(target_dir) = refresh_target {
                if let Err(error) = self.reload_panes_in_tree(&target_dir) {
                    self.status =
                        format!("background paste started; destination refresh failed: {error}");
                }
                self.full_redraw_requested = true;
            }
            if !completed {
                self.file_job_receivers.insert(task_id, receiver);
            } else {
                self.active_file_job_busy_paths.remove(&task_id);
                if self
                    .task_log
                    .iter()
                    .any(|task| task.id == task_id && task.state == TaskState::Running)
                {
                    self.finish_task(
                        task_id,
                        TaskState::Failed,
                        String::from("file worker disconnected"),
                    );
                    self.status = format!("file task {task_id} failed: worker disconnected");
                }
            }
        }
    }

    /// 套用單一背景檔案工作的完成結果。
    ///
    /// 參數：`event: FileJobEvent`，worker 回傳的 paste、compress、extract 或 delete 結果。
    /// 回傳：`()`；I/O 錯誤會完整寫入 task 與狀態列，不會中止主事件迴圈。
    fn apply_file_job_event(&mut self, event: FileJobEvent) {
        match event {
            FileJobEvent::DestinationVisible { .. } => {
                // 目標可見事件已在 `poll_file_jobs` 即時處理，不屬於完成事件。
            }
            FileJobEvent::Progress { .. } => {
                // Progress 已在 `poll_file_jobs` 合併處理，完成事件才會進入這個函數。
            }
            FileJobEvent::Paste {
                task_id,
                clipboard,
                overwrite,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                let operation = clipboard.operation;
                self.record_file_operation(operation, result.history_items);
                if let Some(failure) = result.failure {
                    let status = paste_failure_status(
                        &failure.display_name,
                        &failure.planned_target,
                        &failure.error,
                    );
                    self.finish_task(task_id, TaskState::Failed, status.clone());
                    self.status = status;
                } else {
                    if operation == ClipboardOperation::Cut
                        && self.clipboard.as_ref() == Some(&clipboard)
                    {
                        self.clipboard = None;
                    }
                    let status = paste_success_status(operation, overwrite, result.pasted_count);
                    self.finish_task(task_id, TaskState::Done, status.clone());
                    self.status = status;
                }
                if let Err(error) = self.reload_all_panes() {
                    self.status = format!("{}; refresh failed: {error}", self.status);
                }
                self.full_redraw_requested = true;
            }
            FileJobEvent::Compress {
                task_id,
                pane_id,
                entry_count,
                first_name,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok(archive_path) => {
                        if let Err(error) = self.reload_all_panes() {
                            self.status = format!("compress completed; refresh failed: {error}");
                            self.finish_task(task_id, TaskState::Failed, self.status.clone());
                            return;
                        }
                        if let Some(pane) = self.panes.get_mut(&pane_id) {
                            pane.select_path(&archive_path);
                        }
                        let archive_name = archive_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("archive.zip");
                        self.status = if entry_count == 1 {
                            format!("compressed {first_name} -> {archive_name}")
                        } else {
                            format!("compressed {entry_count} items -> {archive_name}")
                        };
                        self.finish_task(task_id, TaskState::Done, self.status.clone());
                        self.full_redraw_requested = true;
                    }
                    Err(error) => {
                        self.status = format!("compress failed: {error}");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                }
            }
            FileJobEvent::Extract {
                task_id,
                pane_id,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok((extracted, skipped)) if !extracted.is_empty() => {
                        if let Err(error) = self.reload_all_panes() {
                            self.status = format!("extract completed; refresh failed: {error}");
                            self.finish_task(task_id, TaskState::Failed, self.status.clone());
                            return;
                        }
                        if let Some(first) = extracted.first()
                            && let Some(pane) = self.panes.get_mut(&pane_id)
                        {
                            pane.select_path(&first.output_path);
                        }
                        self.status = extraction_status_label(&extracted, skipped);
                        self.finish_task(task_id, TaskState::Done, self.status.clone());
                        self.full_redraw_requested = true;
                    }
                    Ok((_, skipped)) => {
                        self.status = format!("no supported archives selected (skipped {skipped})");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                    Err(error) => {
                        self.status = format!("extract failed: {error}");
                        self.finish_task(task_id, TaskState::Failed, self.status.clone());
                    }
                }
            }
            FileJobEvent::Delete {
                task_id,
                target_name,
                result,
            } => {
                self.active_file_job_busy_paths.remove(&task_id);
                match result {
                    Ok(deleted_names) => {
                        let status = if deleted_names.len() == 1 {
                            format!("deleted permanently {}", deleted_names[0])
                        } else {
                            format!("deleted permanently {} items", deleted_names.len())
                        };
                        self.finish_task(task_id, TaskState::Done, status.clone());
                        self.status = status;
                    }
                    Err(error) => {
                        let status = format!("failed to delete {target_name}: {error}");
                        self.finish_task(task_id, TaskState::Failed, status.clone());
                        self.status = status;
                    }
                }
                if let Err(error) = self.reload_all_panes() {
                    self.status = format!("{}; refresh failed: {error}", self.status);
                }
                self.full_redraw_requested = true;
            }
        }
    }
}

/// 將設定檔中的啟動偏好套用到新建立的 pane。
///
/// 參數：
/// - `config: &AppConfig`，目前啟動所使用的設定。
/// - `pane: &mut PaneState`，要被套用預設值的 pane。
///
/// 回傳：`()`
fn apply_config_to_pane(config: &AppConfig, pane: &mut PaneState) {
    pane.set_show_hidden(config.pane.show_hidden);
    pane.set_sort_mode(sort_mode_from_config(
        config.pane.default_sort,
        config.pane.default_sort_reverse,
    ));
}

/// 收集目前 viewport 中需要查詢背景工作標籤的檔案路徑。
///
/// 參數：
/// - `pane: &PaneState`：提供 filter 後索引、游標與上一幀捲動位置的 panel。
/// - `viewport_height: usize`：列表實際可顯示的資料列數。
///
/// 回傳：`Vec<PathBuf>`，最多只包含一個 viewport 的路徑。刻意不回傳完整目錄清單，
/// 避免 task badge 查詢在大型目錄每幀對數萬個檔案執行 `canonicalize()`。
fn visible_job_badge_paths(pane: &PaneState, viewport_height: usize) -> Vec<PathBuf> {
    let (start, end) = visible_list_window_range(
        pane.visible_indices.len(),
        pane.selected,
        viewport_height.max(1),
        pane.list_state.offset(),
    );
    pane.visible_indices[start..end]
        .iter()
        .filter_map(|entry_index| pane.entries.get(*entry_index))
        .map(|entry| entry.path.clone())
        .collect()
}

/// 將設定檔中的排序偏好轉成 pane 實際使用的排序模式。
///
/// 參數：
/// - `sort: StartupSort`，設定檔指定的排序種類。
/// - `reverse: bool`，是否使用反向排序。
///
/// 回傳：`SortMode`，可直接套用到 pane 的排序模式。
fn sort_mode_from_config(sort: StartupSort, reverse: bool) -> SortMode {
    match sort {
        StartupSort::Alphabetical => SortMode::Alphabetical { reverse },
        StartupSort::Natural => SortMode::Natural { reverse },
        StartupSort::Size => SortMode::Size { reverse },
        StartupSort::Modified => SortMode::Modified { reverse },
        StartupSort::Created => SortMode::Created { reverse },
        StartupSort::Extension => SortMode::Extension { reverse },
        StartupSort::Random => SortMode::Random,
    }
}

/// 將書籤資料轉成彈窗可直接顯示的列內容。
fn bookmark_panel_lines(entries: Vec<BookmarkEntry>) -> Vec<BookmarkPanelLine> {
    entries
        .into_iter()
        .map(|entry| BookmarkPanelLine {
            key: format!("[{}]", entry.key),
            path: entry.target.display_text(),
        })
        .collect()
}

/// 依照搜尋字串過濾書籤清單，讓書籤列表也能使用 `f` 做即時篩選。
fn filtered_bookmark_entries(entries: Vec<BookmarkEntry>, query: &str) -> Vec<BookmarkEntry> {
    fuzzy_matched_indices_by_fields(&entries, query, |entry| {
        vec![entry.key.to_string(), entry.target.display_text()]
    })
    .into_iter()
    .map(|index| entries[index].clone())
    .collect()
}

/// 依書籤列表模式回傳彈窗標題與空狀態訊息。
fn bookmark_picker_copy(mode: BookmarkListMode) -> (&'static str, &'static str) {
    match mode {
        BookmarkListMode::Jump => (" Bookmarks ", "沒有書籤，按 b 再按 s 新增"),
        BookmarkListMode::Delete => (" Delete Bookmark ", "沒有可刪除的書籤"),
    }
}

/// 將 zoxide 目錄清單轉成彈窗可直接顯示的列內容。
fn zoxide_panel_lines(entries: Vec<PathBuf>) -> Vec<ZoxidePanelLine> {
    entries
        .into_iter()
        .map(|path| ZoxidePanelLine {
            path: path.display().to_string(),
        })
        .collect()
}

/// 依照搜尋字串過濾 zoxide 回傳的目錄列表，保留路徑中包含關鍵字的項目。
fn filtered_zoxide_entries(entries: &[PathBuf], query: &str) -> Vec<PathBuf> {
    fuzzy_matched_indices(entries, query, |path| path.display().to_string().into())
        .into_iter()
        .map(|index| entries[index].clone())
        .collect()
}

/// 依照搜尋字串過濾 task 清單，方便在任務很多時快速縮小範圍。
///
/// 參數：`tasks: &[TaskRecord]` 是原始任務；`query: &str` 是面板搜尋文字。
/// 回傳：`Vec<TaskRecord>`，會比對狀態、操作、結果、種類、所有來源與目的地。
fn filtered_task_entries(tasks: &[TaskRecord], query: &str) -> Vec<TaskRecord> {
    fuzzy_matched_indices_by_fields(tasks, query, |task| {
        let mut fields = vec![
            task_state_label(task.state).to_string(),
            task.title.clone(),
            task.detail.clone(),
            task.kind.to_string(),
        ];
        fields.extend(task.source_locations.iter().cloned());
        fields.extend(task.destination_location.iter().cloned());
        fields
    })
    .into_iter()
    .map(|index| tasks[index].clone())
    .collect()
}

/// 以共用模糊 matcher 過濾 `fd` 或 `rg` 已回傳的搜尋結果。
///
/// 參數：
/// - `entries: &[GlobalSearchEntry]`，背景搜尋目前已串流回主執行緒的原始結果。
/// - `query: &str`，結果面板中按 `f` 後輸入的模糊查詢。
///
/// 回傳：`Vec<GlobalSearchEntry>`，依模糊分數排序的可見結果副本；原始串流順序不會被修改。
fn filtered_global_search_entries(
    entries: &[GlobalSearchEntry],
    query: &str,
) -> Vec<GlobalSearchEntry> {
    fuzzy_matched_indices(entries, query, |entry| entry.relative_path.clone().into())
        .into_iter()
        .map(|index| entries[index].clone())
        .collect()
}

/// 從命令列參數中取出單一書籤按鍵。
///
/// 參數：
/// - `args: &str`，使用者在 `:bookmark ...` 後輸入的內容。
///
/// 回傳：`Option<char>`。
/// - `Some(char)` 代表成功解析出唯一按鍵。
/// - `None` 代表輸入為空，或不是單一字元。
fn parse_bookmark_argument(args: &str) -> Option<char> {
    let trimmed = args.trim();
    let mut chars = trimmed.chars();
    let key = chars.next()?;
    if chars.next().is_some() || key.is_whitespace() {
        return None;
    }
    Some(key)
}

/// 從命令列參數中取出 pane 編號。
///
/// 參數：
/// - `args: &str`，使用者在 `:move-panel ...` 後輸入的內容。
///
/// 回傳：`Option<usize>`。
/// - `Some(usize)` 代表成功解析成有效編號。
/// - `None` 代表輸入為空、不是數字或小於 1。
fn parse_pane_id_argument(args: &str) -> Option<usize> {
    let trimmed = args.trim();
    let id = trimmed.parse::<usize>().ok()?;
    (id > 0).then_some(id)
}

/// 判斷目前 command mode 輸入看起來是不是一條目錄或檔案路徑。
fn looks_like_navigation_path(input: &str) -> bool {
    let trimmed = input.trim();
    !trimmed.is_empty()
        && (trimmed.starts_with('/')
            || trimmed.starts_with("~/")
            || trimmed.starts_with("~\\")
            || trimmed == "~"
            || trimmed.starts_with("./")
            || trimmed.starts_with(".\\")
            || trimmed.starts_with("../")
            || trimmed.starts_with("..\\")
            || is_windows_drive_path(trimmed)
            || is_unc_path(trimmed))
}

/// 判斷字串是否為 Windows 磁碟機開頭的絕對路徑，例如 `C:/work` 或 `D:\\repo`。
fn is_windows_drive_path(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

/// 判斷字串是否為 UNC 路徑，例如 `\\\\server\\share` 或 `//server/share`。
fn is_unc_path(input: &str) -> bool {
    input.starts_with("\\\\") || input.starts_with("//")
}

/// 展開 `~` 開頭的家目錄路徑，讓 command mode 也能直接輸入家目錄捷徑。
fn expand_tilde_path(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed == "~" {
        return command_home_dir().map(|home| home.to_string_lossy().to_string());
    }

    let suffix = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))?;
    let home = command_home_dir()?;
    let mut path = home.to_string_lossy().to_string();
    if !path.ends_with(std::path::MAIN_SEPARATOR) {
        path.push(std::path::MAIN_SEPARATOR);
    }
    path.push_str(suffix);
    Some(path)
}

/// 取得 command mode 需要用來展開 `~` 的使用者家目錄。
fn command_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 描述 `g` 面板可快速跳轉的系統常用目錄。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoSpecialDirectory {
    Documents,
    Desktop,
}

impl GoSpecialDirectory {
    /// 回傳狀態列與提示訊息會使用的目錄名稱。
    ///
    /// 參數：無。
    /// 回傳：`&'static str`。
    fn label(self) -> &'static str {
        match self {
            Self::Documents => "Documents",
            Self::Desktop => "Desktop",
        }
    }

    /// 回傳相對於使用者家目錄的預設子目錄名稱。
    ///
    /// 參數：無。
    /// 回傳：`&'static str`。
    fn relative_name(self) -> &'static str {
        match self {
            Self::Documents => "Documents",
            Self::Desktop => "Desktop",
        }
    }
}

/// 根據目前平台的使用者家目錄，推算常用系統目錄位置。
///
/// 設計上先統一使用家目錄底下的標準資料夾名稱，
/// 讓 macOS 與 Windows 都能走同一套邏輯，之後若要擴充其他平台也容易集中修改。
///
/// 參數：
/// - `directory: GoSpecialDirectory`，要解析的常用目錄種類。
///
/// 回傳：`Option<PathBuf>`。
/// - 有找到家目錄時回傳完整路徑。
/// - 若環境沒有提供家目錄資訊則回傳 `None`。
fn special_directory_path(directory: GoSpecialDirectory) -> Option<PathBuf> {
    let home = command_home_dir()?;
    Some(home.join(directory.relative_name()))
}

/// 判斷 regex 批次改名某一列目前屬於可改名、無變化還是無效名稱。
fn classify_regex_rename_preview(original_name: &str, new_name: &str) -> RegexRenameOutcome {
    if new_name == original_name {
        return RegexRenameOutcome::Unchanged;
    }
    if new_name.is_empty()
        || new_name == "."
        || new_name == ".."
        || new_name.contains('/')
        || new_name.contains('\\')
    {
        return RegexRenameOutcome::Invalid;
    }
    RegexRenameOutcome::Ready
}

/// 將 regex 批次改名預覽轉成 pane 可直接顯示的列表內容。
fn regex_rename_panel_lines(previews: &[RegexRenamePreview]) -> Vec<RegexRenamePanelLine> {
    previews
        .iter()
        .map(|preview| RegexRenamePanelLine {
            original_name: preview.original_name.clone(),
            new_name: preview.new_name.clone(),
            status: match preview.outcome {
                RegexRenameOutcome::Ready => String::from("ready"),
                RegexRenameOutcome::Unchanged => String::from("unchanged"),
                RegexRenameOutcome::Conflict => String::from("conflict"),
                RegexRenameOutcome::Invalid => String::from("invalid"),
            },
        })
        .collect()
}

/// 根據目前 preview 內容整理 regex 批次改名面板的狀態列文字。
fn regex_rename_status(
    pattern: &str,
    replacement: &str,
    previews: &[RegexRenamePreview],
) -> String {
    let ready = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
        .count();
    let unchanged = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Unchanged))
        .count();
    let conflicts = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Conflict))
        .count();
    let invalid = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Invalid))
        .count();
    format!(
        "rename-regex /{pattern}/ -> {replacement}  [ready:{ready} unchanged:{unchanged} conflict:{conflicts} invalid:{invalid}]"
    )
}

/// 產生一個不會和當前批次改名結果衝突的暫存路徑，供兩階段 rename 使用。
fn unique_regex_rename_temp_path(
    cwd: &std::path::Path,
    original_name: &str,
    index: usize,
    previews: &[RegexRenamePreview],
) -> PathBuf {
    let mut attempt = 0usize;
    loop {
        let candidate = cwd.join(format!(
            ".tfm-rename-regex-{index}-{attempt}-{original_name}"
        ));
        let used_as_target = previews
            .iter()
            .any(|preview| cwd.join(&preview.new_name) == candidate);
        if !candidate.exists() && !used_as_target {
            return candidate;
        }
        attempt += 1;
    }
}

/// 把終端送進來的按鍵事件轉成使用者真正想輸入的字元。
///
/// 這個 helper 主要用在命令列、rename、search 這類文字輸入框。
/// 某些終端在 macOS 上會把 `Shift+6` 回報成 `Char('6') + Shift`，
/// 若直接把底層字元寫進 buffer，就會得到 `6` 而不是 `^`。
fn typed_char_from_key(key: &KeyEvent) -> Option<char> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };

    if !key.modifiers.contains(KeyModifiers::SHIFT) {
        return Some(c);
    }

    Some(match c {
        'a'..='z' => c.to_ascii_uppercase(),
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '\\' => '|',
        '`' => '~',
        other => other,
    })
}

/// 判斷目前按鍵是否是沒有 modifier 的一般小寫命令鍵。
///
/// 這類按鍵主要用在 normal mode、panel 導航或 Vim 風格命令，
/// 目的是把「文字輸入」與「功能命令」分成兩條不同路徑處理。
fn key_matches_plain_letter(key: &KeyEvent, lowercase: char) -> bool {
    if key.code == KeyCode::Char(lowercase) && key.modifiers.is_empty() {
        return true;
    }

    if !key.modifiers.is_empty() {
        return false;
    }

    matches!(
        (lowercase, key.code),
        ('h', KeyCode::Left) | ('j', KeyCode::Down) | ('k', KeyCode::Up) | ('l', KeyCode::Right)
    )
}

/// 判斷某些終端送出的 `Shift+字母` 是否應視為大寫命令。
///
/// 參數：
/// - `key: &KeyEvent`，目前收到的鍵盤事件。
/// - `uppercase: char`，邏輯上希望匹配的大寫英文字母。
///
/// 回傳：`bool`。
/// - `true` 代表事件要視為這個大寫命令。
/// - `false` 代表不是。
fn key_matches_shifted_letter(key: &KeyEvent, uppercase: char) -> bool {
    let lower = uppercase.to_ascii_lowercase();
    key.code == KeyCode::Char(uppercase)
        || (key.code == KeyCode::Char(lower) && key.modifiers.contains(KeyModifiers::SHIFT))
}

/// 判斷目前按鍵是否應視為 `~`，支援不同終端可能回報的格式差異。
///
/// 常見情況：
/// - 直接回報 `Char('~')`
/// - 回報 `Char('`') + Shift`
fn key_matches_tilde(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('~')
        || (key.code == KeyCode::Char('`') && key.modifiers.contains(KeyModifiers::SHIFT))
}

/// 判斷某個英文字母命令是否要接受大小寫等價輸入。
///
/// 這主要用在 `y/n` 這種確認提示，或某些不區分大小寫的互動按鍵。
fn key_matches_letter_any_case(key: &KeyEvent, letter: char) -> bool {
    let lower = letter.to_ascii_lowercase();
    let upper = letter.to_ascii_uppercase();
    key_matches_plain_letter(key, lower) || key_matches_shifted_letter(key, upper)
}

/// 判斷 `Ctrl+字母` 指令，支援不同終端可能送出的大小寫字元格式。
fn key_matches_ctrl_letter(key: &KeyEvent, letter: char) -> bool {
    let lower = letter.to_ascii_lowercase();
    let upper = letter.to_ascii_uppercase();
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c == lower || c == upper)
}

/// 判斷 `Ctrl+Shift+字母` 指令，支援不同終端可能送出的大小寫字元格式。
fn key_matches_ctrl_shift_letter(key: &KeyEvent, letter: char) -> bool {
    let lower = letter.to_ascii_lowercase();
    let upper = letter.to_ascii_uppercase();
    key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
        && matches!(key.code, KeyCode::Char(c) if c == lower || c == upper)
}

/// 把 `Ctrl+數字` 轉成目標 pane 編號。
///
/// 目前規則：
/// - `Ctrl+1` 到 `Ctrl+9` 對應 pane 1..9
/// - `Ctrl+0` 對應 pane 10
fn ctrl_digit_target_pane_id(key: &KeyEvent) -> Option<usize> {
    if !key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }

    match key.code {
        KeyCode::Char('1') => Some(1),
        KeyCode::Char('2') => Some(2),
        KeyCode::Char('3') => Some(3),
        KeyCode::Char('4') => Some(4),
        KeyCode::Char('5') => Some(5),
        KeyCode::Char('6') => Some(6),
        KeyCode::Char('7') => Some(7),
        KeyCode::Char('8') => Some(8),
        KeyCode::Char('9') => Some(9),
        KeyCode::Char('0') => Some(10),
        _ => None,
    }
}

/// 把不帶修飾鍵的數字轉成目標 pane 編號，供多 pane 模式直接切換焦點。
///
/// 目前規則：
/// - `1` 到 `9` 對應 pane 1..9
/// - `0` 對應 pane 10
fn plain_digit_target_pane_id(key: &KeyEvent) -> Option<usize> {
    if !key.modifiers.is_empty() {
        return None;
    }

    match key.code {
        KeyCode::Char('1') => Some(1),
        KeyCode::Char('2') => Some(2),
        KeyCode::Char('3') => Some(3),
        KeyCode::Char('4') => Some(4),
        KeyCode::Char('5') => Some(5),
        KeyCode::Char('6') => Some(6),
        KeyCode::Char('7') => Some(7),
        KeyCode::Char('8') => Some(8),
        KeyCode::Char('9') => Some(9),
        KeyCode::Char('0') => Some(10),
        _ => None,
    }
}

/// 描述 command mode 補全候選目前要往前還是往後切換。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuggestionNavigation {
    Next,
    Previous,
}

/// 把不同終端可能送出的快捷鍵格式，統一轉成 command 補全的切換方向。
///
/// 目前支援：
/// - `Shift+N` / `Shift+P`
/// - `Ctrl+N` / `Ctrl+P`
/// - `Tab` / `Shift+Tab`
/// - `Down` / `Up`
///
/// 這樣就算不同 terminal 對 modifier 的回報格式不一致，
/// command mode 仍然至少有一組可用的候選切換方式。
fn command_suggestion_navigation(key: &KeyEvent) -> Option<SuggestionNavigation> {
    if key_matches_shifted_letter(key, 'N')
        || key_matches_ctrl_letter(key, 'n')
        || key.code == KeyCode::Tab
        || key.code == KeyCode::Down
    {
        return Some(SuggestionNavigation::Next);
    }

    if key_matches_shifted_letter(key, 'P')
        || key_matches_ctrl_letter(key, 'p')
        || key.code == KeyCode::BackTab
        || key.code == KeyCode::Up
    {
        return Some(SuggestionNavigation::Previous);
    }

    None
}

/// 根據目前書籤彈窗的內容，產生適合顯示在狀態列的提示文字。
fn bookmark_list_status(
    query: &str,
    count: usize,
    selected: usize,
    mode: BookmarkListMode,
    editing: bool,
) -> String {
    if editing {
        return match mode {
            BookmarkListMode::Jump => format!(
                "bookmark search: {} ({count})",
                if query.is_empty() { "all" } else { query }
            ),
            BookmarkListMode::Delete => format!(
                "bookmark delete search: {} ({count})",
                if query.is_empty() { "all" } else { query }
            ),
        };
    }

    if count == 0 {
        match mode {
            BookmarkListMode::Jump => {
                if query.is_empty() {
                    String::from("bookmark jump: empty")
                } else {
                    format!("bookmark jump: {} (0)", query)
                }
            }
            BookmarkListMode::Delete => {
                if query.is_empty() {
                    String::from("bookmark delete: empty")
                } else {
                    format!("bookmark delete: {} (0)", query)
                }
            }
        }
    } else {
        match mode {
            BookmarkListMode::Jump => format!(
                "bookmarks jump: {}/{} (j/k move, Enter open, f search, Esc close)",
                selected.saturating_add(1).min(count),
                count
            ),
            BookmarkListMode::Delete => format!(
                "bookmarks delete: {}/{} (press key or Enter delete, f search, Esc close)",
                selected.saturating_add(1).min(count),
                count
            ),
        }
    }
}

/// 根據目前 zoxide 面板內容，產生適合顯示在狀態列的提示文字。
fn zoxide_list_status(query: &str, count: usize, selected: usize, editing: bool) -> String {
    if editing {
        format!(
            "zoxide search: {} ({count})",
            if query.is_empty() { "all" } else { query }
        )
    } else if count == 0 {
        if query.is_empty() {
            String::from("zoxide: empty")
        } else {
            format!("zoxide: {} (0)", query)
        }
    } else {
        format!(
            "zoxide: {}/{} (j/k move, Enter open, f search, Esc close)",
            selected.saturating_add(1).min(count),
            count
        )
    }
}

/// 超過此大小的單批檔案工作一律放到背景，避免可感知的 TUI 停頓。
const BACKGROUND_FILE_JOB_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

/// 判斷 paste 是否應移出主執行緒。
///
/// 參數：`clipboard: &ClipboardState` 為來源批次；`target_dir: &Path` 為目的目錄。
/// 回傳：`bool`；來源包含目錄、目的地為網路磁碟，或來源檔案總大小達門檻時回傳
/// `true`。目錄一律在背景處理，避免為了判斷大小而在 UI thread 遞迴走訪內容。
fn paste_should_run_in_background(clipboard: &ClipboardState, target_dir: &Path) -> bool {
    is_probably_network_or_external_path(target_dir)
        || clipboard
            .entries
            .iter()
            .any(|entry| entry.source_path.is_dir())
        || clipboard
            .entries
            .iter()
            .filter_map(|entry| fs::metadata(&entry.source_path).ok())
            .map(|metadata| metadata.len())
            .try_fold(0u64, |total, size| total.checked_add(size))
            .is_none_or(|total| total >= BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
}

/// 判斷壓縮或解壓項目是否可能長時間占用 CPU／磁碟。
///
/// 參數：`entries: &[FileEntry]`，目前選取項目。
/// 回傳：`bool`；資料夾或總檔案大小達 8 MiB 時使用背景工作。
fn entries_should_run_in_background(entries: &[super::entry::FileEntry]) -> bool {
    entries.iter().any(|entry| entry.is_dir)
        || entries
            .iter()
            .map(|entry| entry.size)
            .try_fold(0u64, |total, size| total.checked_add(size))
            .is_none_or(|total| total >= BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
}

/// 以跨平台保守規則辨識可能產生長延遲的網路或外接 volume。
///
/// 參數：`path: &Path`，要寫入的目標。
/// 回傳：`bool`；Windows UNC 與 macOS `/Volumes/...` 回傳 `true`。Windows 映射磁碟
/// 無法只靠路徑可靠辨識，但大型來源仍會由大小門檻轉入背景。
fn is_probably_network_or_external_path(path: &Path) -> bool {
    let display = path.to_string_lossy();
    is_unc_path(&display)
        || display == "/Volumes"
        || display.starts_with("/Volumes/")
        || display.starts_with("/Volumes\\")
}

/// 在 worker 執行完整 paste 批次，不接觸 App 或任何可變 UI 狀態。
///
/// 參數：`clipboard: &ClipboardState` 為固定來源；`target_dir: &Path` 為目的目錄；
/// `overwrite: bool` 表示是否覆蓋。
/// 回傳：`PasteJobResult`，包含成功的 Undo 資料及第一個失敗項目。
fn perform_paste_job<F>(
    clipboard: &ClipboardState,
    target_dir: &Path,
    overwrite: bool,
    progress: &mut F,
) -> PasteJobResult
where
    F: FnMut(TransferProgress),
{
    let mut history_items = Vec::new();
    let mut pasted_count = 0usize;

    for entry in &clipboard.entries {
        if entry.source_path.parent() == Some(target_dir)
            && clipboard.operation == ClipboardOperation::Cut
        {
            continue;
        }

        let planned_target =
            PaneState::planned_paste_target_in_dir(&entry.source_path, target_dir, overwrite)
                .unwrap_or_else(|_| target_dir.join(&entry.display_name));
        let result = match clipboard.operation {
            ClipboardOperation::Copy => PaneState::copy_path_to_dir_with_history_progress(
                &entry.source_path,
                target_dir,
                overwrite,
                progress,
            ),
            ClipboardOperation::Cut => PaneState::move_path_to_dir_with_history_progress(
                &entry.source_path,
                target_dir,
                overwrite,
                progress,
            ),
        };
        match result {
            Ok(outcome) => {
                pasted_count += 1;
                history_items.push(OperationItem {
                    source_path: entry.source_path.clone(),
                    destination_path: outcome.target_path,
                    replaced_backup: outcome.backup_path,
                });
            }
            Err(error) => {
                return PasteJobResult {
                    history_items,
                    pasted_count,
                    failure: Some(PasteJobFailure {
                        display_name: entry.display_name.clone(),
                        planned_target,
                        error,
                    }),
                };
            }
        }
    }

    PasteJobResult {
        history_items,
        pasted_count,
        failure: None,
    }
}

/// 平行計算目前 panel 每個直接子目錄的真實內容大小。
///
/// 參數：
/// - `directories: Vec<PathBuf>`，目前 panel 的直接子目錄；每個路徑各自成為一列。
/// - `cancelled: &AtomicBool`，切換 linemode、目錄或重啟掃描時由主執行緒設為 `true`。
/// - `sender: &mpsc::Sender<DirectorySizeEvent>`，把部分值與最終值送回 TUI。
///
/// 回傳：`() `。工作數受 CPU 平行度限制；目錄少時會把額度用於同一棵大型
/// 子樹，目錄多時則會同時計算多列。無法讀取的項目會略過，且不追蹤 symlink，
/// 避免循環目錄與意外走訪其他 share。各列最多約每 200ms 回報一次，完成時一定回報
/// 精確終值。
fn scan_directory_sizes(
    directories: Vec<PathBuf>,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<DirectorySizeEvent>,
) {
    if directories.is_empty() {
        return;
    }

    let available_workers = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 12);
    let root_workers = available_workers.min(directories.len());
    let threads_per_root = (available_workers / root_workers).max(1);
    let next_root = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..root_workers {
            let directories = &directories;
            let next_root = &next_root;
            scope.spawn(move || {
                loop {
                    if cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let index = next_root.fetch_add(1, Ordering::Relaxed);
                    let Some(root) = directories.get(index) else {
                        return;
                    };
                    scan_one_directory_size(root, cancelled, sender, threads_per_root);
                }
            });
        }
    });
}

/// 使用受限制的平行 walker 計算單一直接子目錄。
///
/// 參數：`root` 是列表中的直接子目錄；`cancelled` 是取消旗標；`sender` 將部分與
/// 最終 byte 傳回 TUI；`worker_threads` 是這棵子樹可使用的最大執行緒數。
///
/// 回傳：`() `。每個一般檔案只累加 `metadata.len()`；目錄本身與 symbolic link 不計入，
/// 因此結果代表內容的 logical bytes，不是檔案系統的磁碟配置空間。
fn scan_one_directory_size(
    root: &Path,
    cancelled: &AtomicBool,
    sender: &mpsc::Sender<DirectorySizeEvent>,
    worker_threads: usize,
) {
    let total_bytes = AtomicU64::new(0);
    let last_update_ms = AtomicU64::new(0);
    let started_at = Instant::now();
    let mut walker = WalkBuilder::new(root);
    walker
        .hidden(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .ignore(false)
        .follow_links(false)
        .threads(worker_threads.max(1));

    walker.build_parallel().run(|| {
        Box::new(|result| {
            if cancelled.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            let Some(file_type) = entry.file_type() else {
                return WalkState::Continue;
            };
            if !file_type.is_file() {
                return WalkState::Continue;
            }
            let Ok(metadata) = entry.metadata() else {
                return WalkState::Continue;
            };
            let bytes = total_bytes
                .fetch_add(metadata.len(), Ordering::Relaxed)
                .saturating_add(metadata.len());
            let elapsed_ms = started_at.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let previous_update = last_update_ms.load(Ordering::Relaxed);
            if should_report_directory_size(elapsed_ms, previous_update)
                && last_update_ms
                    .compare_exchange(
                        previous_update,
                        elapsed_ms,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok()
            {
                let _ = sender.send(DirectorySizeEvent::Update {
                    path: root.to_path_buf(),
                    bytes,
                    complete: false,
                });
            }
            WalkState::Continue
        })
    });

    if !cancelled.load(Ordering::Relaxed) {
        let _ = sender.send(DirectorySizeEvent::Update {
            path: root.to_path_buf(),
            bytes: total_bytes.load(Ordering::Relaxed),
            complete: true,
        });
    }
}

/// 判斷目錄容量部分結果是否已到下一次回報時間。
///
/// 參數：`elapsed_ms: u64` 是掃描啟動後的毫秒數；`previous_update_ms: u64` 是上次回報時間。
/// 回傳：`bool`；間隔達 200ms 時為 `true`，否則為 `false`。
fn should_report_directory_size(elapsed_ms: u64, previous_update_ms: u64) -> bool {
    elapsed_ms.saturating_sub(previous_update_ms) >= DIRECTORY_SIZE_UPDATE_INTERVAL_MS
}

/// byte 進度發生變化時送出事件；呼叫端負責以時間節流，避免大量 channel 訊息。
///
/// 參數：`sender` 為 worker event channel；`task_id` 為工作編號；`completed_bytes` 與
/// `total_bytes` 為累計量；`last_progress` 保存上次送出的 byte 組合。
/// 回傳：`() `；channel 已關閉時安靜停止回報，不影響檔案工作的錯誤處理。
fn send_progress_if_changed(
    sender: &mpsc::Sender<FileJobEvent>,
    task_id: usize,
    completed_bytes: u64,
    total_bytes: u64,
    last_progress: &mut Option<(u64, u64)>,
) {
    let progress = (completed_bytes, total_bytes.max(completed_bytes));
    if *last_progress == Some(progress) {
        return;
    }
    *last_progress = Some(progress);
    let _ = sender.send(FileJobEvent::Progress {
        task_id,
        completed_bytes: progress.0,
        total_bytes: progress.1,
    });
}

fn ensure_path_writable(path: &Path) {
    if let Ok(mut perms) = fs::metadata(path).map(|m| m.permissions()) {
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

/// 移除單一檔案或符號連結，遇到權限受阻時自動嘗試解除唯讀後重試，並回傳釋放的 byte 數。
fn remove_file_or_symlink_with_retry(path: &Path) -> io::Result<u64> {
    let size = fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0);
    if let Err(err) = fs::remove_file(path) {
        if err.kind() == io::ErrorKind::PermissionDenied {
            ensure_path_writable(path);
            if let Some(parent) = path.parent() {
                ensure_path_writable(parent);
            }
            fs::remove_file(path)?;
            return Ok(size);
        }
        if err.kind() != io::ErrorKind::NotFound {
            return Err(err);
        }
    }
    Ok(size)
}

const DELETE_WORKERS: usize = 8;

/// 高速遞迴刪除子目錄或檔案，遇到唯讀權限受阻時自動嘗試排除。
fn remove_dir_all_fast_recursive<F>(path: &Path, on_progress: &mut F) -> io::Result<()>
where
    F: FnMut(u64),
{
    ensure_path_writable(path);
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        if let Ok(file_type) = entry.file_type() {
            if file_type.is_dir() && !file_type.is_symlink() {
                let _ = remove_dir_all_fast_recursive(&entry_path, on_progress);
            } else {
                let size = remove_file_or_symlink_with_retry(&entry_path).unwrap_or(0);
                on_progress(size);
            }
        } else {
            let size = remove_file_or_symlink_with_retry(&entry_path).unwrap_or(0);
            on_progress(size);
        }
    }
    ensure_path_writable(path);
    if let Err(_) = fs::remove_dir(path) {
        let _ = fs::remove_dir_all(path);
    }
    Ok(())
}

/// 多執行緒平行刪除目錄，大幅提高 NVMe/SSD 與檔案系統的 unlink 吞吐量並回報 byte 進度。
fn remove_dir_all_parallel_with_progress<F>(path: &Path, on_progress: &mut F) -> io::Result<()>
where
    F: FnMut(u64) + Send,
{
    ensure_path_writable(path);
    let read_dir = match fs::read_dir(path) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let entries: Vec<PathBuf> = read_dir.flatten().map(|e| e.path()).collect();
    if entries.is_empty() {
        let _ = fs::remove_dir(path);
        return Ok(());
    }

    if entries.len() <= 4 {
        for child in &entries {
            if child.is_dir() && !child.is_symlink() {
                let _ = remove_dir_all_fast_recursive(child, on_progress);
            } else {
                let size = remove_file_or_symlink_with_retry(child).unwrap_or(0);
                on_progress(size);
            }
        }
    } else {
        let chunk_size = (entries.len() + DELETE_WORKERS - 1) / DELETE_WORKERS;
        let progress_mutex = std::sync::Mutex::new(on_progress);
        thread::scope(|scope| {
            for chunk in entries.chunks(chunk_size) {
                let chunk = chunk.to_vec();
                let p_mutex = &progress_mutex;
                scope.spawn(move || {
                    let mut local_bytes = 0u64;
                    let mut local_progress = |increment: u64| {
                        local_bytes = local_bytes.saturating_add(increment);
                        if local_bytes >= 1024 * 512 {
                            if let Ok(mut guard) = p_mutex.lock() {
                                guard(local_bytes);
                            }
                            local_bytes = 0;
                        }
                    };
                    for child in chunk {
                        if child.is_dir() && !child.is_symlink() {
                            let _ = remove_dir_all_fast_recursive(&child, &mut local_progress);
                        } else {
                            let size = remove_file_or_symlink_with_retry(&child).unwrap_or(0);
                            local_progress(size);
                        }
                    }
                    if local_bytes > 0 {
                        if let Ok(mut guard) = p_mutex.lock() {
                            guard(local_bytes);
                        }
                    }
                });
            }
        });
    }

    ensure_path_writable(path);
    if let Err(_) = fs::remove_dir(path) {
        // 若仍有殘留項目（例如 macOS Finder 動態寫入的 .DS_Store 或特殊屬性），
        // 執行最終保底清理，確保 100% 清空
        let _ = fs::remove_dir_all(path);
    }
    if path.exists() {
        ensure_path_writable(path);
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// 建立 paste 成功後的狀態文字，讓同步與背景流程使用相同規則。
///
/// 參數：`operation` 為 copy/cut；`overwrite` 表示覆蓋；`count` 為成功數量。
/// 回傳：`String`，可直接顯示於狀態列並寫入 task detail。
fn paste_success_status(operation: ClipboardOperation, overwrite: bool, count: usize) -> String {
    match operation {
        ClipboardOperation::Copy if overwrite && count == 1 => {
            String::from("pasted copy with overwrite: 1 item")
        }
        ClipboardOperation::Copy if overwrite => {
            format!("pasted copy with overwrite: {count} items")
        }
        ClipboardOperation::Copy if count == 1 => String::from("pasted copy: 1 item"),
        ClipboardOperation::Copy => format!("pasted copy: {count} items"),
        ClipboardOperation::Cut if overwrite && count == 1 => {
            String::from("moved with overwrite: 1 item")
        }
        ClipboardOperation::Cut if overwrite => format!("moved with overwrite: {count} items"),
        ClipboardOperation::Cut if count == 1 => String::from("moved: 1 item"),
        ClipboardOperation::Cut => format!("moved: {count} items"),
    }
}

/// 建立貼上失敗時供 status area 顯示的完整診斷訊息。
///
/// 第一行只描述失敗的來源項目，讓使用者能快速辨識是哪一筆操作；第二行保留完整
/// destination 與作業系統錯誤。UI 會依終端寬度自動換行，因此 UNC/SMB 長路徑即使
/// 需要三行以上也不會遺失尾端最重要的 OS error。
///
/// 參數：
/// - `source_name: &str`，貼上來源的顯示名稱。
/// - `destination: &Path`，本次操作實際預計使用的完整目標路徑。
/// - `error: &io::Error`，底層檔案系統或作業系統回傳的原始錯誤。
///
/// 回傳：`String`，含明確換行及完整診斷資訊的狀態文字。
fn paste_failure_status(source_name: &str, destination: &Path, error: &io::Error) -> String {
    format!(
        "paste failed for {source_name}\ndestination: {} | OS error: {error}",
        destination.display()
    )
}

/// 描述底部快捷鍵列中的一組高頻操作提示。
///
/// 欄位：
/// - `key: &'static str`，實際要按下的快捷鍵。
/// - `label: &'static str`，簡短的英文功能名稱。
///
/// 清單本身的順序就是顯示優先度；畫面不足時只會捨棄尾端低優先項目，避免把
/// `~/F1 help` 等重要入口裁掉，或顯示只剩一半的快捷鍵說明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusShortcutHint {
    key: &'static str,
    label: &'static str,
}

/// 回傳底部 status bar 允許顯示的高頻快捷鍵，並依實際使用優先度排序。
///
/// 參數：無。
/// 回傳：`&'static [StatusShortcutHint]`，第一筆固定為 Help；低頻功能應留在 F1
/// 完整說明，不要加入這份清單讓 status bar 重新變得難以閱讀。
fn status_shortcut_hints() -> &'static [StatusShortcutHint] {
    &[
        StatusShortcutHint {
            key: "~/F1",
            label: "help",
        },
        StatusShortcutHint {
            key: "hjkl",
            label: "move",
        },
        StatusShortcutHint {
            key: "Enter",
            label: "open",
        },
        StatusShortcutHint {
            key: "b",
            label: "bookmark",
        },
        StatusShortcutHint {
            key: "Tab",
            label: "preview",
        },
        StatusShortcutHint {
            key: "y",
            label: "copy",
        },
        StatusShortcutHint {
            key: "x",
            label: "cut",
        },
        StatusShortcutHint {
            key: "p/P",
            label: "paste/overwrite",
        },
        StatusShortcutHint {
            key: "v",
            label: "select",
        },
        StatusShortcutHint {
            key: "s/S",
            label: "search",
        },
        StatusShortcutHint {
            key: "f/F",
            label: "filter/fuzzy",
        },
        StatusShortcutHint {
            key: "r",
            label: "rename",
        },
        StatusShortcutHint {
            key: "a",
            label: "create",
        },
        StatusShortcutHint {
            key: "u",
            label: "undo",
        },
        StatusShortcutHint {
            key: "w",
            label: "panel",
        },
    ]
}

/// 依 terminal 寬度建立底部快捷鍵列，並把版本固定在最右側。
///
/// 參數：
/// - `width: u16`，目前快捷鍵列可使用的 terminal cell 寬度。
/// - `theme: Theme`，用來替按鍵套用目前主題的 accent 顏色。
///
/// 回傳：`Line<'static>`，可直接交給 ratatui `Paragraph` 繪製的單行內容。
fn status_shortcut_line(width: u16, theme: Theme) -> Line<'static> {
    let available_width = usize::from(width);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let version_width = version.len();
    let version_gap = 2usize;
    let mut used_width = 0usize;
    let mut spans = Vec::new();

    for (index, hint) in status_shortcut_hints().iter().enumerate() {
        let separator = if index == 0 { "" } else { "  " };
        let item_width = separator.len() + hint.key.len() + 1 + hint.label.len();
        let required_width = used_width
            .saturating_add(item_width)
            .saturating_add(version_gap)
            .saturating_add(version_width);
        if required_width > available_width {
            break;
        }

        if !separator.is_empty() {
            spans.push(Span::raw(separator));
        }
        spans.push(Span::styled(hint.key, theme.accent_style()));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(hint.label));
        used_width = used_width.saturating_add(item_width);
    }

    let padding_width = available_width.saturating_sub(used_width.saturating_add(version_width));
    if padding_width > 0 {
        spans.push(Span::raw(" ".repeat(padding_width)));
    }
    spans.push(Span::styled(version, theme.accent_style()));

    Line::from(spans)
}

/// 依終端 cell 寬度把 status 文字預先切成實際要繪製的多行內容。
///
/// 不直接使用 Rust 字串長度，因為中文等寬字元通常占兩個 terminal cell。預先換行後
/// 再交給 `Paragraph`，可確保高度計算與真正畫面使用完全相同的內容，也避免 ratatui
/// 私有的 rendered-line API。長路徑會按 cell 邊界切開，不會因為沒有空白而被截斷。
///
/// 參數：
/// - `status: &str`，準備顯示的完整狀態文字，可包含換行。
/// - `width: u16`，status area 可使用的終端欄寬。
///
/// 回傳：`String`，已插入必要換行、可直接交給 `Paragraph` 的文字。
fn wrap_status_text(status: &str, width: u16) -> String {
    let max_width = usize::from(width.max(1));
    let mut wrapped = Vec::new();

    for logical_line in status.split('\n') {
        let mut current = String::new();
        let mut current_width = 0usize;

        for character in logical_line.chars() {
            let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
            if current_width > 0 && current_width.saturating_add(character_width) > max_width {
                wrapped.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(character);
            current_width = current_width.saturating_add(character_width);
        }
        wrapped.push(current);
    }

    wrapped.join("\n")
}

/// 計算已換行 status 內容應占用的畫面高度。
///
/// 參數：
/// - `wrapped_status: &str`，經 `wrap_status_text` 處理後的狀態文字。
/// - `max_height: u16`，扣除主列表最低高度與快捷鍵區後可使用的最大高度。
///
/// 回傳：`u16`，至少一行且不超過可用畫面的 status area 高度。
fn status_area_height(wrapped_status: &str, max_height: u16) -> u16 {
    let required = wrapped_status.split('\n').count().max(1) as u16;
    required.min(max_height.max(1))
}

/// 判斷狀態列文字是否代表錯誤或目前操作無法執行。
///
/// 參數：
/// - `status: &str`，目前要顯示在畫面底部的狀態訊息。
///
/// 回傳：`bool`。
/// - `true` 代表應使用主題的危險色顯示。
/// - `false` 代表一般通知，維持預設文字顏色。
///
/// 這裡集中判斷訊息前綴，避免在每一個產生錯誤的操作中額外傳遞 UI 顏色狀態。
fn status_is_error(status: &str) -> bool {
    let normalized = status.trim().to_ascii_lowercase();
    [
        "error",
        "failed",
        "invalid",
        "usage:",
        "unknown",
        "cannot",
        "nothing selected",
        "panel no longer exists",
        "paste failed",
        "rename-regex: resolve conflicts",
        "rename-regex: nothing to apply",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
}

/// 描述 help 面板中某一列按下 Enter 後要執行的行為。
#[derive(Clone, Copy)]
enum HelpAction {
    Command(&'static str),
    Delete,
    Filter,
    FuzzyFilter,
    Sort,
    Hidden,
    Visual,
    QuitHint,
}

/// 描述 help 面板中完整的一筆資料。
#[derive(Clone)]
struct HelpEntry {
    line: HelpPanelLine,
    action: HelpAction,
}

/// 先依搜尋條件過濾 trash 原始資料，再提供給面板使用。
fn trash_panel_entries(trash_store: &TrashStore, query: &str) -> io::Result<Vec<TrashListEntry>> {
    let entries = trash_store.list_entries()?;
    Ok(fuzzy_matched_indices_by_fields(&entries, query, |entry| {
        vec![
            entry.display_name.clone(),
            entry.original_path.display().to_string(),
        ]
    })
    .into_iter()
    .map(|index| entries[index].clone())
    .collect())
}

/// 將目前 trash store 中的項目轉成面板可直接顯示的列內容。
fn trash_panel_lines(
    trash_store: &TrashStore,
    query: &str,
    marked_ids: &[String],
    visual_range: Option<(usize, usize)>,
) -> io::Result<Vec<TrashPanelLine>> {
    Ok(trash_panel_entries(trash_store, query)?
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let visually_selected = visual_range
                .map(|(start, end)| {
                    let range_start = start.min(end);
                    let range_end = start.max(end);
                    index >= range_start && index <= range_end
                })
                .unwrap_or(false);
            TrashPanelLine {
                name: entry.display_name,
                original_path: entry.original_path.display().to_string(),
                deleted_at: format_deleted_at(entry.deleted_at_unix_ms),
                marked: marked_ids.iter().any(|id| id == &entry.id) || visually_selected,
            }
        })
        .collect())
}

/// 依照搜尋條件過濾 F1 功能面板的完整資料。
fn help_entries(query: &str) -> Vec<HelpEntry> {
    let entries = vec![
        help_entry(
            ":rename",
            "r",
            "重新命名目前選取的檔案或資料夾",
            HelpAction::Command("rename"),
        ),
        help_entry(
            ":rename-regex",
            "R",
            "對目前選取或標記項目建立 regex 批次改名預覽；也可使用 :reg，預覽顯示 ready 才能套用",
            HelpAction::Command("rename-regex"),
        ),
        help_entry(
            ":create",
            "a",
            "建立新檔案、資料夾或巢狀路徑",
            HelpAction::Command("create"),
        ),
        help_entry(
            ":jump",
            "z",
            "用 fzf 遞迴掃描目前 panel 的目錄樹，快速挑選檔案或資料夾後直接跳過去",
            HelpAction::Command("jump"),
        ),
        help_entry(
            ":zoxide",
            "Z",
            "打開 zoxide 目錄列表，依照常用頻率快速跳到歷史工作目錄",
            HelpAction::Command("zoxide"),
        ),
        help_entry(
            ":goto <path>",
            "gt",
            "讓目前 panel 直接跳到指定路徑，支援相對路徑、絕對路徑、Windows 磁碟機路徑與 smb:// share",
            HelpAction::Command("goto "),
        ),
        help_entry(
            ":goto document",
            "gd",
            "快速跳到 Documents 目錄",
            HelpAction::Command("goto ~/Documents"),
        ),
        help_entry(
            ":goto desktop",
            "gk",
            "快速跳到 Desktop 目錄",
            HelpAction::Command("goto ~/Desktop"),
        ),
        help_entry(
            ":bookmark",
            "b",
            "打開書籤功能面板，可選擇自動儲存、列表跳轉、刪除單筆或清空全部書籤",
            HelpAction::Command("bookmark"),
        ),
        help_entry(
            ":diff",
            ":diff",
            "開啟全螢幕多 Panel 目錄矩陣與檔案內容差異比對 (N-Way Diff Matrix)",
            HelpAction::Command("diff"),
        ),
        help_entry(
            ":bookmark add",
            "ba",
            "自動挑選下一個可用代號，把目前 panel 的位置存成書籤",
            HelpAction::Command("bookmark add"),
        ),
        help_entry(
            ":bookmark jump",
            "bg/'{key}",
            "用列表挑選要跳去的書籤，或直接用單鍵快速跳轉",
            HelpAction::Command("bookmark jump"),
        ),
        help_entry(
            ":bookmark list",
            "bg",
            "列出目前可用的書籤清單，Enter 或 l 直接跳過去",
            HelpAction::Command("bookmark list"),
        ),
        help_entry(
            ":bookmark delete",
            "bd",
            "打開書籤刪除列表，可按對應按鍵或 Enter 刪除單筆書籤",
            HelpAction::Command("bookmark delete"),
        ),
        help_entry(
            ":bookmark clear",
            "bD",
            "直接刪除全部書籤",
            HelpAction::Command("bookmark clear"),
        ),
        help_entry(
            ":linemode",
            "m",
            "打開 linemode 面板，改變列表右側欄位顯示；目前支援 size、permissions、btime、mtime、none",
            HelpAction::Command("linemode "),
        ),
        help_entry(
            ":linemode size",
            "ms",
            "將列表右側欄位切成 size；資料夾在背景遞迴計算真實容量，檔案顯示大小",
            HelpAction::Command("linemode size"),
        ),
        help_entry(
            ":linemode permissions",
            "mp",
            "將列表右側欄位切成 permissions 顯示",
            HelpAction::Command("linemode permissions"),
        ),
        help_entry(
            ":linemode btime",
            "mb",
            "將列表右側欄位切成 btime 顯示",
            HelpAction::Command("linemode btime"),
        ),
        help_entry(
            ":linemode mtime",
            "mt",
            "將列表右側欄位切成 mtime 顯示",
            HelpAction::Command("linemode mtime"),
        ),
        help_entry(
            ":linemode none",
            "mn",
            "關閉 linemode，回到由排序方式決定的右側欄位顯示",
            HelpAction::Command("linemode none"),
        ),
        help_entry(
            ":copy",
            "y",
            "複製目前選取項目到內部剪貼簿",
            HelpAction::Command("copy"),
        ),
        help_entry(
            ":copy-picker",
            "c",
            "打開文字複製小視窗，可快速複製檔案路徑、目錄路徑、檔名或無副檔名檔名",
            HelpAction::Command("copy-picker"),
        ),
        help_entry(
            ":copy file-path",
            "cu",
            "打開 Copy 面板後複製目前項目的完整檔案路徑",
            HelpAction::Command("copy-picker"),
        ),
        help_entry(
            ":copy directory-path",
            "cd",
            "打開 Copy 面板後複製目前項目的所在目錄路徑；若本身是資料夾就複製該資料夾路徑",
            HelpAction::Command("copy-picker"),
        ),
        help_entry(
            ":copy filename",
            "cf",
            "打開 Copy 面板後只複製目前項目的檔名",
            HelpAction::Command("copy-picker"),
        ),
        help_entry(
            ":copy filename-without-extension",
            "cn",
            "打開 Copy 面板後複製去掉副檔名的檔名",
            HelpAction::Command("copy-picker"),
        ),
        help_entry(
            ":mark toggle",
            "Space",
            "切換目前游標所在項目的標記狀態，方便逐項多選",
            HelpAction::Command("mark-toggle"),
        ),
        help_entry(
            ":mark-all",
            "Ctrl-a",
            "把目前 panel 中所有可見的檔案與資料夾全部標記起來，方便批次操作",
            HelpAction::Command("mark-all"),
        ),
        help_entry(
            ":mark-invert",
            "Ctrl-r",
            "反轉目前 panel 所有可見項目的標記狀態",
            HelpAction::Command("mark-invert"),
        ),
        help_entry(
            ":unmark-all",
            "Ctrl-Shift-a",
            "清掉目前 panel 內所有已標記項目",
            HelpAction::Command("unmark-all"),
        ),
        help_entry(
            ":cut",
            "x",
            "剪下目前選取項目到內部剪貼簿",
            HelpAction::Command("cut"),
        ),
        help_entry(
            ":move <path>",
            "",
            "直接把目前選取或標記的項目移動到指定目錄",
            HelpAction::Command("move "),
        ),
        help_entry(
            ":move-panel",
            "",
            "把目前選取或標記的項目移動到指定 panel 編號目前所在的目錄",
            HelpAction::Command("move-panel "),
        ),
        help_entry(
            ":panel <id>",
            "1..9 / 0, Ctrl-p",
            "多 panel 時可直接按數字切換焦點；也可打開 panel 切換命令輸入指定編號",
            HelpAction::Command("panel "),
        ),
        help_entry(
            ":paste",
            "p",
            "貼上剪貼簿項目到目前目錄；若遇到同名項目，會先詢問是否整批覆蓋",
            HelpAction::Command("paste"),
        ),
        help_entry(
            ":paste!",
            "P",
            "貼上剪貼簿項目到目前目錄；若同名已存在就直接覆蓋，不會再詢問",
            HelpAction::Command("paste!"),
        ),
        help_entry(
            ":undo",
            "u",
            "復原最近一次完整 copy 或 move 批次；可連續執行，copy 建立物會移入 trash",
            HelpAction::Command("undo"),
        ),
        help_entry(
            ":cancel copied",
            "Y",
            "清掉目前內部剪貼簿中的 copy 狀態，不影響檔案本身",
            HelpAction::Command("cancel-copy"),
        ),
        help_entry(
            ":cancel cut",
            "X",
            "清掉目前內部剪貼簿中的 cut 狀態，不影響檔案本身",
            HelpAction::Command("cancel-cut"),
        ),
        help_entry(
            ":compress",
            "C",
            "把目前選取或標記的項目壓成 zip；多選時預設檔名為 archive.zip",
            HelpAction::Command("compress"),
        ),
        help_entry(
            ":extract",
            "E",
            "解開目前選取或標記的壓縮檔，支援 zip、tar.gz、tar、gz",
            HelpAction::Command("extract"),
        ),
        help_entry(
            ":open",
            "o/Enter",
            "用預設外部方式打開目前選取項目；文字檔走 $EDITOR，其他交給系統",
            HelpAction::Command("open"),
        ),
        help_entry(
            ":open-picker",
            "O/Shift-Enter",
            "打開 Open with 小視窗，手動選擇 Editor、Vim、Open 或 Reveal",
            HelpAction::Command("open-picker"),
        ),
        help_entry(
            ":vim",
            "",
            "直接用 vim 打開目前選取的檔案或目錄",
            HelpAction::Command("vim"),
        ),
        help_entry(
            ":reveal",
            "",
            "在系統檔案管理器中顯示目前選取的檔案或目錄",
            HelpAction::Command("reveal"),
        ),
        help_entry(
            ":delete",
            "d",
            "將目前選取項目移到 trash，並顯示確認提示",
            HelpAction::Delete,
        ),
        help_entry(
            ":delete!",
            "D",
            "永久刪除目前選取或標記項目，會先顯示確認提示",
            HelpAction::Command("delete!"),
        ),
        help_entry(
            ":trash",
            "tt",
            "打開 trash 面板，查看已移入 trash 的項目，並用 d/D/u/U 操作",
            HelpAction::Command("trash"),
        ),
        help_entry(
            ":tasks",
            "T",
            "打開目前 panel 的任務面板；x 取消選取任務，X 取消所有可取消任務",
            HelpAction::Command("tasks"),
        ),
        help_entry(
            ":status",
            "",
            "顯示 fd、fzf、rg、zoxide 是否已安裝並可從系統 PATH 使用",
            HelpAction::Command("status"),
        ),
        help_entry(
            ":trash undo",
            "tu",
            "快速還原最近一次移到 trash 的檔案或資料夾",
            HelpAction::Command("trash undo"),
        ),
        help_entry(
            ":trash panel actions",
            "trash:d, trash:D, trash:u, trash:U",
            "在 trash 面板中刪除單筆、刪除全部、還原單筆或還原全部；都會先顯示確認視窗",
            HelpAction::Command("trash"),
        ),
        help_entry(
            ":search",
            "s",
            "用 fd 遞迴搜尋檔名與路徑；結果列表可按 f 做模糊過濾",
            HelpAction::Command("search"),
        ),
        help_entry(
            ":search-content",
            "S",
            "用 rg 遞迴搜尋檔案內容；結果列表可按 f 做模糊過濾",
            HelpAction::Command("search-content"),
        ),
        help_entry(
            ":preview-search",
            "/",
            "在 preview 內容中搜尋文字",
            HelpAction::Command("preview-search"),
        ),
        help_entry(
            ":preview",
            "Tab",
            "切換 preview mode；平常隱藏 preview，開啟後用整個 panel 顯示內容",
            HelpAction::Command("preview"),
        ),
        help_entry(
            ":split",
            "wj",
            "在目前 panel 下方建立新的 panel",
            HelpAction::Command("split"),
        ),
        help_entry(
            ":vsplit",
            "wl",
            "在目前 panel 右側建立新的 panel",
            HelpAction::Command("vsplit"),
        ),
        help_entry(
            ":split-up",
            "wk",
            "在目前 panel 上方建立新的 panel",
            HelpAction::Command("split-up"),
        ),
        help_entry(
            ":split-left",
            "wh",
            "在目前 panel 左側建立新的 panel",
            HelpAction::Command("split-left"),
        ),
        help_entry(
            ":close",
            "wc",
            "關閉目前 panel",
            HelpAction::Command("close"),
        ),
        help_entry(
            ":only",
            "wo",
            "只保留目前 panel",
            HelpAction::Command("only"),
        ),
        help_entry(
            ":terminal",
            "wt",
            "在 active panel 目前目錄開啟新終端；Windows 會繼承 PaneFM 的安全權杖與環境",
            HelpAction::Command("terminal"),
        ),
        help_entry(
            ":theme list",
            "tl",
            "打開主題列表；游標會停在目前使用中的主題",
            HelpAction::Command("theme list"),
        ),
        help_entry(
            ":theme next",
            "tn",
            "直接切到下一個主題",
            HelpAction::Command("theme next"),
        ),
        help_entry(
            ":help",
            "~/F1",
            "打開這個功能說明面板",
            HelpAction::Command("help"),
        ),
        help_entry(
            ":filter",
            "f",
            "開啟一般子字串過濾（可於輸入框按 Tab 切換模糊模式）",
            HelpAction::Filter,
        ),
        help_entry(
            ":filter fuzzy",
            "F",
            "開啟模糊搜尋過濾（Fuzzy filter，依相關性評分排序）",
            HelpAction::FuzzyFilter,
        ),
        help_entry(":sort", ",", "打開排序方式快捷鍵面板", HelpAction::Sort),
        help_entry(
            ":sort modified",
            ",m",
            "依修改時間正序排序",
            HelpAction::Sort,
        ),
        help_entry(
            ":sort modified reverse",
            ",M",
            "依修改時間倒序排序",
            HelpAction::Sort,
        ),
        help_entry(":sort birth", ",b", "依建立時間正序排序", HelpAction::Sort),
        help_entry(
            ":sort birth reverse",
            ",B",
            "依建立時間倒序排序",
            HelpAction::Sort,
        ),
        help_entry(
            ":sort alphabetical",
            ",a",
            "依字母順序正序排序",
            HelpAction::Sort,
        ),
        help_entry(
            ":sort alphabetical reverse",
            ",A",
            "依字母順序倒序排序",
            HelpAction::Sort,
        ),
        help_entry(
            ":sort natural",
            ",n",
            "依自然順序正序排序",
            HelpAction::Sort,
        ),
        help_entry(
            ":sort natural reverse",
            ",N",
            "依自然順序倒序排序",
            HelpAction::Sort,
        ),
        help_entry(
            ":sort extension",
            ",e",
            "依副檔名正序排序",
            HelpAction::Sort,
        ),
        help_entry(
            ":sort extension reverse",
            ",E",
            "依副檔名倒序排序",
            HelpAction::Sort,
        ),
        help_entry(":sort size", ",s", "依檔案大小正序排序", HelpAction::Sort),
        help_entry(
            ":sort size reverse",
            ",S",
            "依檔案大小倒序排序",
            HelpAction::Sort,
        ),
        help_entry(":sort random", ",r", "隨機排序目前列表", HelpAction::Sort),
        help_entry(":hidden", ".", "切換是否顯示隱藏檔", HelpAction::Hidden),
        help_entry(
            ":visual",
            "v",
            "進入視覺範圍標記模式，使用 j/k 移動，再按 v 或 Esc 結束",
            HelpAction::Visual,
        ),
        help_entry(
            ":quit",
            "q",
            "離開 terminal file manager",
            HelpAction::QuitHint,
        ),
    ];

    fuzzy_matched_indices_by_fields(&entries, query, |entry| {
        vec![
            entry.line.command.clone(),
            entry.line.shortcut.clone(),
            entry.line.description.clone(),
        ]
    })
    .into_iter()
    .map(|index| entries[index].clone())
    .collect()
}

/// 只取出 help 面板渲染需要的列內容。
fn help_panel_lines(query: &str) -> Vec<HelpPanelLine> {
    help_entries(query)
        .into_iter()
        .map(|entry| entry.line)
        .collect()
}

/// 根據解壓結果數量與略過項目數，整理出適合顯示在狀態列的訊息。
fn extraction_status_label(extracted: &[ExtractedArchive], skipped: usize) -> String {
    if extracted.len() == 1 {
        let output_name = extracted[0]
            .output_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("output");
        if skipped == 0 {
            format!("extracted {output_name}")
        } else {
            format!("extracted {output_name} (skipped {skipped})")
        }
    } else if skipped == 0 {
        format!("extracted {} archives", extracted.len())
    } else {
        format!("extracted {} archives (skipped {skipped})", extracted.len())
    }
}

/// 根據目前 command mode 的輸入內容，整理出適合顯示的補全候選。
fn command_suggestions_for_buffer(
    base_dir: Option<&Path>,
    query: &str,
) -> Vec<CommandSuggestionLine> {
    if let Some(context) = command_path_completion_context(base_dir, query) {
        return path_completion_suggestions(&context);
    }
    command_suggestions(query)
}

/// 根據目前 command mode 的輸入內容，整理出適合顯示的命令補全候選。
fn command_suggestions(query: &str) -> Vec<CommandSuggestionLine> {
    let trimmed = query.trim();
    let mut suggestions = Vec::new();
    let mut seen = BTreeSet::new();

    for entry in help_entries("") {
        let HelpAction::Command(command) = entry.action else {
            continue;
        };
        if !(trimmed.is_empty()
            || command.starts_with(trimmed)
            || command
                .split_whitespace()
                .next()
                .is_some_and(|head| head.starts_with(trimmed)))
        {
            continue;
        }
        if !seen.insert(command.to_string()) {
            continue;
        }
        suggestions.push(CommandSuggestionLine {
            command: command.to_string(),
            display_command: entry.line.command,
            shortcut: entry.line.shortcut,
            description: entry.line.description,
        });
    }

    if trimmed.chars().count() > 1 {
        suggestions.sort_by(|left, right| {
            command_suggestion_sort_key(trimmed, &left.command)
                .cmp(&command_suggestion_sort_key(trimmed, &right.command))
        });
    }
    suggestions.truncate(8);

    suggestions
}

/// 計算 command 補全候選的排序鍵，讓較接近使用者輸入的指令排在前面。
///
/// 目前會優先比較：
/// 1. 指令第一段名稱和查詢字串的長度差距
/// 2. 指令第一段名稱的字母順序
/// 3. 完整命令模板，作為最後的穩定排序條件
fn command_suggestion_sort_key(query: &str, command: &str) -> (usize, String, String) {
    let head = command.split_whitespace().next().unwrap_or(command);
    let remainder = head.chars().count().saturating_sub(query.chars().count());
    (remainder, head.to_string(), command.to_string())
}

/// 找出多個候選字串的最長共同前綴，供路徑補全先延伸到共享部分。
fn longest_common_prefix(values: &[&str]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };

    let mut prefix = (*first).to_string();
    for value in values.iter().skip(1) {
        let mut shared = String::new();
        for (left, right) in prefix.chars().zip(value.chars()) {
            if left != right {
                break;
            }
            shared.push(left);
        }
        prefix = shared;
        if prefix.is_empty() {
            break;
        }
    }
    prefix
}

/// 描述 command mode 中一次路徑補全需要的上下文資訊。
struct CommandPathCompletionContext {
    replacement_prefix: String,
    typed_directory: String,
    search_dir: PathBuf,
    partial_name: String,
    preferred_separator: char,
    /// `true` 代表路徑會觸發 UNC/SMB 網路存取，不可在 command render 時同步掃描。
    network_path: bool,
}

/// 若目前 command buffer 正在輸入路徑，整理出路徑補全所需的上下文。
fn command_path_completion_context(
    base_dir: Option<&Path>,
    query: &str,
) -> Option<CommandPathCompletionContext> {
    let base_dir = base_dir?;
    let (replacement_prefix, raw_path) = if let Some(path) = query.strip_prefix("goto ") {
        (String::from("goto "), path)
    } else if looks_like_navigation_path(query) {
        (String::new(), query.trim())
    } else {
        return None;
    };

    let preferred_separator = if raw_path.contains('\\') { '\\' } else { '/' };
    let (typed_directory, partial_name) = split_typed_path(raw_path);
    let expanded_directory = expand_tilde_path(&typed_directory).unwrap_or(typed_directory.clone());
    let search_dir = if expanded_directory.is_empty() {
        base_dir.to_path_buf()
    } else {
        let expanded_path = PathBuf::from(&expanded_directory);
        if expanded_path.is_absolute()
            || is_windows_drive_path(&expanded_directory)
            || is_unc_path(&expanded_directory)
        {
            expanded_path
        } else {
            base_dir.join(expanded_path)
        }
    };

    Some(CommandPathCompletionContext {
        replacement_prefix,
        typed_directory,
        search_dir,
        partial_name,
        preferred_separator,
        network_path: is_unc_path(raw_path) || raw_path.trim_start().starts_with("smb://"),
    })
}

/// 依照目前的路徑補全上下文，建立 command palette 要顯示的候選列表。
fn path_completion_suggestions(
    context: &CommandPathCompletionContext,
) -> Vec<CommandSuggestionLine> {
    // command suggestions 會在每次按鍵與每次 render 時計算；若這裡讀取 UNC，失聯
    // 主機會把 TUI 主執行緒鎖住數十秒。網路路徑只在 Enter 後交給背景 goto。
    if context.network_path {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(&context.search_dir) else {
        return Vec::new();
    };

    let partial_lower = context.partial_name.to_ascii_lowercase();
    let mut candidates = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let file_type = entry.file_type().ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !partial_lower.is_empty() && !name.to_ascii_lowercase().starts_with(&partial_lower) {
                return None;
            }

            let mut completed = format!("{}{}", context.typed_directory, name);
            if file_type.is_dir() {
                completed.push(context.preferred_separator);
            }
            let mut display_name = name;
            if file_type.is_dir() {
                display_name.push(context.preferred_separator);
            }

            Some((
                file_type.is_dir(),
                display_name.to_ascii_lowercase(),
                CommandSuggestionLine {
                    command: format!("{}{}", context.replacement_prefix, completed),
                    display_command: display_name,
                    shortcut: String::new(),
                    description: String::new(),
                },
            ))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));

    candidates
        .into_iter()
        .map(|(_, _, suggestion)| suggestion)
        .take(8)
        .collect()
}

/// 將使用者目前輸入的路徑拆成「父目錄前綴」與「最後一段正在輸入的名稱」。
fn split_typed_path(input: &str) -> (String, String) {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }
    if trimmed.ends_with('/') || trimmed.ends_with('\\') {
        return (trimmed.to_string(), String::new());
    }

    let slash_index = trimmed.rfind(['/', '\\']);
    match slash_index {
        Some(index) => (
            trimmed[..=index].to_string(),
            trimmed[index + 1..].to_string(),
        ),
        None => (String::new(), trimmed.to_string()),
    }
}

/// 建立單一功能說明列與其動作。
fn help_entry(command: &str, shortcut: &str, description: &str, action: HelpAction) -> HelpEntry {
    HelpEntry {
        line: HelpPanelLine {
            command: command.to_string(),
            shortcut: shortcut.to_string(),
            description: description.to_string(),
        },
        action,
    }
}

/// 產生 trash 面板底部狀態列訊息。
fn trash_panel_status(
    query: &str,
    count: usize,
    selected: usize,
    editing: bool,
    marked_count: usize,
) -> String {
    if editing {
        if query.is_empty() {
            format!("trash search: all ({count})")
        } else {
            format!("trash search: {query} ({count})")
        }
    } else if count == 0 {
        String::from("trash: empty")
    } else {
        format!(
            "trash: {}/{} [marked: {}] (Enter/u restore, U all, d delete, D all, V mark, f search)",
            selected + 1,
            count,
            marked_count
        )
    }
}

/// 產生說明面板底部狀態列訊息。
fn help_panel_status(query: &str, count: usize, editing: bool) -> String {
    if editing {
        format!(
            "help search: {} ({count})",
            if query.is_empty() { "all" } else { query }
        )
    } else if query.is_empty() {
        format!("help: {count} commands (f to search)")
    } else {
        format!("help: {} ({count})", query)
    }
}

/// 產生 task 面板底部狀態列訊息。
fn task_panel_status(query: &str, count: usize, selected: usize, editing: bool) -> String {
    if editing {
        format!(
            "task search: {} ({count})",
            if query.is_empty() { "all" } else { query }
        )
    } else if count == 0 {
        if query.is_empty() {
            String::from("tasks: empty")
        } else {
            format!("tasks: {} (0)", query)
        }
    } else {
        format!(
            "tasks: {}/{} (j/k move, x cancel, X cancel all, f search, h close)",
            selected + 1,
            count
        )
    }
}

/// 依照本次貼上衝突的名稱與數量，產生覆蓋確認視窗的狀態列文字。
fn paste_overwrite_confirm_status(target_name: &str, entry_count: usize) -> String {
    if entry_count <= 1 {
        format!("confirm overwrite {target_name}: y/n")
    } else {
        format!("confirm overwrite {target_name} ({entry_count} items): y/n")
    }
}

/// 當使用者取消這次覆蓋貼上時，回傳狀態列要顯示的訊息。
fn paste_overwrite_cancelled_status(target_name: &str, entry_count: usize) -> String {
    if entry_count <= 1 {
        format!("paste cancelled: {target_name}")
    } else {
        format!("paste cancelled: {target_name} ({entry_count} items)")
    }
}

/// 依照 trash 確認操作種類，回傳確認視窗與狀態列要顯示的文字。
fn trash_confirm_status(
    action: &TrashConfirmAction,
    target_name: &str,
    entry_count: usize,
) -> String {
    let verb = match action {
        TrashConfirmAction::RestoreFromPanel { .. } => "restore",
        TrashConfirmAction::DeleteFromPanel { .. } => "delete",
    };
    if entry_count <= 1 {
        format!("confirm {verb} {target_name}: y/n")
    } else {
        format!("confirm {verb} {target_name} ({entry_count} items): y/n")
    }
}

/// 當使用者取消 trash 確認視窗時，回傳應顯示的狀態列訊息。
fn trash_confirm_cancelled_status(
    action: &TrashConfirmAction,
    target_name: &str,
    entry_count: usize,
) -> String {
    let verb = match action {
        TrashConfirmAction::RestoreFromPanel { .. } => "restore",
        TrashConfirmAction::DeleteFromPanel { .. } => "delete",
    };
    if entry_count <= 1 {
        format!("{verb} cancelled: {target_name}")
    } else {
        format!("{verb} cancelled: {target_name} ({entry_count} items)")
    }
}

/// 取出 trash 確認操作所屬的 panel 編號，讓確認視窗可以畫回原本的列表內。
fn trash_confirm_panel_id(action: &TrashConfirmAction) -> Option<usize> {
    match action {
        TrashConfirmAction::RestoreFromPanel { pane_id, .. }
        | TrashConfirmAction::DeleteFromPanel { pane_id, .. } => Some(*pane_id),
    }
}

/// 從 trash 確認操作還原出原本的 trash 面板狀態，讓取消或重繪時能留在同一個列表。
fn trash_panel_pending_action_from_confirm_action(
    action: &TrashConfirmAction,
    marked_ids: Vec<String>,
    visual_anchor: Option<usize>,
) -> PendingAction {
    match action {
        TrashConfirmAction::RestoreFromPanel {
            pane_id,
            search,
            selected,
            ..
        }
        | TrashConfirmAction::DeleteFromPanel {
            pane_id,
            search,
            selected,
            ..
        } => PendingAction::TrashPanel {
            pane_id: *pane_id,
            selected: *selected,
            search: search.clone(),
            marked_ids,
            visual_anchor,
        },
    }
}

/// 取出目前 pending action 對應的 trash 面板狀態，讓 confirm 視窗打開時底層仍可維持 trash 列表。
fn trash_panel_overlay_state_from_pending_action(
    pending_action: &Option<PendingAction>,
    pane_id: usize,
) -> Option<(usize, PanelSearchState, Vec<String>, Option<usize>)> {
    match pending_action {
        Some(PendingAction::TrashPanel {
            pane_id: action_pane_id,
            selected,
            search,
            marked_ids,
            visual_anchor,
        }) if *action_pane_id == pane_id => Some((
            *selected,
            search.clone(),
            marked_ids.clone(),
            *visual_anchor,
        )),
        Some(PendingAction::ConfirmTrashAction {
            action,
            marked_ids,
            visual_anchor,
            ..
        }) => match action {
            TrashConfirmAction::RestoreFromPanel {
                pane_id: action_pane_id,
                search,
                selected,
                ..
            }
            | TrashConfirmAction::DeleteFromPanel {
                pane_id: action_pane_id,
                search,
                selected,
                ..
            } if *action_pane_id == pane_id => Some((
                *selected,
                search.clone(),
                marked_ids.clone(),
                *visual_anchor,
            )),
            _ => None,
        },
        _ => None,
    }
}

/// 將目前 task log 轉成面板可直接渲染的資料列。
fn task_panel_lines(tasks: &[TaskRecord]) -> Vec<TaskPanelLine> {
    tasks
        .iter()
        .map(|task| TaskPanelLine {
            state: task_state_label(task.state).to_string(),
            started_at: format_task_time(task.started_at_unix_ms),
            finished_at: task
                .finished_at_unix_ms
                .map(format_task_time)
                .unwrap_or_else(|| String::from("--:--:--")),
            progress: task_progress_label(task),
            title: task.title.clone(),
            source_locations: task.source_locations.clone(),
            destination_location: task.destination_location.clone(),
            detail: task.detail.clone(),
        })
        .collect()
}

/// 產生 task 面板的 byte 進度欄，讓使用者直接判斷傳輸是否仍在前進。
///
/// 參數：`task: &TaskRecord`，要顯示的 task。
/// 回傳：`String`；支援進度的工作顯示 `24.4G / 77.2G`，一般外部工作顯示 `-`。
fn task_progress_label(task: &TaskRecord) -> String {
    match (task.completed_bytes, task.total_bytes) {
        (Some(completed), Some(total)) if total > 0 => format!(
            "{} / {}",
            format_task_bytes(completed),
            format_task_bytes(total.max(completed))
        ),
        (Some(completed), _) if completed > 0 => format_task_bytes(completed),
        (Some(0), _) => String::from("0B"),
        _ => String::from("-"),
    }
}

/// 把 byte 數量轉成 task 面板使用的緊湊單位。
///
/// 參數：`bytes: u64`，要顯示的原始 byte 數。
/// 回傳：`String`，依大小使用 `B`、`K`、`M`、`G` 或 `T`，並保留最多一位小數。
fn format_task_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value.fract() == 0.0 {
        format!("{value:.0}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// 將 task 狀態轉成簡短標籤。
fn task_state_label(state: TaskState) -> &'static str {
    match state {
        TaskState::Running => "RUNNING",
        TaskState::Done => "DONE",
        TaskState::Failed => "FAILED",
        TaskState::Cancelled => "CANCELLED",
        TaskState::Interrupted => "INTERRUPTED",
    }
}

/// 將 unix 毫秒時間轉成 task 面板使用的簡短時間。
fn format_task_time(unix_ms: u64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms))
        .format("%H:%M:%S")
        .to_string()
}

/// 取得目前系統時間的 unix 毫秒。
fn unix_time_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 將 unix 毫秒時間轉成較容易閱讀的本地時間字串。
fn format_deleted_at(unix_ms: u64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms))
        .format("%m/%d %H:%M")
        .to_string()
}

/// 回傳建立流程的狀態列內容，讓使用者知道目前正處於哪一種編輯模式。
fn create_status_label(mode: &str) -> String {
    format!("create entry: {mode}")
}

/// 依照目前 preview search 文字與命中數量產生狀態列訊息。
fn preview_search_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("preview search: all")
    } else {
        format!("preview search: {buffer} ({matches})")
    }
}

/// 依照目前列表內 find-next 文字與命中數量產生狀態列訊息。
fn list_find_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("find next: type query")
    } else {
        format!("find next: {buffer} ({matches})")
    }
}

/// 依照目前列表內 find-next 文字與命中數量產生鎖定後的狀態列訊息。
fn list_find_locked_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("find next: empty")
    } else {
        format!("find next locked: {buffer} ({matches})")
    }
}

/// 依照目前 global search 文字、結果數與模式，產生狀態列訊息。
fn global_search_status(
    mode: SearchMode,
    buffer: &str,
    matches: usize,
    editing: bool,
    searched: bool,
    loading: bool,
) -> String {
    let interaction_mode = if editing { "insert" } else { "normal" };
    let label = mode.status_label();
    if loading {
        format!("{label} ({interaction_mode}): loading...")
    } else if !searched {
        if buffer.is_empty() {
            format!("{label} ({interaction_mode}): type query and Enter")
        } else {
            format!("{label} ({interaction_mode}): {buffer} (press Enter to search)")
        }
    } else if buffer.is_empty() {
        format!("{label} ({interaction_mode}): all ({matches})")
    } else {
        format!("{label} ({interaction_mode}): {buffer} ({matches})")
    }
}

/// 回傳 global search 套用結果模糊 filter 後的可見筆數。
///
/// 參數：
/// - `search: &GlobalSearchState`，目前 `s` 或 `S` 搜尋面板的完整狀態。
///
/// 回傳：`usize`，目前可供游標移動與開啟的結果數量。
fn global_search_visible_len(search: &GlobalSearchState) -> usize {
    filtered_global_search_entries(&search.results, &search.filter.buffer).len()
}

/// 建立 filter 狀態列文字。
fn format_filter_status(filter: &FilterState) -> String {
    let mode_label = match filter.mode {
        FilterMode::Normal => "normal",
        FilterMode::Fuzzy => "fuzzy",
    };
    if filter.buffer.is_empty() {
        format!("filter [{mode_label}]: all (Tab to switch)")
    } else if filter.editing {
        format!("filter [{mode_label}]: {}", filter.buffer)
    } else {
        format!("filter locked [{mode_label}]: {}", filter.buffer)
    }
}

/// 依照 global search 的模糊 filter 狀態產生狀態列訊息。
///
/// 參數：
/// - `filter: &PanelSearchState`，filter 查詢與是否仍在輸入中的狀態。
/// - `matches: usize`，套用模糊 filter 後的可見結果數量。
///
/// 回傳：`String`，供狀態列顯示目前查詢、模式與命中數。
fn global_search_filter_status(filter: &PanelSearchState, matches: usize) -> String {
    let mode = if filter.editing { "insert" } else { "locked" };
    if filter.buffer.is_empty() {
        format!("fuzzy filter ({mode}): all ({matches})")
    } else {
        format!("fuzzy filter ({mode}): {} ({matches})", filter.buffer)
    }
}

/// 產生搜尋引擎缺少外部工具時的狀態列訊息。
///
/// 參數：
/// - `mode: SearchMode`，目前執行的是檔名搜尋或內容搜尋。
/// - `tool: &str`，缺少的外部工具名稱，例如 `fd` 或 `rg`。
///
/// 回傳：`String`，包含搜尋類型、工具名稱與 `:status` 操作提示。
fn missing_search_tool_status(mode: SearchMode, tool: &str) -> String {
    format!("{} requires {tool}; run :status", mode.status_label())
}

/// 把 `fzf` 回傳的相對路徑文字轉回實際檔案系統路徑。
fn jump_selection_to_path(root_dir: &PathBuf, selection: &str) -> PathBuf {
    let mut target = root_dir.clone();
    let trimmed = selection.trim_end_matches('/');
    for segment in trimmed.split('/').filter(|segment| !segment.is_empty()) {
        target.push(segment);
    }
    target
}

/// 依照編輯模式決定建立輸入框的標題文字。
fn create_editor_title(mode: RenameMode) -> &'static str {
    match mode {
        RenameMode::Insert => " Create (append / for dir): ",
        RenameMode::Normal => " Create (normal): ",
    }
}

/// 將字元插入到指定的字元游標位置，並在插入後把游標往右移一格。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的檔名字串。
/// - `cursor: &mut usize`，以字元數計算的游標位置。
/// - `ch: char`，要插入的新字元。
///
/// 回傳：`()`
fn insert_char(buffer: &mut String, cursor: &mut usize, ch: char) {
    let byte_index = char_to_byte_index(buffer, *cursor);
    buffer.insert(byte_index, ch);
    *cursor += 1;
}

/// 刪除游標左側的一個字元，行為對齊一般文字編輯器的 Backspace。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的檔名字串。
/// - `cursor: &mut usize`，以字元數計算的游標位置。
///
/// 回傳：`()`
fn backspace_char(buffer: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }

    let remove_at = *cursor - 1;
    let start = char_to_byte_index(buffer, remove_at);
    let end = char_to_byte_index(buffer, *cursor);
    buffer.replace_range(start..end, "");
    *cursor -= 1;
}

/// 刪除字元游標目前指向的字元，供共用輸入器處理 Delete 鍵。
///
/// 參數：
/// - `buffer: &mut String`，目前正在編輯的 UTF-8 字串。
/// - `cursor: usize`，以字元數表示的游標位置。
///
/// 回傳：`()`, 游標在字串尾端時不修改內容。
fn delete_char_at(buffer: &mut String, cursor: usize) {
    if cursor >= buffer.chars().count() {
        return;
    }
    let start = char_to_byte_index(buffer, cursor);
    let end = char_to_byte_index(buffer, cursor + 1);
    buffer.replace_range(start..end, "");
}

/// 把字元游標位置轉成 Rust 字串的位元組索引，方便做安全的字串編輯。
///
/// 參數：
/// - `text: &str`，來源字串。
/// - `char_index: usize`，以字元數表示的目標位置。
///
/// 回傳：`usize`，對應的位元組索引。
fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

/// 將游標往右移動一個字元，但不會超過字串末端。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，移動後的新游標位置。
fn move_cursor_right(buffer: &str, cursor: usize) -> usize {
    let end = buffer.chars().count();
    (cursor + 1).min(end)
}

/// 將 Insert 模式的插入點轉成 Normal 模式可指向的字元位置。
fn normal_cursor(buffer: &str, cursor: usize) -> usize {
    cursor.min(buffer.chars().count().saturating_sub(1))
}

/// 在 Normal 模式向右移動一格，但不會移到最後一個字元之外。
fn normal_move_right(buffer: &str, cursor: usize) -> usize {
    (cursor + 1).min(buffer.chars().count().saturating_sub(1))
}

/// 回傳 normal 模式中 `$` 應該停留的位置，也就是最後一個可見字元。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
///
/// 回傳：`usize`，normal 模式游標應停留的字元索引。
fn rename_line_end_cursor(buffer: &str) -> usize {
    buffer.chars().count().saturating_sub(1)
}

/// 判斷某個字元是否應該被視為檔名中的「單字內容」。
///
/// 參數：
/// - `ch: char`，要判斷的字元。
///
/// 回傳：`bool`。
/// - `true` 代表它屬於單字本體，例如英數字。
/// - `false` 代表它是分隔符，例如 `-`、`_`、`.` 或空白。
fn is_rename_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
}

/// 找出 normal 模式下 `w` 應跳到的位置，也就是下一段名稱的開頭。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，下一個單字的起始字元索引。
fn rename_next_word_start(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 0;
    }

    let mut index = cursor.min(len);
    if index < len && is_rename_word_char(chars[index]) {
        while index < len && is_rename_word_char(chars[index]) {
            index += 1;
        }
    }

    while index < len && !is_rename_word_char(chars[index]) {
        index += 1;
    }

    index.min(rename_line_end_cursor(buffer))
}

/// 找出 normal 模式下 `b` 應跳到的位置，也就是前一段名稱的開頭。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，前一個單字的起始字元索引。
fn rename_previous_word_start(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    if chars.is_empty() {
        return 0;
    }

    let mut index = cursor.min(chars.len().saturating_sub(1));
    if !is_rename_word_char(chars[index]) {
        while index > 0 && !is_rename_word_char(chars[index]) {
            index -= 1;
        }
        if index == 0 && !is_rename_word_char(chars[index]) {
            return 0;
        }
    } else if index > 0 {
        index -= 1;
        while index > 0 && !is_rename_word_char(chars[index]) {
            index -= 1;
        }
        if index == 0 && !is_rename_word_char(chars[index]) {
            return 0;
        }
    }

    while index > 0 && is_rename_word_char(chars[index - 1]) {
        index -= 1;
    }

    index
}

/// 找出 normal 模式下 `e` 應跳到的位置，也就是目前或下一段名稱的結尾。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，目標單字最後一個字元的索引。
fn rename_word_end(buffer: &str, cursor: usize) -> usize {
    let chars: Vec<char> = buffer.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 0;
    }

    let mut index = cursor.min(len.saturating_sub(1));
    while index < len && !is_rename_word_char(chars[index]) {
        index += 1;
    }

    if index >= len {
        return rename_line_end_cursor(buffer);
    }

    while index + 1 < len && is_rename_word_char(chars[index + 1]) {
        index += 1;
    }

    index
}

/// 找出 rename 一開始應該停留的位置，預設會停在副檔名前，方便先改主檔名。
///
/// 參數：
/// - `name: &str`，目前選取項目的原始名稱。
///
/// 回傳：`usize`，以字元數表示的初始游標位置。
fn rename_basename_cursor(name: &str) -> usize {
    let dot_index = name.rfind('.').filter(|index| *index > 0);
    match dot_index {
        Some(index) => name[..index].chars().count(),
        None => name.chars().count(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::sync::{Mutex, OnceLock};

    use tempfile::tempdir;

    use super::{
        App, BACKGROUND_FILE_JOB_THRESHOLD_BYTES, BookmarkListMode, ClipboardEntry,
        ClipboardOperation, ClipboardState, DirectoryLoadEvent, DirectoryLoadJob, FilterState,
        GlobalSearchState, ListFindState, PanelSearchState, PendingAction, RegexRenameOutcome,
        RenameMode, SearchMode, TaskRecord, TaskState, TrashConfirmAction, VisualSelectionState,
        bookmark_panel_lines, command_suggestion_navigation, command_suggestions,
        command_suggestions_for_buffer, ctrl_digit_target_pane_id, filtered_bookmark_entries,
        filtered_global_search_entries, help_entries, is_probably_network_or_external_path,
        is_windows_drive_path, key_matches_ctrl_letter, key_matches_ctrl_shift_letter,
        key_matches_letter_any_case, key_matches_plain_letter, key_matches_shifted_letter,
        looks_like_navigation_path, missing_search_tool_status, paste_should_run_in_background,
        plain_digit_target_pane_id, query_zoxide_directories, rename_basename_cursor,
        rename_next_word_start, rename_previous_word_start, rename_word_end, task_progress_label,
        trash_confirm_panel_id, trash_panel_overlay_state_from_pending_action, typed_char_from_key,
        visible_job_badge_paths,
    };
    use crate::{
        config::{
            ActionLaunchMode, ActionTargetScope, AppConfig, CustomOpenActionConfig, LoadedConfig,
            StartupSort,
        },
        file_manager::{
            bookmark::{BookmarkEntry, BookmarkTarget},
            layout::{LayoutNode, SplitDirection},
            open::{LaunchMode, OpenPickerAction},
            pane::{FilterMode, LineMode, PaneState, SortMode},
            search::{GlobalSearchEntry, GlobalSearchEvent},
        },
        theme::{Theme, ThemePreset},
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::{collections::BTreeSet, fs, sync::mpsc, thread, time::Duration};

    #[test]
    /// 驗證狀態列只會把錯誤類訊息判斷為危險色，一般通知不會被誤標紅。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn status_is_error_distinguishes_errors_from_notifications() {
        assert!(super::status_is_error("failed to open file"));
        assert!(super::status_is_error("usage: reg <pattern> <replace>"));
        assert!(super::status_is_error(
            "rename-regex: resolve conflicts before apply"
        ));
        assert!(!super::status_is_error("opened directory"));
        assert!(!super::status_is_error("rename-regex: renamed 2 items"));
        assert!(!super::status_is_error("trash cancelled: note.txt"));
    }

    #[test]
    /// 驗證底部快捷鍵列永遠先顯示 Help，再依照移動、開啟與書籤等使用頻率排列。
    /// 保護目的：避免新增命令時又依 command 定義順序插入提示，讓使用者在不知道按鍵時
    /// 看不到最重要的 `~/F1 help` 入口。
    fn status_shortcut_hints_keep_help_first_and_follow_usage_priority() {
        let hints = super::status_shortcut_hints();

        assert_eq!((hints[0].key, hints[0].label), ("~/F1", "help"));
        assert_eq!((hints[1].key, hints[1].label), ("hjkl", "move"));
        assert_eq!((hints[2].key, hints[2].label), ("Enter", "open"));
        assert_eq!((hints[3].key, hints[3].label), ("b", "bookmark"));
    }

    #[test]
    /// 驗證窄 terminal 只保留能完整放下的高優先快捷鍵，並固定保留右側版本號。
    /// 保護目的：多 panel 或小視窗會縮短 status bar；此測試避免重新出現只看得到半個
    /// 快捷鍵名稱，並確認寬畫面使用的是目前正確的 Tab preview 與 P 覆蓋貼上提示。
    fn status_shortcut_line_drops_low_priority_items_instead_of_clipping() {
        let theme = Theme::default_theme();
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let narrow = super::status_shortcut_line(31, theme);
        let narrow_text = narrow
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let wide = super::status_shortcut_line(u16::MAX, theme);
        let wide_text = wide
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(narrow_text.len(), 31);
        assert!(narrow_text.starts_with("~/F1 help"));
        assert!(narrow_text.ends_with(&version));
        assert!(wide_text.contains("Tab preview"));
        assert!(wide_text.contains("p/P paste/overwrite"));
        assert!(!wide_text.contains("P preview"));
        assert!(wide_text.ends_with(&version));

        let version_only = super::status_shortcut_line(version.len() as u16, theme);
        let version_only_text = version_only
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(version_only_text, version);
    }

    #[test]
    /// 驗證貼上錯誤會把摘要與完整診斷拆行，並保留 destination 及原始 OS error。
    /// 保護目的：避免 SMB/UNC 長路徑再次把最重要的錯誤尾端截掉，導致公司環境無法除錯。
    fn paste_failure_status_preserves_destination_and_os_error() {
        let destination =
            std::path::Path::new(r"\\server\shared\department\release\large-archive.zip");
        let error = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Access is denied. (os error 5)",
        );

        let status = super::paste_failure_status("large-archive.zip", destination, &error);
        let mut lines = status.lines();

        assert_eq!(lines.next(), Some("paste failed for large-archive.zip"));
        let detail = lines.next().expect("diagnostic detail line");
        assert!(detail.contains(destination.to_string_lossy().as_ref()));
        assert!(detail.contains("OS error: Access is denied. (os error 5)"));
        assert_eq!(lines.next(), None);
        assert!(super::status_is_error(&status));
    }

    #[test]
    /// 驗證長錯誤會依終端寬度增加 status area，高度不足時則遵守畫面上限。
    /// 保護目的：避免 layout 重構後又把 status 固定成一行，或讓錯誤區吃掉整個檔案列表。
    fn status_area_height_wraps_long_errors_and_preserves_short_notifications() {
        let long_error = concat!(
            "paste failed for archive.zip\n",
            "destination: \\\\server\\shared\\department\\release\\archive.zip | ",
            "OS error: The network name cannot be found. (os error 67)"
        );

        let short_status = super::wrap_status_text("opened directory", 80);
        let wrapped_error = super::wrap_status_text(long_error, 40);
        let narrow_error = super::wrap_status_text(long_error, 20);

        assert_eq!(super::status_area_height(&short_status, 20), 1);
        assert!(super::status_area_height(&wrapped_error, 20) >= 3);
        assert_eq!(super::status_area_height(&narrow_error, 2), 2);
        assert!(wrapped_error.contains("OS error"));
    }

    #[test]
    /// 驗證 status 換行使用 terminal cell 寬度，而不是 UTF-8 byte 或 Unicode 字元數。
    /// 保護目的：公司 SMB 路徑可能包含中文，必須避免配置高度不足而截掉錯誤內容。
    fn status_wrapping_accounts_for_wide_cjk_characters() {
        let wrapped = super::wrap_status_text("錯誤位置", 4);

        assert_eq!(wrapped, "錯誤\n位置");
        assert_eq!(super::status_area_height(&wrapped, 10), 2);
    }

    #[test]
    /// 驗證缺少搜尋工具時會顯示正確搜尋類型，並引導使用者打開 status 面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn missing_search_tool_status_names_mode_and_dependency_panel() {
        assert_eq!(
            missing_search_tool_status(SearchMode::Path, "fd"),
            "global search requires fd; run :status"
        );
        assert_eq!(
            missing_search_tool_status(SearchMode::Content, "rg"),
            "content search requires rg; run :status"
        );
    }

    #[test]
    /// 驗證檔名與內容搜尋標題會明確顯示用途及實際使用的外部工具。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn search_panel_titles_name_search_tool() {
        assert_eq!(
            SearchMode::Path.panel_title(true),
            " Global search file by fd "
        );
        assert_eq!(
            SearchMode::Content.panel_title(true),
            " Global search content by rg "
        );
        assert_eq!(
            SearchMode::Content.panel_title(false),
            " Global search content by rg "
        );
    }

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    /// 建立測試專用預設設定，避免每個 App 案例重複準備 config 與來源路徑。
    fn default_loaded_config() -> LoadedConfig {
        LoadedConfig {
            config: AppConfig::default(),
            source: None,
        }
    }

    /// 輪詢測試中的背景搜尋直到完成，並設定 timeout 防止失敗時無限等待。
    fn wait_for_global_search(app: &mut App) {
        for _ in 0..50 {
            app.poll_background_tasks();
            if app
                .global_search
                .as_ref()
                .is_some_and(|search| search.searched && !search.loading)
            {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("global search did not complete in time");
    }

    /// 輪詢測試中的大型檔案工作直到全部完成，避免測試直接依賴執行緒排程速度。
    ///
    /// 保護目的：paste/compress/extract 已移出主執行緒；測試必須驗證 task 完成事件
    /// 確實回到 App，而不是以固定 sleep 掩蓋偶發競態。
    fn wait_for_file_jobs(app: &mut App) {
        for _ in 0..200 {
            app.poll_background_tasks();
            if app.file_job_receivers.is_empty() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("background file job did not complete in time");
    }

    #[test]
    /// 驗證文字輸入 helper 會把 `Shift+6` 這類終端事件正規化成真正的符號字元。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn typed_char_from_key_normalizes_shifted_symbols() {
        assert_eq!(
            typed_char_from_key(&KeyEvent::new(KeyCode::Char('6'), KeyModifiers::SHIFT)),
            Some('^')
        );
        assert_eq!(
            typed_char_from_key(&KeyEvent::new(KeyCode::Char('-'), KeyModifiers::SHIFT)),
            Some('_')
        );
        assert_eq!(
            typed_char_from_key(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT)),
            Some('A')
        );
    }

    #[test]
    /// 驗證功能型按鍵 helper 會接受常見的 terminal 事件變體。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn key_normalization_helpers_accept_terminal_variants() {
        assert!(key_matches_plain_letter(
            &KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            'h'
        ));
        assert!(key_matches_plain_letter(
            &KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            'j'
        ));
        assert!(key_matches_plain_letter(
            &KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            'k'
        ));
        assert!(key_matches_plain_letter(
            &KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            'l'
        ));
        assert!(key_matches_shifted_letter(
            &KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT),
            'N'
        ));
        assert!(key_matches_shifted_letter(
            &KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE),
            'N'
        ));
        assert!(key_matches_ctrl_letter(
            &KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            'p'
        ));
        assert!(key_matches_ctrl_letter(
            &KeyEvent::new(
                KeyCode::Char('P'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            'p'
        ));
        assert!(key_matches_ctrl_shift_letter(
            &KeyEvent::new(
                KeyCode::Char('A'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            'a'
        ));
        assert!(key_matches_letter_any_case(
            &KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            'y'
        ));
        assert!(key_matches_letter_any_case(
            &KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE),
            'y'
        ));
    }

    #[test]
    /// 驗證 `Ctrl+數字` 會正確轉成 pane 編號，供 pane 快速切換功能共用。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn ctrl_digit_target_pane_id_maps_to_expected_panes() {
        assert_eq!(
            ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL)),
            Some(1)
        );
        assert_eq!(
            ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('9'), KeyModifiers::CONTROL)),
            Some(9)
        );
        assert_eq!(
            ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('0'), KeyModifiers::CONTROL)),
            Some(10)
        );
        assert_eq!(
            ctrl_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    /// 驗證不帶修飾鍵的數字會正確轉成 pane 編號，供多 pane 直接切換焦點使用。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn plain_digit_target_pane_id_maps_to_expected_panes() {
        assert_eq!(
            plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
            Some(1)
        );
        assert_eq!(
            plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('9'), KeyModifiers::NONE)),
            Some(9)
        );
        assert_eq!(
            plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE)),
            Some(10)
        );
        assert_eq!(
            plain_digit_target_pane_id(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    /// 驗證 command 補全的切換快捷鍵支援多種常見 terminal 回報格式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn command_suggestion_navigation_accepts_terminal_variants() {
        assert_eq!(
            command_suggestion_navigation(&KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT)),
            Some(super::SuggestionNavigation::Next)
        );
        assert_eq!(
            command_suggestion_navigation(&KeyEvent::new(
                KeyCode::Char('N'),
                KeyModifiers::CONTROL
            )),
            Some(super::SuggestionNavigation::Next)
        );
        assert_eq!(
            command_suggestion_navigation(&KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Some(super::SuggestionNavigation::Next)
        );
        assert_eq!(
            command_suggestion_navigation(&KeyEvent::new(KeyCode::Char('p'), KeyModifiers::SHIFT)),
            Some(super::SuggestionNavigation::Previous)
        );
        assert_eq!(
            command_suggestion_navigation(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(super::SuggestionNavigation::Previous)
        );
    }

    #[test]
    /// 驗證 command mode 也會把 `Shift+6` 正規化成 `^`，避免 regex 指令難以輸入。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_accepts_shifted_caret_symbol() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        app.handle_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::SHIFT))
            .expect("type caret");

        assert_eq!(app.command_buffer, "^");
    }

    #[test]
    /// 驗證 `Ctrl+p` 會打開 command UI，並預先填入 `panel ` 方便直接輸入目標編號。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_p_opens_prefilled_panel_command() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
            .expect("open prefilled panel command");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "panel ");
        assert_eq!(app.status, "command mode");
    }

    #[test]
    /// 驗證 normal mode 按下 `R` 會打開預填好的 `rename-regex ` 命令輸入框。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_r_opens_prefilled_rename_regex_command() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
            .expect("open prefilled rename-regex command");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "rename-regex ");
        assert_eq!(app.status, "command mode");
    }

    #[test]
    /// 驗證 normal mode 按下第一個 `g` 會先打開 `g` 系列命令面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_g_opens_go_picker() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("open go picker");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::GoPicker { pane_id: 1 })
        ));
        assert_eq!(app.status, "go: choose g/t/d/k from the panel");
    }

    #[test]
    /// 驗證 normal mode 按下 `gt` 會先經過 `g` 面板，再打開預填好的 `goto ` 命令輸入框。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_gt_opens_prefilled_goto_command() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("pending g");
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("open prefilled goto command");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "goto ");
        assert_eq!(app.status, "command mode");
    }

    #[test]
    /// 驗證 `gd` 會直接切到使用者的 Documents 目錄。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_gd_jumps_to_documents_directory() {
        let _guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock");
        let dir = tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let documents = home.join("Documents");
        fs::create_dir_all(&documents).expect("documents");

        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("USERPROFILE", &home);
        }

        let result = (|| {
            let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
            app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
                .expect("open go picker");
            app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
                .expect("jump documents");
            assert_eq!(app.panes.get(&1).expect("pane").cwd, documents);
        })();

        unsafe {
            match original_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match original_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }

        result
    }

    #[test]
    /// 驗證 `gk` 會直接切到使用者的 Desktop 目錄。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_gk_jumps_to_desktop_directory() {
        let _guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock");
        let dir = tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let desktop = home.join("Desktop");
        fs::create_dir_all(&desktop).expect("desktop");

        let original_home = std::env::var_os("HOME");
        let original_userprofile = std::env::var_os("USERPROFILE");
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("USERPROFILE", &home);
        }

        let result = (|| {
            let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
            app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
                .expect("open go picker");
            app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
                .expect("jump desktop");
            assert_eq!(app.panes.get(&1).expect("pane").cwd, desktop);
        })();

        unsafe {
            match original_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match original_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }

        result
    }

    #[test]
    /// 驗證 command mode 遇到看起來像路徑的輸入時，Enter 會直接執行，不會先套用補全建議。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_enter_executes_path_like_input_instead_of_autocomplete() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.command_mode = true;
        app.command_buffer = String::from("C:/");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute path-like input");

        assert!(!app.command_mode);
        assert!(app.command_buffer.is_empty());
        assert!(app.status.starts_with("path jump failed: C:/"));
    }

    #[test]
    /// 驗證 `:goto <path>` 會讓目前 pane 跳到指定子目錄。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_goto_command_changes_to_target_directory() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        fs::create_dir(&docs).expect("docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("goto docs").expect("goto command");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
        assert_eq!(app.status, format!("jumped to path: {}", docs.display()));
    }

    #[test]
    /// 驗證直接輸入絕對路徑也能跳到目標目錄，不必一定寫 `:goto`。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bare_path_command_changes_directory() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        fs::create_dir(&docs).expect("docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command(&docs.display().to_string())
            .expect("bare path command");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
    }

    #[test]
    /// 驗證 Windows 磁碟機路徑會被當成絕對路徑，而不是相對於目前目錄拼接。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn windows_drive_path_is_recognized_as_absolute_like_path() {
        assert!(is_windows_drive_path("C:/"));
        assert!(is_windows_drive_path("D:\\work"));
        assert!(looks_like_navigation_path("R:/repo"));
        assert!(!is_windows_drive_path("docs/readme"));
    }

    #[test]
    /// 驗證 Windows UNC 與 macOS `/Volumes` 目的地都會被視為背景傳輸目標。
    ///
    /// 保護目的：兩個正式支援平台使用不同網路路徑形式；若任一形式漏判，大檔案貼上
    /// 就可能退回主執行緒並再次凍結 TUI。
    fn network_destination_detection_covers_windows_and_macos() {
        assert!(is_probably_network_or_external_path(std::path::Path::new(
            "//server/share"
        )));
        assert!(is_probably_network_or_external_path(std::path::Path::new(
            "/Volumes/company/share"
        )));
        assert!(!is_probably_network_or_external_path(std::path::Path::new(
            "/Users/otto/Documents"
        )));
    }

    #[test]
    /// 驗證 command mode 在輸入路徑時，會改成列出目前目錄下的路徑候選。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn command_suggestions_switch_to_path_completion_candidates() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("draft.md"), "draft").expect("draft");

        let suggestions = command_suggestions_for_buffer(Some(dir.path()), "goto d");

        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].command, "goto docs/");
        assert_eq!(suggestions[0].display_command, "docs/");
        assert!(suggestions[0].shortcut.is_empty());
        assert!(suggestions[0].description.is_empty());
    }

    #[test]
    /// 驗證 UNC 路徑輸入期間不會建立需要讀取網路目錄的即時補全候選。
    ///
    /// 保護目的：command suggestion 會在按鍵與 render 階段反覆計算；若對
    /// `//server/share` 呼叫 `read_dir`，Windows 遇到失聯主機時會凍結整個 TUI。
    /// 網路路徑必須等 Enter 後交給背景 goto，不能在使用者仍輸入時碰檔案系統。
    fn command_suggestions_do_not_scan_unc_paths() {
        let dir = tempdir().expect("tempdir");

        let unc_suggestions =
            command_suggestions_for_buffer(Some(dir.path()), "goto //192.0.2.10/share");
        let smb_suggestions =
            command_suggestions_for_buffer(Some(dir.path()), "goto smb://192.0.2.10/share");

        assert!(unc_suggestions.is_empty());
        assert!(smb_suggestions.is_empty());
    }

    #[test]
    /// 驗證 UNC goto 的 loader 即使尚未完成，啟動函式仍會立即把控制權交回 TUI。
    ///
    /// 保護目的：公司網路主機不存在或 SMB 回應緩慢時，Windows 檔案系統呼叫可能
    /// 等待很久；此測試用 channel 人為暫停 worker，確認主執行緒仍可用 Esc 取消，
    /// 且取消後晚到的結果不會覆蓋原本 panel。
    fn app_unc_goto_runs_in_background_and_escape_discards_late_result() {
        let dir = tempdir().expect("tempdir");
        let original_cwd = dir.path().to_path_buf();
        let mut app = App::new(original_cwd.clone(), default_loaded_config()).expect("app");
        let target = std::path::PathBuf::from("//192.0.2.10/share");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        app.start_network_goto_with(target, move |mut pane, target| {
            started_tx.send(()).expect("report worker started");
            release_rx.recv().expect("release worker");
            pane.cwd = target;
            Ok(pane)
        })
        .expect("start background goto");

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("background loader should start without blocking caller");
        assert!(app.active_network_goto_task_id.is_some());

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel network goto");
        release_tx.send(()).expect("finish stale worker");
        thread::sleep(Duration::from_millis(10));
        app.poll_background_tasks();

        assert!(app.active_network_goto_task_id.is_none());
        assert_eq!(app.panes.get(&1).expect("pane").cwd, original_cwd);
        assert!(matches!(
            app.task_log.last().map(|task| task.state),
            Some(TaskState::Cancelled)
        ));
    }

    #[test]
    /// 驗證 command mode 在路徑補全模式下按 Tab，會直接把目前候選補進輸入框。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_tab_autocompletes_path_candidate() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        for ch in "goto d".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type path command");
        }

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("autocomplete path");

        assert_eq!(app.command_buffer, "goto docs/");
    }

    #[test]
    /// 驗證多個路徑候選存在時，第一次 Tab 會先補到最長共同前綴。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_tab_completes_longest_common_path_prefix_first() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::create_dir(dir.path().join("downloads")).expect("downloads");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        for ch in "goto d".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type path command");
        }

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("complete common prefix");

        assert_eq!(app.command_buffer, "goto do");
    }

    #[test]
    /// 驗證共同前綴補滿後，連按 Tab 會在同一組路徑候選間輪流切換。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_tab_cycles_path_candidates_after_common_prefix() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::create_dir(dir.path().join("downloads")).expect("downloads");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        for ch in "goto do".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type path command");
        }

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("cycle to first candidate");
        let first = app.command_buffer.clone();

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("cycle to second candidate");
        let second = app.command_buffer.clone();

        assert_eq!(first, "goto docs/");
        assert_eq!(second, "goto downloads/");
    }

    #[test]
    /// 驗證一般列表模式下，方向鍵會走和 `hjkl` 相同的移動與進出目錄邏輯。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_normal_mode_arrow_keys_map_to_vim_movement() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let initial_cwd = app.panes.get(&1).expect("pane").cwd.clone();

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .expect("move down");
        let selected_after_down = app
            .panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .path
            .clone();
        assert_eq!(selected_after_down, beta);

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .expect("move up");
        let selected_after_up = app
            .panes
            .get(&1)
            .expect("pane")
            .selected_entry()
            .expect("selected")
            .path
            .clone();
        assert_eq!(selected_after_up, alpha);

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .expect("enter directory");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, alpha);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
            .expect("go parent");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, initial_cwd);
    }

    #[test]
    /// 驗證一般列表模式用 `l` / `Left` / `Right` 切換目錄後，zoxide 也會同步學習這些位置。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_normal_mode_directory_navigation_updates_zoxide() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        fs::create_dir(&alpha).expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .expect("enter directory");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, alpha);

        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE))
            .expect("go parent");
        assert_eq!(app.panes.get(&1).expect("pane").cwd, dir.path());

        let tracked = query_zoxide_directories().expect("query zoxide");
        assert!(
            tracked.iter().any(|path| path == &alpha),
            "expected zoxide to contain {}",
            alpha.display()
        );
        assert!(
            tracked.iter().any(|path| path == dir.path()),
            "expected zoxide to contain {}",
            dir.path().display()
        );
    }

    #[test]
    /// 驗證 command mode 按下 Tab 時，會直接採用目前最接近的命令提示。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_tab_autocompletes_closest_command_suggestion() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        for ch in "zo".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type zo");
        }

        let suggestions = command_suggestions(&app.command_buffer);
        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].command, "zoxide");
        assert_eq!(suggestions[0].shortcut, "Z");
        assert_eq!(app.command_suggestion_selected, 0);

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("autocomplete closest suggestion");

        assert_eq!(app.command_buffer, "zoxide");
        assert_eq!(app.command_suggestion_selected, 0);
    }

    #[test]
    /// 驗證 command mode 會接受不同終端送出的候選切換事件格式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_cycles_autocomplete_accepts_terminal_variants() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("type t");

        let suggestions = command_suggestions(&app.command_buffer);
        assert!(!suggestions.is_empty());

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("type n normally");
        assert_eq!(app.command_buffer, "tn");
        assert_eq!(app.command_suggestion_selected, 0);

        app.command_buffer = String::from("t");
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT))
            .expect("next suggestion with lowercase+shift");
        assert_eq!(
            app.command_suggestion_selected,
            1.min(suggestions.len() - 1)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::CONTROL))
            .expect("next suggestion with uppercase ctrl");
        assert_eq!(
            app.command_suggestion_selected,
            (2).min(suggestions.len().saturating_sub(1))
        );

        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .expect("next suggestion with down");
        assert_eq!(
            app.command_suggestion_selected,
            (3).min(suggestions.len().saturating_sub(1))
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE))
            .expect("previous suggestion with uppercase char");
        assert_eq!(
            app.command_suggestion_selected,
            (2).min(suggestions.len().saturating_sub(1))
        );

        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .expect("previous suggestion with up");
        assert_eq!(
            app.command_suggestion_selected,
            (1).min(suggestions.len().saturating_sub(1))
        );
    }

    #[test]
    /// 驗證 command mode 可先用提示切換快捷鍵選中候選，再按 Tab 套用該提示。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_tab_uses_currently_selected_suggestion() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .expect("type r");

        let suggestions = command_suggestions(&app.command_buffer);
        assert!(suggestions.len() >= 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT))
            .expect("move to next suggestion");
        let selected = app.command_suggestion_selected;

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("apply selected suggestion");

        assert_eq!(app.command_buffer, suggestions[selected].command);
        assert_eq!(app.command_suggestion_selected, selected);
    }

    #[test]
    /// 驗證 command mode 按下 Enter 會先補齊候選，命令完整時再執行。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_enter_autocompletes_then_executes() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .expect("type r");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("autocomplete rename");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "rename");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute rename");

        assert!(!app.command_mode);
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::Rename { .. })
        ));
    }

    #[test]
    /// 驗證 command mode 在使用者已輸入 `goto smb://...` 時，Enter 會直接執行而不覆蓋成預設模板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_enter_executes_goto_smb_with_arguments() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        for ch in "goto smb://192.0.2.10/tfm-test-share/docs".chars() {
            let modifiers = if ch.is_ascii_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), modifiers))
                .expect("type command");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute command with args");

        assert!(!app.command_mode);
        assert!(app.pending_launch.is_some());
        assert!(
            app.status
                .starts_with("已請求系統掛載 SMB：smb://192.0.2.10/tfm-test-share/docs")
        );
    }

    #[test]
    /// 驗證帶參數的指令提示只會補上 `goto ` 前綴，不會把範例參數塞進輸入框。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_command_mode_autocomplete_uses_goto_prefix_instead_of_example_arguments() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");
        for ch in "go".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type partial command");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("autocomplete goto");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "goto ");
    }

    #[test]
    /// 驗證 `only_current_pane` 會只保留目前焦點窗格。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_only_keeps_focused_pane() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.only_current_pane();

        assert_eq!(app.ordered_pane_ids().len(), 1);
        assert_eq!(
            app.layout,
            LayoutNode::Leaf {
                pane_id: app.focused_pane
            }
        );
    }

    #[test]
    /// 驗證啟動設定會正確套用到第一個 pane 的隱藏檔與排序偏好。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_new_applies_startup_pane_preferences() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".hidden"), "secret").expect("hidden");
        fs::write(dir.path().join("visible.txt"), "visible").expect("visible");

        let loaded = LoadedConfig {
            config: AppConfig {
                pane: crate::config::PaneConfig {
                    show_hidden: true,
                    default_sort: StartupSort::Size,
                    default_sort_reverse: true,
                },
                ..AppConfig::default()
            },
            source: None,
        };

        let app = App::new(dir.path().to_path_buf(), loaded).expect("app");
        let pane = app.panes.get(&1).expect("pane");

        assert!(pane.show_hidden);
        assert_eq!(pane.sort_mode, SortMode::Size { reverse: true });
        assert_eq!(pane.visible_indices.len(), 2);
    }

    #[test]
    /// 驗證新分割出來的 pane 會繼承原 pane 的顯示隱藏檔與排序方式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_split_inherits_pane_preferences() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".hidden"), "secret").expect("hidden");
        fs::write(dir.path().join("visible.txt"), "visible").expect("visible");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        {
            let pane = app.panes.get_mut(&1).expect("pane");
            pane.set_show_hidden(true);
            pane.set_sort_mode(SortMode::Modified { reverse: true });
        }

        app.split_current(SplitDirection::Vertical).expect("split");

        let pane = app.panes.get(&2).expect("new pane");
        assert!(pane.show_hidden);
        assert_eq!(pane.sort_mode, SortMode::Modified { reverse: true });
        assert_eq!(pane.visible_indices.len(), 2);
    }

    #[test]
    /// 驗證刪除確認流程在確認後會真正刪除選取項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_delete_confirmation_removes_selected_entry() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm delete");

        assert!(!file_path.exists());
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "trashed delete-me.txt");
    }

    #[test]
    /// 驗證刪除確認視窗再次按 `d` 會關閉視窗，而不會執行刪除或移入 trash。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_delete_confirmation_d_closes_without_deleting() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("keep-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("close delete confirmation");

        assert!(file_path.exists());
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "trash cancelled: keep-me.txt");
    }

    #[test]
    /// 驗證兩個開在同一目錄的 panel，其中一個刪除檔案後，另一個也會同步刷新列表。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_delete_refreshes_other_panels_in_same_directory() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("shared.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.focus_pane_by_id(1);

        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        assert!(!file_path.exists());
        for pane_id in [1, 2] {
            let pane = app.panes.get(&pane_id).expect("pane");
            assert!(
                pane.entries.iter().all(|entry| entry.path != file_path),
                "panel {pane_id} still shows deleted file"
            );
        }
    }

    #[test]
    /// 驗證移到 trash 的項目可以透過 restore 命令還原。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_restore_latest_from_trash_recovers_file() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("restore-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");
        assert!(!file_path.exists());

        app.restore_latest_from_trash().expect("restore");

        assert!(file_path.exists());
        assert_eq!(app.status, "restored restore-me.txt");
    }

    #[test]
    /// 驗證 trash 面板可以列出項目，並透過 Enter 還原目前選到的檔案。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_trash_panel_lists_and_restores_entry() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("panel-restore.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash panel");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { selected: 0, .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open restore confirm");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmTrashAction { .. })
        ));
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm restore from panel");

        assert!(file_path.exists());
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert_eq!(app.status, "restored panel-restore.txt");
    }

    #[test]
    /// 驗證 trash 面板可用 `d` 永久刪除目前選到的項目，且會先進確認視窗。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_trash_panel_can_delete_selected_entry_permanently() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("purge-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash panel");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("open delete confirm");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmTrashAction { .. })
        ));
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("delete selected trash entry");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert_eq!(app.trash_store.list_entries().expect("list").len(), 0);
        assert_eq!(app.status, "deleted permanently purge-me.txt");
    }

    #[test]
    /// 驗證 trash 面板在確認刪除時仍保留原本列表狀態，取消後會回到同一個 trash 面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_trash_panel_delete_confirm_cancel_returns_to_same_trash_panel() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("cancel-delete.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash panel");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("open delete confirm");

        let (selected, search, marked_ids, visual_anchor) =
            trash_panel_overlay_state_from_pending_action(&app.pending_action, 1)
                .expect("trash overlay state");
        assert_eq!(selected, 0);
        assert_eq!(search.buffer, "");
        assert!(marked_ids.is_empty());
        assert_eq!(visual_anchor, None);

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel delete confirm");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel {
                pane_id: 1,
                selected: 0,
                ..
            })
        ));
        assert_eq!(app.status, "delete cancelled: cancel-delete.txt");
    }

    #[test]
    /// 驗證 trash 面板可用 `D` 永久刪除目前篩選結果的全部項目，且會先確認。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_trash_panel_shift_d_deletes_filtered_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("zzzzzz-alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm alpha");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm beta");

        app.open_trash_panel().expect("open trash panel");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start trash search");
        for _ in 0..6 {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
                .expect("type unique filter");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock trash filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
            .expect("open delete all confirm");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmTrashAction { .. })
        ));
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm clear filtered trash");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        let remaining = app.trash_store.list_entries().expect("list remaining");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].display_name, "beta.txt");
        assert_eq!(app.status, "deleted permanently zzzzzz-alpha.txt");
    }

    #[test]
    /// 驗證 trash 面板可用 `V` 標記多個項目，並透過 `U` 一次還原。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_trash_panel_visual_mark_restore_multiple_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm first");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm second");

        app.open_trash_panel().expect("open trash");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("start visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .expect("extend visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("commit visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::SHIFT))
            .expect("open restore all confirm");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmTrashAction { .. })
        ));
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm restore marked items");

        assert!(alpha.exists());
        assert!(beta.exists());
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert_eq!(app.status, "restored 2 items");
    }

    #[test]
    /// 驗證 trash 面板在已有 `V` 標記時，按 `u` 也會一次還原全部標記項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_trash_panel_visual_mark_lower_u_restores_multiple_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("lower-u-alpha.txt");
        let beta = dir.path().join("lower-u-beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm first");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm second");

        app.open_trash_panel().expect("open trash");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("start visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .expect("extend visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("commit visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
            .expect("open restore confirm");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmTrashAction { .. })
        ));
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm restore marked items");

        assert!(alpha.exists());
        assert!(beta.exists());
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert_eq!(app.status, "restored 2 items");
    }

    #[test]
    /// 驗證 trash 面板在已有 `V` 標記時，按 `d` 也會一次刪除全部標記項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_trash_panel_visual_mark_lower_d_deletes_multiple_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("lower-d-alpha.txt");
        let beta = dir.path().join("lower-d-beta.txt");
        fs::write(&alpha, "alpha").expect("alpha");
        fs::write(&beta, "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm first");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm second");

        app.open_trash_panel().expect("open trash");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("start visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .expect("extend visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT))
            .expect("commit visual mark");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("open delete confirm");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmTrashAction { .. })
        ));
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm delete marked items");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert_eq!(app.trash_store.list_entries().expect("list").len(), 0);
        assert_eq!(app.status, "deleted permanently 2 items");
    }

    #[test]
    /// 驗證從 trash 面板按 F1 打開 help 後，按 Esc 會回到原本的 trash 列表。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_from_trash_returns_to_trash_on_escape() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("from-trash-help.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");

        app.open_trash_panel().expect("open trash");
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help from trash");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
    }

    #[test]
    /// 驗證從 trash 打開 help 並執行 `:trash undo` 後，會回到最近的 trash 列表上下文。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_enter_from_trash_executes_undo_and_returns_to_trash() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("undo-via-help.txt");
        fs::write(&file_path, "hello").expect("file");
        let undo_index = help_entries("")
            .iter()
            .position(|entry| entry.line.command == ":trash undo")
            .expect("trash undo help entry");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_delete_confirmation(false);
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm trash");
        assert!(!file_path.exists());

        app.open_trash_panel().expect("open trash");
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help from trash");
        for _ in 0..undo_index {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
                .expect("move to trash undo help entry");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute trash undo from help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { .. })
        ));
        assert!(file_path.exists());
        assert_eq!(app.status, "restored undo-via-help.txt");
    }

    #[test]
    /// 驗證 `:tasks` 會打開目前 pane 的任務面板，且空清單時狀態訊息正確。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_tasks_command_opens_task_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.execute_command("tasks").expect("open tasks");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TaskPanel {
                pane_id: 1,
                selected: 0,
                ..
            })
        ));
        assert_eq!(app.status, "tasks: empty");
    }

    #[test]
    /// 驗證一般外部開啟會建立 task，並在主事件迴圈回報成功後標記完成。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_open_task_is_created_and_can_finish() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("queue open");

        assert_eq!(app.task_log.len(), 1);
        let task = app.task_log.last().expect("task");
        assert_eq!(task.kind, "open");
        assert_eq!(task.state, TaskState::Running);

        let queued = app.take_pending_launch().expect("queued launch");
        let task_id = queued.task_id;
        app.finish_launch_task(task_id, Ok(()));

        let task = app
            .task_log
            .iter()
            .find(|task| task.id == task_id)
            .expect("task");
        assert_eq!(task.state, TaskState::Done);
        assert_eq!(task.detail, "completed");
        assert!(task.finished_at_unix_ms.is_some());
    }

    #[test]
    /// 驗證 `z` 打開 fzf jump 時會建立 task，取消後也會正確標成 cancelled。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_fzf_jump_task_is_created_and_cancelled() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_fzf_jump();

        assert_eq!(app.task_log.len(), 1);
        let request = app.take_pending_fzf_jump().expect("fzf request");
        let task_id = request.task_id;
        let task = app
            .task_log
            .iter()
            .find(|task| task.id == task_id)
            .expect("task");
        assert_eq!(task.kind, "jump");
        assert_eq!(task.state, TaskState::Running);

        app.apply_fzf_jump_selection(request, None);

        let task = app
            .task_log
            .iter()
            .find(|task| task.id == task_id)
            .expect("task");
        assert_eq!(task.state, TaskState::Cancelled);
        assert_eq!(task.detail, "fzf cancelled");
    }

    #[test]
    /// 驗證在一般列表按下 Enter 會依預設外部開啟規則排入文字編輯器啟動。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_enter_queues_default_open_for_text_file() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("default open");

        let launch = app.take_pending_launch().expect("launch");
        let expected = if std::env::var("EDITOR")
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            LaunchMode::TerminalBlocking
        } else {
            LaunchMode::Detached
        };
        assert_eq!(launch.launch.mode, expected);
        assert_eq!(app.status, "opening notes.txt with editor");
    }

    #[test]
    /// 驗證按下 `O` 會打開 inline `Open with` 小視窗。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_o_opens_open_picker() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
            .expect("open picker");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::OpenPicker { .. })
        ));
    }

    #[test]
    /// 驗證按下 `Shift+Enter` 也會打開 inline `Open with` 小視窗。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_enter_opens_open_picker() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
            .expect("open picker");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::OpenPicker { .. })
        ));
    }

    #[test]
    /// 驗證 open picker 打開後，再按一次 `O` 會直接關閉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_o_toggles_open_picker_closed() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
            .expect("open picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::SHIFT))
            .expect("toggle close open picker");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 open picker 打開後，再按一次 `Shift+Enter` 也會直接關閉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_enter_toggles_open_picker_closed() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
            .expect("open picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT))
            .expect("toggle close open picker");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證自訂 open action 會出現在 Open with 面板中，並能排入外部啟動佇列。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_open_picker_includes_custom_actions() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut loaded = default_loaded_config();
        loaded
            .config
            .actions
            .open_with
            .push(CustomOpenActionConfig {
                name: "Git log".to_string(),
                scope: ActionTargetScope::Both,
                mode: ActionLaunchMode::TerminalBlocking,
                command: Some("git -C {parent} log --oneline".to_string()),
                mac_command: None,
                windows_command: Some("git -C {parent} log --oneline".to_string()),
            });

        let mut app = App::new(dir.path().to_path_buf(), loaded).expect("app");
        app.open_selected_with_picker().expect("open picker");

        match app.pending_action.as_mut() {
            Some(PendingAction::OpenPicker {
                options, selected, ..
            }) => {
                *selected = options
                    .iter()
                    .position(|option| option.label == "Git log")
                    .expect("custom option");
            }
            other => panic!("unexpected pending action: {other:?}"),
        }

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("queue custom action");

        let launch = app.take_pending_launch().expect("launch");
        assert_eq!(launch.launch.mode, LaunchMode::TerminalBlocking);
        assert!(launch.launch.args.join(" ").contains("git -C"));
        assert!(app.status.contains("running Git log on notes.txt"));
    }

    #[test]
    /// 驗證自訂 Open with 動作若與內建選項同名，會在原位置覆寫內建動作。
    ///
    /// 保護目的：`plugins.toml` 是使用者客製化層；使用者定義 `Vim` 或 `Reveal`
    /// 時必須採用外掛命令，不能保留內建動作，也不能在選單中顯示兩次。
    fn app_open_picker_custom_actions_override_builtin_names() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "hello").expect("file");

        let mut loaded = default_loaded_config();
        for name in ["Vim", " reveal ", "Git log", "git LOG"] {
            loaded
                .config
                .actions
                .open_with
                .push(CustomOpenActionConfig {
                    name: name.to_string(),
                    scope: ActionTargetScope::Both,
                    mode: ActionLaunchMode::TerminalBlocking,
                    command: Some("echo {path}".to_string()),
                    mac_command: None,
                    windows_command: None,
                });
        }

        let app = App::new(dir.path().to_path_buf(), loaded).expect("app");
        let target = app.selected_open_target().expect("selected target");
        let options = app.open_picker_options_for_target(&target);

        assert_eq!(
            options
                .iter()
                .filter(|option| option.label.eq_ignore_ascii_case("Vim"))
                .count(),
            1
        );
        let vim = options
            .iter()
            .find(|option| option.label.eq_ignore_ascii_case("Vim"))
            .expect("Vim option");
        assert!(matches!(vim.action, OpenPickerAction::Custom(_)));
        assert_eq!(
            options
                .iter()
                .filter(|option| option.label.trim().eq_ignore_ascii_case("Reveal"))
                .count(),
            1
        );
        let reveal = options
            .iter()
            .find(|option| option.label.trim().eq_ignore_ascii_case("Reveal"))
            .expect("Reveal option");
        assert!(matches!(reveal.action, OpenPickerAction::Custom(_)));
        assert_eq!(
            options
                .iter()
                .filter(|option| option.label.eq_ignore_ascii_case("Git log"))
                .count(),
            1
        );
        let git_log = options
            .iter()
            .find(|option| option.label.eq_ignore_ascii_case("git LOG"))
            .expect("Git log option");
        let OpenPickerAction::Custom(action) = &git_log.action else {
            panic!("Git log should be a custom action");
        };
        assert_eq!(action.name, "git LOG");
    }

    #[test]
    /// 驗證按下 `Tab` 會直接進入 preview mode。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_tab_opens_preview_mode() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("open preview with tab");

        assert!(app.panes.get(&1).expect("pane").is_preview_active());
        assert_eq!(app.status, "preview mode");
    }

    #[test]
    /// 驗證選到資料夾時，預設外部開啟會走系統開啟模式，而不是終端編輯器。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_open_directory_uses_detached_system_open() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open directory");

        let launch = app.take_pending_launch().expect("launch");
        assert_eq!(launch.launch.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證按下 `b` 會先打開書籤功能面板，再用 `a` 自動分配代號存書籤。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_picker_saves_with_auto_key() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        fs::create_dir(&docs).expect("docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .go_to_path(&docs)
            .expect("go docs");

        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("open bookmark picker");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::BookmarkPicker { pane_id: 1 })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("add bookmark");

        assert_eq!(app.status, format!("bookmark [a] = {}", docs.display()));
        assert!(
            fs::read_to_string(dir.path().join("bookmark.toml"))
                .expect("bookmark file")
                .contains("a =")
        );
    }

    #[test]
    /// 驗證按下 `w` 會打開 panel 操作面板，讓第二個按鍵可視化選擇。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_w_opens_window_picker() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("open window picker");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::WindowPicker { pane_id: 1 })
        );
        assert_eq!(app.status, "panel: choose h/j/k/l/c/o/t/d from the panel");
    }

    #[test]
    /// 驗證 `wt` 只使用目前 active panel 的 cwd 建立新終端請求。
    ///
    /// 保護目的：多 panel 時不可誤用第一個 panel 或 PaneFM 啟動目錄，否則終端會開錯位置。
    fn app_wt_opens_terminal_in_active_panel_directory() {
        let dir = tempdir().expect("tempdir");
        let first_dir = dir.path().join("first");
        let second_dir = dir.path().join("second");
        fs::create_dir(&first_dir).expect("first dir");
        fs::create_dir(&second_dir).expect("second dir");
        let mut app = App::new(first_dir, default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical)
            .expect("second panel");
        app.current_pane_mut().expect("active panel").cwd = second_dir.clone();
        app.current_pane_mut()
            .expect("active panel")
            .reload()
            .expect("reload second");

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("window picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("terminal action");

        let queued = app.take_pending_launch().expect("terminal launch");
        let path_is_active = queued
            .launch
            .args
            .iter()
            .any(|arg| arg == &second_dir.display().to_string())
            || matches!(
                queued.launch.mode,
                LaunchMode::NewTerminal { ref current_dir } if current_dir == &second_dir
            );
        assert!(path_is_active);
        assert_eq!(
            app.status,
            format!("opening terminal: {}", second_dir.display())
        );
    }

    #[test]
    /// 驗證 `wc` 會關閉目前 panel。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_window_picker_wc_closes_current_panel() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "hello").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("open window picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .expect("close current panel");

        assert_eq!(app.panes.len(), 1);
        assert_eq!(app.focused_pane, 1);
        assert_eq!(app.status, "closed panel 2");
    }

    #[test]
    /// 驗證仍可用 `'{key}` 直接跳回既有書籤，保留快速單鍵 workflow。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_direct_jump_still_works() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        let src = dir.path().join("src");
        fs::create_dir(&docs).expect("docs");
        fs::create_dir(&src).expect("src");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\n", docs.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .go_to_path(&src)
            .expect("go src");

        app.handle_key(KeyEvent::new(KeyCode::Char('\''), KeyModifiers::NONE))
            .expect("start bookmark jump");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("jump bookmark");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
        assert_eq!(app.status, "jumped to bookmark [a]");
    }

    #[test]
    /// 驗證按下 `m` 後再按 `s`，會套用 linemode size，而不改變目前排序方式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_linemode_picker_applies_size_without_changing_sort_order() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "1234").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_sort_mode(SortMode::Modified { reverse: true });

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("open linemode");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::LineModePicker { pane_id: 1 })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("apply line mode size");

        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.line_mode, Some(LineMode::Size));
        assert_eq!(pane.sort_mode, SortMode::Modified { reverse: true });
        assert_eq!(app.status, "linemode: size");
    }

    #[test]
    /// 驗證 linemode 面板收到非保留鍵時，不會誤存書籤，而是維持原本面板等待合法指令。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_linemode_picker_ignores_unknown_keys() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("open linemode");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("ignore unknown key");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::LineModePicker { pane_id: 1 })
        );
        assert_eq!(app.status, "linemode: choose a key from the panel");
    }

    #[test]
    /// 驗證 linemode 面板打開後，再按一次 `m` 會直接關閉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_linemode_picker_m_toggles_closed() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("open linemode");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("toggle close linemode");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 linemode 面板的 mtime 已改成 `t`，避免和 opener `m` 衝突。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_linemode_picker_t_applies_mtime() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("open linemode");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("apply mtime linemode");

        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.line_mode, Some(LineMode::Mtime));
        assert_eq!(app.status, "linemode: mtime");
    }

    #[test]
    /// 驗證 `bookmark.toml` 中既有的書籤可以在啟動後直接用命令跳轉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_jump_command_uses_bookmark_file() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        fs::create_dir(&docs).expect("docs");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("d = \"{}\"\n", docs.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("bookmark jump d")
            .expect("jump command");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, docs);
        assert_eq!(app.status, "jumped to bookmark [d]");
    }

    #[test]
    /// 驗證經由 `goto smb://...` 進入中文 SMB 目錄後，書籤檔仍保存 encoded URI，狀態列則顯示可讀中文。
    /// 保護目的：避免改善 Bookmark UI 時把解碼後文字寫回檔案，導致重新啟動後無法可靠跳轉 SMB。
    fn app_bookmark_set_persists_smb_location_after_goto() {
        let dir = tempdir().expect("tempdir");
        let mount_root = dir.path().join("mounts");
        let share_docs = mount_root.join("shared").join("網路事業部").join("otto");
        fs::create_dir_all(&share_docs).expect("share docs");
        let encoded = "smb://192.0.2.10/shared/%E7%B6%B2%E8%B7%AF%E4%BA%8B%E6%A5%AD%E9%83%A8/otto";

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.goto_smb_location_with_mount_root(encoded, &mount_root)
            .expect("goto smb");
        app.set_bookmark('s').expect("set bookmark");

        let bookmark_file =
            fs::read_to_string(dir.path().join("bookmark.toml")).expect("bookmark file");
        assert!(bookmark_file.contains(encoded));
        assert_eq!(
            app.status,
            "bookmark [s] = smb://192.0.2.10/shared/網路事業部/otto"
        );
    }

    #[test]
    /// 驗證 Bookmark 彈窗與其模糊 filter 都使用解碼後的中文 SMB 路徑。
    /// 保護目的：確保使用者看得到並能以中文搜尋書籤，同時列表背後仍保留可供跳轉的原始 target。
    fn bookmark_list_displays_and_filters_decoded_smb_path() {
        let encoded = "smb://192.0.2.10/shared/%E7%B6%B2%E8%B7%AF%E4%BA%8B%E6%A5%AD%E9%83%A8/otto";
        let entries = vec![BookmarkEntry {
            key: 's',
            target: BookmarkTarget::SmbLocation(encoded.to_string()),
        }];

        let lines = bookmark_panel_lines(entries.clone());
        assert_eq!(lines[0].path, "smb://192.0.2.10/shared/網路事業部/otto");
        let filtered = filtered_bookmark_entries(entries, "網事");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].target.as_storage_value(), encoded);
    }

    #[test]
    /// 驗證 SMB 書籤在跳轉時會自動走 SMB 掛載／進入流程，成功後直接切到目標目錄。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_jump_to_smb_bookmark_enters_target() {
        let dir = tempdir().expect("tempdir");
        let mount_root = dir.path().join("mounts");
        let share_docs = mount_root.join("shared").join("docs");
        fs::create_dir_all(&share_docs).expect("share docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.jump_to_bookmark_target_with_mount_root(
            1,
            's',
            &BookmarkTarget::SmbLocation(String::from("smb://192.0.2.10/shared/docs")),
            &mount_root,
        )
        .expect("jump smb bookmark");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, share_docs);
        assert_eq!(app.status, "jumped to bookmark [s]");
    }

    #[test]
    /// 驗證等待 linemode 按鍵時打開 F1，離開 help 後仍能回到原本的 linemode 面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_restores_pending_linemode_picker() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE))
            .expect("open linemode");
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close help");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::LineModePicker { pane_id: 1 })
        );
        assert_eq!(app.status, "linemode: choose a key from the panel");
    }

    #[test]
    /// 驗證 `:bookmark list` 會打開彈窗，並可用 Enter 跳到選中的書籤。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_list_popup_opens_and_jumps() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\nb = \"{}\"\n", alpha.display(), beta.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("bookmark list").expect("open list");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::BookmarkList {
                pane_id: 1,
                selected: 0,
                mode: BookmarkListMode::Jump,
                ..
            })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open bookmark");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, beta);
        assert_eq!(app.status, "jumped to bookmark [b]");
    }

    #[test]
    /// 驗證書籤列表會綁在開啟它的 pane 上，從第二個 pane 打開時也只影響第二個 pane。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_list_is_scoped_to_focused_pane() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\nb = \"{}\"\n", alpha.display(), beta.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);

        app.execute_command("bookmark list").expect("open list");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::BookmarkList {
                pane_id: 2,
                selected: 0,
                mode: BookmarkListMode::Jump,
                ..
            })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open bookmark");

        assert_eq!(app.panes.get(&2).expect("pane").cwd, beta);
        assert_ne!(app.panes.get(&1).expect("pane").cwd, beta);
    }

    #[test]
    /// 驗證按下 `b` 再按 `d` 會進入刪除列表，並可按對應書籤鍵直接刪除。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_delete_mode_removes_entry_by_matching_key() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\nb = \"{}\"\n", alpha.display(), beta.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("open bookmark picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .expect("open delete list");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::BookmarkList {
                pane_id: 1,
                selected: 0,
                mode: BookmarkListMode::Delete,
                ..
            })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("delete bookmark by key");

        assert_eq!(app.status, "bookmark [b] deleted");
        let content = fs::read_to_string(dir.path().join("bookmark.toml")).expect("bookmark file");
        assert!(content.contains("a = "));
        assert!(!content.contains("b = "));
    }

    #[test]
    /// 驗證書籤刪除列表可用游標移動後按 Enter 刪除選中的書籤。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_delete_mode_removes_selected_entry_with_enter() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\nb = \"{}\"\n", alpha.display(), beta.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("bookmark delete")
            .expect("open delete list");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("delete selected bookmark");

        assert_eq!(app.status, "bookmark [b] deleted");
        let content = fs::read_to_string(dir.path().join("bookmark.toml")).expect("bookmark file");
        assert!(content.contains("a = "));
        assert!(!content.contains("b = "));
    }

    #[test]
    /// 驗證按下 `b` 再按 `D` 會直接清空全部書籤。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_picker_can_delete_all_bookmarks() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        fs::create_dir(&alpha).expect("alpha");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\n", alpha.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("open bookmark picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
            .expect("delete all bookmarks");

        assert_eq!(app.status, "all bookmarks deleted");
        assert!(
            app.bookmark_store.list().is_empty(),
            "bookmark store should be empty after clear"
        );
    }

    #[test]
    /// 驗證書籤功能面板打開後，再按一次 `b` 會直接關閉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_picker_b_toggles_closed() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("open bookmark picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("toggle close bookmark picker");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證按下 `Z` 會直接打開 zoxide 目錄列表。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_z_opens_zoxide_list() {
        let dir = tempdir().expect("tempdir");
        let docs = dir.path().join("docs");
        fs::create_dir(&docs).expect("docs");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.go_to_path_and_track(1, &docs).expect("go docs");

        app.handle_key(KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::SHIFT))
            .expect("open zoxide list");

        assert!(matches!(
            &app.pending_action,
            Some(PendingAction::ZoxideList {
                pane_id: 1,
                selected: 0,
                entries,
                ..
            }) if !entries.is_empty()
        ));
    }

    #[test]
    /// 驗證 task 面板支援 `f` 搜尋，且來源／目的地也能用來篩選任務。
    /// 保護目的：檔案操作完成後，使用者常只記得 share 或目錄名稱；若搜尋只比對標題，
    /// 新增的診斷位置雖然看得到，卻無法在長期歷史中快速找回。
    fn app_task_panel_supports_filtering() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.task_log.push(TaskRecord {
            id: 1,
            pane_id: 1,
            kind: String::from("search"),
            title: String::from("alpha task"),
            detail: String::from("first detail"),
            source_locations: Vec::new(),
            destination_location: None,
            state: TaskState::Done,
            progress_percent: None,
            completed_bytes: None,
            total_bytes: None,
            started_at_unix_ms: 0,
            finished_at_unix_ms: Some(1),
        });
        app.task_log.push(TaskRecord {
            id: 2,
            pane_id: 1,
            kind: String::from("search"),
            title: String::from("beta task"),
            detail: String::from("second detail"),
            source_locations: vec![String::from("/source/report.txt")],
            destination_location: Some(String::from("/share/beta-destination")),
            state: TaskState::Running,
            progress_percent: None,
            completed_bytes: None,
            total_bytes: None,
            started_at_unix_ms: 2,
            finished_at_unix_ms: None,
        });

        app.open_task_panel();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start task filter");
        for ch in "destination".chars() {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type task query");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock task filter");

        match app.pending_action.as_ref() {
            Some(PendingAction::TaskPanel {
                selected, search, ..
            }) => {
                assert_eq!(*selected, 0);
                assert_eq!(search.buffer, "destination");
                assert!(!search.editing);
            }
            other => panic!("unexpected pending action: {other:?}"),
        }
        assert_eq!(
            app.status,
            "tasks: 1/1 (j/k move, x cancel, X cancel all, f search, h close)"
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("show filtered task detail");
        assert_eq!(app.status, "task 2 [search] second detail");
    }

    #[test]
    /// 驗證書籤列表支援 `f` 搜尋，並可直接打開過濾後唯一保留的書籤。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_bookmark_list_supports_filtering() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");
        fs::write(
            dir.path().join("bookmark.toml"),
            format!("a = \"{}\"\nb = \"{}\"\n", alpha.display(), beta.display()),
        )
        .expect("bookmark file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_bookmark_list();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start bookmark filter");
        for ch in ['b', 'e', 't', 'a'] {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type bookmark query");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock bookmark filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open filtered bookmark");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, beta);
        assert_eq!(app.status, "jumped to bookmark [b]");
    }

    #[test]
    /// 驗證 zoxide 列表支援 `f` 搜尋，並可跳到過濾後唯一保留的目錄。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_zoxide_list_supports_filtering() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha");
        let beta = dir.path().join("beta");
        fs::create_dir(&alpha).expect("alpha");
        fs::create_dir(&beta).expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.go_to_path_and_track(1, &alpha).expect("go alpha");
        app.go_to_path_and_track(1, &beta).expect("go beta");
        app.open_zoxide_list();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start zoxide filter");
        for ch in ['b', 'e', 't', 'a'] {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type zoxide query");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock zoxide filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open filtered zoxide path");

        assert_eq!(app.panes.get(&1).expect("pane").cwd, beta);
        assert_eq!(app.status, format!("jumped via zoxide: {}", beta.display()));
    }

    #[test]
    /// 驗證 `Shift+;` 也能正確打開命令模式，避免不同終端的事件格式造成 `:` 失效。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_semicolon_opens_command_mode() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(';'), KeyModifiers::SHIFT))
            .expect("open command mode");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "");
        assert_eq!(app.status, "command mode");
    }

    #[test]
    /// 驗證 `:panel <id>` 會把焦點直接切到指定 panel。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_panel_command_focuses_target_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);
        app.focus_pane_by_id(1);
        assert_eq!(app.focused_pane, 1);

        app.execute_command("panel 2").expect("focus panel 2");

        assert_eq!(app.focused_pane, 2);
        assert_eq!(app.status, "focused panel 2");
    }

    #[test]
    /// 驗證 `:status` 會在目前 focus panel 顯示外部工具狀態，且 Enter 可關閉查詢面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_status_command_opens_dependency_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.execute_command("status").expect("open status panel");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ToolPanel {
                pane_id: 1,
                selected: 0
            })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("close status panel");
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "dependency panel closed");
    }

    #[test]
    /// 驗證 `Ctrl+數字` 可直接切換焦點 panel。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_digit_focuses_target_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);
        app.focus_pane_by_id(1);
        assert_eq!(app.focused_pane, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::CONTROL))
            .expect("focus panel 2");

        assert_eq!(app.focused_pane, 2);
        assert_eq!(app.status, "focused panel 2");
    }

    #[test]
    /// 驗證多 panel 時直接按數字鍵，也能快速把焦點切到指定 panel。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_plain_digit_focuses_target_panel_when_multiple_panels_exist() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);
        app.focus_pane_by_id(1);
        assert_eq!(app.focused_pane, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
            .expect("focus panel 2");

        assert_eq!(app.focused_pane, 2);
        assert_eq!(app.status, "focused panel 2");
    }

    #[test]
    /// 驗證 `Ctrl+0` 會對應到 panel 10，讓雙位數前的最後一個快捷鍵也可直接使用。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_zero_focuses_tenth_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        for _ in 0..9 {
            app.split_current(SplitDirection::Vertical).expect("split");
        }
        assert_eq!(app.focused_pane, 10);
        app.focus_pane_by_id(1);

        app.handle_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::CONTROL))
            .expect("focus panel 10");

        assert_eq!(app.focused_pane, 10);
        assert_eq!(app.status, "focused panel 10");
    }

    #[test]
    /// 驗證 help 面板中需要參數的命令，按 Enter 後會打開預填命令，而不是直接執行空參數。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_argument_command_opens_prefilled_command_mode() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_help_panel();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start help search");
        for ch in ['C', 't', 'r', 'l', '-', 'p'] {
            let modifiers = if ch.is_ascii_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(ch), modifiers))
                .expect("type help query");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock help search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open panel command");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "panel ");
        assert_eq!(app.status, "command mode");
    }

    #[test]
    /// 驗證某些終端直接回報 `:` 而不帶 Shift modifier 時，也能正確打開命令模式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_plain_colon_opens_command_mode() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE))
            .expect("open command mode");

        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "");
        assert_eq!(app.status, "command mode");
    }

    #[test]
    /// 驗證 F1 說明面板可以打開，並在面板內用 `f` 進行搜尋。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_supports_filtering() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel { .. })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start help search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock help search");

        match app.pending_action.as_ref() {
            Some(PendingAction::HelpPanel { search, .. }) => {
                assert_eq!(search.buffer, "res");
                assert!(!search.editing);
            }
            other => panic!("unexpected pending action: {other:?}"),
        }
        let matches = help_entries("res").len();
        assert!(
            matches > 1,
            "fuzzy filter should find non-contiguous matches"
        );
        assert_eq!(app.status, format!("help: res ({matches})"));
    }

    #[test]
    /// 驗證 help 面板搜尋輸入中的 `Tab` 不會誤套用 command hint。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_search_tab_does_not_apply_command_autocomplete() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE))
            .expect("open help");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("start help search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("tab in help search");

        match app.pending_action.as_ref() {
            Some(PendingAction::HelpPanel {
                search, selected, ..
            }) => {
                assert_eq!(search.buffer, "re");
                assert!(search.editing);
                assert_eq!(*selected, 0);
            }
            other => panic!("unexpected pending action: {other:?}"),
        }
        assert_eq!(
            app.status,
            format!("help search: re ({})", help_entries("re").len())
        );
    }


    #[test]
    /// 驗證 help 面板已開啟時，再按一次 `~` 會直接關閉回 normal mode。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_tilde_toggles_help_panel_closed() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE))
            .expect("open help with tilde");
        app.handle_key(KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE))
            .expect("close help with tilde");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證某些終端把 `~` 回報成 `Shift+\`` 時，也能正確打開 help 面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_backtick_opens_help_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('`'), KeyModifiers::SHIFT))
            .expect("open help with shift backtick");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel { .. })
        ));
    }

    #[test]
    /// 驗證按下 `t` 會先打開 `t` 系列快捷鍵面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_t_opens_theme_command_picker() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("open theme command picker with t");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ThemeCommandPicker { pane_id: 1 })
        ));
        assert_eq!(app.status, "theme/trash: choose l/n/t/u from the panel");
    }

    #[test]
    /// 驗證 `tt` 會直接進入 Trash 列表，不再多開一層選單。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_tt_opens_trash_panel_directly() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("open t picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("open trash panel");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TrashPanel { pane_id: 1, .. })
        ));
    }

    #[test]
    /// 驗證 `tl` 會從 `t` 系列面板打開標題為 Theme List 的主題列表。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_tl_opens_theme_list() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("open t picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("open theme list");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ThemePicker { selected: 3, .. })
        ));
    }

    #[test]
    /// 驗證 `tn` 會從 `t` 系列面板切換下一個主題並保存設定。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_tn_cycles_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("open t picker");
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("cycle theme");

        assert_eq!(app.theme_preset, ThemePreset::CatppuccinLatte);
        assert!(
            std::fs::read_to_string(dir.path().join("config.toml"))
                .expect("read config")
                .contains("theme = \"catppuccin-latte\"")
        );
    }

    #[test]
    /// 驗證按下 `T` 會直接打開目前 pane 的 task 面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_t_opens_task_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::SHIFT))
            .expect("open tasks with T");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::TaskPanel {
                pane_id: 1,
                selected: 0,
                ..
            })
        ));
        assert_eq!(app.status, "tasks: empty");
    }

    #[test]
    /// 驗證 help 面板按下 Enter 後，會直接切到對應的互動模式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_enter_executes_selected_action() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_help_panel();

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute rename from help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::Rename { .. })
        ));
    }

    #[test]
    /// 驗證 help 面板在列表模式下按 `h` 會和 `Esc` 一樣關閉，保持與 `l` 的左右對稱操作。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_h_closes_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_help_panel();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("close help with h");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 normal mode 的 `J / K` 會用固定大步長快速移動列表游標。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_j_and_k_move_by_large_step() {
        let dir = tempdir().expect("tempdir");
        for index in 0..12 {
            fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
        }

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
            .expect("fast down");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
        assert_eq!(app.status, "fast down: 5");

        app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
            .expect("fast up");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
        assert_eq!(app.status, "fast up: 5");
    }

    #[test]
    /// 驗證 preview mode 的 `J / K` 會用固定大步長快速捲動內容。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_shift_j_and_k_scroll_by_large_step() {
        let dir = tempdir().expect("tempdir");
        let content = (0..20)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.path().join("notes.txt"), content).expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(4);
        app.open_preview_focus();

        app.handle_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
            .expect("preview fast down");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 5);

        app.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
            .expect("preview fast up");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
    }

    #[test]
    /// 驗證 help 面板支援 `J / K` 與 `Ctrl-d / Ctrl-u`，讓大步長與分頁移動可在暫時列表中共用。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_supports_fast_and_page_navigation() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_help_panel();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('J'), KeyModifiers::NONE))
            .expect("help fast down");
        match app.pending_action {
            Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 5),
            ref other => panic!("unexpected pending action: {other:?}"),
        }

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("help page down");
        match app.pending_action {
            Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 15),
            ref other => panic!("unexpected pending action: {other:?}"),
        }

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .expect("help page up");
        match app.pending_action {
            Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 5),
            ref other => panic!("unexpected pending action: {other:?}"),
        }

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::NONE))
            .expect("help fast up");
        match app.pending_action {
            Some(PendingAction::HelpPanel { selected, .. }) => assert_eq!(selected, 0),
            ref other => panic!("unexpected pending action: {other:?}"),
        }
    }

    #[test]
    /// 驗證 help 面板中的 `:delete` 會保留 `d` 快捷鍵，並透過 Enter 進入刪除確認。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_help_panel_delete_entry_matches_delete_behavior() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-from-help.txt");
        fs::write(&file_path, "hello").expect("file");

        let entries = help_entries("");
        let delete_entry = entries
            .iter()
            .find(|entry| entry.line.command == ":delete")
            .expect("delete help entry");
        let trash_entry = entries
            .iter()
            .find(|entry| entry.line.command == ":trash")
            .expect("trash help entry");
        let delete_index = entries
            .iter()
            .position(|entry| entry.line.command == ":delete")
            .expect("delete help index");
        assert_eq!(delete_entry.line.shortcut, "d");
        assert_eq!(trash_entry.line.shortcut, "tt");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_help_panel();

        for _ in 0..delete_index {
            app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
                .expect("move to delete help entry");
        }
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("execute delete from help");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { .. })
        ));
    }

    #[test]
    /// 驗證輪替主題時會切換到下一個預設值。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_cycle_theme_switches_to_next_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.cycle_theme();

        assert_eq!(app.theme_preset, ThemePreset::CatppuccinLatte);
        assert_eq!(app.theme, ThemePreset::CatppuccinLatte.into());
        assert_eq!(app.status, "theme: catppuccin-latte");
    }

    #[test]
    /// 驗證打開主題選擇視窗時，游標會落在目前主題。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_open_theme_picker_tracks_current_preset() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_theme_picker();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker {
                selected: 3,
                original: ThemePreset::CatppuccinMocha,
            })
        );
    }

    #[test]
    /// 驗證依主題名稱字串指定主題時會正確更新狀態。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_set_theme_by_name_updates_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.set_theme_by_name("ocean");

        assert_eq!(app.theme_preset, ThemePreset::Nord);
        assert_eq!(app.theme, ThemePreset::Nord.into());
        assert_eq!(app.status, "theme: nord");
    }

    #[test]
    /// 驗證在主題選擇視窗按下 Enter 後會套用目前選取的主題。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_theme_picker_confirm_applies_selected_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::ThemePicker {
            selected: 2,
            original: ThemePreset::CatppuccinMocha,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("apply theme");

        assert_eq!(app.theme_preset, ThemePreset::Nord);
        assert_eq!(app.theme, ThemePreset::Nord.into());
        assert_eq!(app.status, "theme: nord");
    }

    #[test]
    /// 驗證主題選擇視窗也遵守核心 `h/l` 規則：`l` 套用、`h` 關閉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_theme_picker_supports_h_and_l_core_navigation() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::ThemePicker {
            selected: 2,
            original: ThemePreset::CatppuccinMocha,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("close theme picker");
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "theme picker cancelled");

        app.pending_action = Some(PendingAction::ThemePicker {
            selected: 2,
            original: ThemePreset::CatppuccinMocha,
        });
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("apply theme with l");
        assert_eq!(app.theme_preset, ThemePreset::Nord);
        assert_eq!(app.status, "theme: nord");
    }

    #[test]
    /// 驗證主題選擇視窗支援 `j/k` 上下移動，且索引會停在有效範圍內。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_theme_picker_supports_j_and_k_navigation() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::ThemePicker {
            selected: 3,
            original: ThemePreset::CatppuccinMocha,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker {
                selected: 4,
                original: ThemePreset::CatppuccinMocha,
            })
        );
        assert_eq!(app.theme, ThemePreset::CatppuccinLatte.into());
        assert_eq!(app.theme_preset, ThemePreset::CatppuccinMocha);

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("move up");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker {
                selected: 3,
                original: ThemePreset::CatppuccinMocha,
            })
        );
        assert_eq!(app.theme, ThemePreset::CatppuccinMocha.into());
    }

    #[test]
    /// 驗證主題選擇視窗支援 `Ctrl-d/u` 半頁移動，方便快速瀏覽完整主題清單。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_theme_picker_supports_ctrl_page_navigation() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::ThemePicker {
            selected: 0,
            original: ThemePreset::CatppuccinMocha,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("move one page down");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker {
                selected: 10,
                original: ThemePreset::CatppuccinMocha,
            })
        );
        assert_eq!(app.theme, ThemePreset::MonokaiPro.into());

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .expect("move one page up");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::ThemePicker {
                selected: 0,
                original: ThemePreset::CatppuccinMocha,
            })
        );
        assert_eq!(app.theme, ThemePreset::Dracula.into());
    }

    #[test]
    /// 驗證即時預覽後按下 Esc 會還原開啟列表前的主題，且不會修改已保存的主題。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_theme_picker_cancel_restores_original_theme() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let original = app.theme_preset;

        app.open_theme_picker();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("preview next theme");
        assert_ne!(app.theme, Theme::from(original));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel theme preview");

        assert!(app.pending_action.is_none());
        assert_eq!(app.theme, Theme::from(original));
        assert_eq!(app.theme_preset, original);
        assert_eq!(app.config.ui.theme_preset, original);
    }

    #[test]
    /// 驗證排序面板可用 `h` 關閉，避免和整體核心操作規則不一致。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_sort_picker_h_closes_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_sort_picker();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("close sort picker");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "sort cancelled");
    }

    #[test]
    /// 驗證排序面板打開後，再按一次 `,` 會直接關閉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_sort_picker_comma_toggles_closed() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_sort_picker();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
            .expect("toggle close sort picker");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "sort cancelled");
    }

    #[test]
    /// 驗證打開重新命名視窗時，會帶入目前選取項目的原名稱與預設輸入值。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_start_rename_opens_dialog_with_selected_name() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_rename();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("alpha.txt"),
                buffer: String::from("alpha.txt"),
                cursor: 5,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證在重新命名視窗按下 Enter 後會套用新的檔名。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_rename_confirm_updates_selected_entry() {
        let dir = tempdir().expect("tempdir");
        let old_path = dir.path().join("alpha.txt");
        let new_path = dir.path().join("beta.txt");
        fs::write(&old_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::from("beta.txt"),
            cursor: 4,
            mode: RenameMode::Insert,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("rename");

        assert!(!old_path.exists());
        assert!(new_path.exists());
        assert_eq!(app.status, "renamed alpha.txt -> beta.txt");
    }

    #[test]
    /// 驗證 `:rename-regex` 會打開預覽面板，並正確標示 ready / unchanged。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_rename_regex_command_opens_preview_panel() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.md"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("mark second");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");

        app.execute_command("rename-regex '^(.*)\\.txt$' '$1.md'")
            .expect("open regex rename");

        match app.pending_action.as_ref() {
            Some(PendingAction::RegexRename { previews, .. }) => {
                assert_eq!(previews.len(), 2);
                assert_eq!(previews[0].new_name, "alpha.md");
                assert_eq!(previews[0].outcome, RegexRenameOutcome::Ready);
                assert_eq!(previews[1].new_name, "beta.md");
                assert_eq!(previews[1].outcome, RegexRenameOutcome::Unchanged);
            }
            other => panic!("unexpected pending action: {other:?}"),
        }
    }

    #[test]
    /// 驗證 regex 批次改名在按下 Enter 後會一次套用所有 ready 項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_rename_regex_preview_applies_ready_entries() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "a").expect("alpha");
        fs::write(&beta, "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("mark second");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.execute_command("rename-regex '^(.*)\\.txt$' 'file_$1.md'")
            .expect("open regex rename");

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("apply regex rename");

        assert!(!alpha.exists());
        assert!(!beta.exists());
        assert!(dir.path().join("file_alpha.md").exists());
        assert!(dir.path().join("file_beta.md").exists());
        assert_eq!(app.status, "rename-regex: renamed 2 items");
    }

    #[test]
    /// 驗證從命令輸入介面送出 `reg` 後，預覽面板再次按 Enter 會實際完成改名。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_regex_rename_command_ui_enter_applies_preview() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("alpha.txt");
        let target = dir.path().join("alpha.md");
        fs::write(&source, "a").expect("source");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT))
            .expect("open regex command");
        assert!(app.command_mode);
        app.command_buffer = String::from("reg '^(.*)\\.txt$' '$1.md'");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("submit regex command");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::RegexRename { .. })
        ));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("apply regex preview");

        assert!(!source.exists());
        assert!(target.exists());
        assert_eq!(app.status, "rename-regex: renamed 1 item");
    }

    #[test]
    /// 驗證 regex 批次改名若會撞名，會標示 conflict，且 Enter 不會直接套用。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_rename_regex_preview_blocks_conflicts() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "a").expect("alpha");
        fs::write(&beta, "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("mark second");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.execute_command("rename-regex '^(.*)\\.txt$' 'same.txt'")
            .expect("open regex rename");

        match app.pending_action.as_ref() {
            Some(PendingAction::RegexRename { previews, .. }) => {
                assert!(
                    previews
                        .iter()
                        .all(|preview| preview.outcome == RegexRenameOutcome::Conflict)
                );
            }
            other => panic!("unexpected pending action: {other:?}"),
        }

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("try apply conflicting rename");

        assert!(alpha.exists());
        assert!(beta.exists());
        assert_eq!(app.status, "rename-regex: resolve conflicts before apply");
    }

    #[test]
    /// 驗證 rename 預設游標會停在副檔名前，方便優先修改主檔名。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn rename_basename_cursor_stops_before_extension() {
        assert_eq!(rename_basename_cursor("alpha.txt"), 5);
        assert_eq!(rename_basename_cursor("archive.tar.gz"), 11);
        assert_eq!(rename_basename_cursor(".gitignore"), 10);
        assert_eq!(rename_basename_cursor("folder"), 6);
    }

    #[test]
    /// 驗證 rename 可以在 insert 與 normal 模式之間切換，並保留游標位置。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn rename_mode_switches_between_insert_and_normal() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_rename();

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("switch to normal");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("alpha.txt"),
                buffer: String::from("alpha.txt"),
                cursor: 5,
                mode: RenameMode::Normal,
            })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("move left");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
            .expect("back to insert");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("alpha.txt"),
                buffer: String::from("alpha.txt"),
                cursor: 4,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證 rename 的 Vim 單字移動會依照檔名分隔符正確跳轉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn rename_word_motion_helpers_follow_filename_segments() {
        let name = "my-long_file.txt";

        assert_eq!(rename_next_word_start(name, 0), 3);
        assert_eq!(rename_next_word_start(name, 3), 8);
        assert_eq!(rename_next_word_start(name, 8), 13);

        assert_eq!(rename_previous_word_start(name, 13), 8);
        assert_eq!(rename_previous_word_start(name, 8), 3);
        assert_eq!(rename_previous_word_start(name, 3), 0);

        assert_eq!(rename_word_end(name, 0), 1);
        assert_eq!(rename_word_end(name, 3), 6);
        assert_eq!(rename_word_end(name, 8), 11);
        assert_eq!(rename_word_end(name, 12), 15);
    }

    #[test]
    /// 驗證 rename 的 normal 模式支援 `w`、`b`、`e`、`a`、`A` 這些 Vim 風格操作。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn rename_normal_mode_supports_vim_word_motions_and_insert_shortcuts() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("my-long_file.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.start_rename();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("switch to normal");

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE))
            .expect("move to previous word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("move to next word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("move to word end");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("append after cursor");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("my-long_file.txt"),
                buffer: String::from("my-long_file.txt"),
                cursor: 16,
                mode: RenameMode::Insert,
            })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("back to normal");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE))
            .expect("jump to start");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("jump to next word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("jump to end of word");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("append inside basename");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("my-long_file.txt"),
                buffer: String::from("my-long_file.txt"),
                cursor: 7,
                mode: RenameMode::Insert,
            })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("back to normal again");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE))
            .expect("append at end");

        assert_eq!(
            app.pending_action,
            Some(PendingAction::Rename {
                pane_id: 1,
                original_name: String::from("my-long_file.txt"),
                buffer: String::from("my-long_file.txt"),
                cursor: 16,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證 `y` 複製後可以用 `p` 把檔案貼到另一個目錄，且來源會保留。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_copy_and_paste_preserves_source_file() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("alpha.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy");

        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.operation),
            Some(ClipboardOperation::Copy)
        );
        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.entries.len()),
            Some(1)
        );

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("paste");

        assert!(source_file.exists());
        assert!(target_dir.join("alpha.txt").exists());
        assert_eq!(app.status, "pasted copy: 1 item");
    }

    #[test]
    /// 驗證大型檔案貼上會先建立背景 task，而不是在按下 `p` 時同步完成。
    ///
    /// 保護目的：大 ZIP 或 SMB 傳輸可能需要數分鐘；測試以 sparse 8 MiB 檔觸發
    /// 背景門檻，確認主處理函式先返回、完成事件仍會刷新列表並建立 Undo 歷史。
    fn app_large_paste_runs_as_background_task_and_records_completion() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("large.zip");
        fs::File::create(&source_file)
            .expect("large source")
            .set_len(BACKGROUND_FILE_JOB_THRESHOLD_BYTES)
            .expect("size source");

        let mut app = App::new(source_dir, default_loaded_config()).expect("app");
        app.copy_selected();
        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload target");

        app.paste_into_focused_pane().expect("queue paste");

        assert!(!app.file_job_receivers.is_empty());
        assert!(app.status.contains("in background"));
        wait_for_file_jobs(&mut app);

        assert_eq!(
            fs::metadata(target_dir.join("large.zip"))
                .expect("target metadata")
                .len(),
            BACKGROUND_FILE_JOB_THRESHOLD_BYTES
        );
        assert_eq!(app.status, "pasted copy: 1 item");
        assert_eq!(app.operation_history.len(), 1);
        assert!(matches!(
            app.task_log.last().map(|task| task.state),
            Some(TaskState::Done)
        ));
        let task = app.task_log.last().expect("background paste task");
        assert_eq!(
            task.source_locations,
            vec![source_file.display().to_string()],
            "背景貼上必須永久保存實際來源，不可只留下完成訊息"
        );
        assert_eq!(
            task.destination_location,
            Some(target_dir.display().to_string()),
            "背景貼上完成後仍必須能辨識目的目錄"
        );
        assert_eq!(
            app.task_log.last().and_then(|task| task.completed_bytes),
            app.task_log.last().and_then(|task| task.total_bytes),
            "背景貼上完成後已處理 byte 必須等於總 byte，不能停在最後一次中途回報"
        );
    }

    #[test]
    /// 驗證 PaneFM 正常關閉時會把尚未完成的 task 標成 `Interrupted` 並保存到檔案。
    ///
    /// 保護目的：大型本機或 SMB copy 可能執行超過半小時；若使用者關閉程式，舊版
    /// 記憶體 task 會完全消失。此測試確保關閉後仍可追查開始時間、進度與中斷原因，
    /// 且不會錯誤顯示為仍在執行。
    fn app_shutdown_persists_running_tasks_as_interrupted() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let task_id = app.push_task(
            1,
            "paste",
            String::from("copy large.zip"),
            String::from("destination: share"),
            vec![String::from("/source/large.zip")],
            Some(String::from("/destination/share")),
        );
        app.update_task_progress(task_id, 42, 100);

        app.prepare_for_shutdown().expect("persist shutdown");
        let tasks = super::load_task_history(&app.task_history_path).expect("load history");
        let task = tasks.last().expect("persisted task");

        assert_eq!(task.state, TaskState::Interrupted);
        assert_eq!(task.progress_percent, Some(42));
        assert_eq!(task.completed_bytes, Some(42));
        assert_eq!(task.total_bytes, Some(100));
        assert!(task.finished_at_unix_ms.is_some());
        assert!(task.detail.contains("interrupted when PaneFM closed"));
    }

    #[test]
    /// 驗證啟動時會載入上次 task 歷史，並修正來不及正常關閉的 `Running` 紀錄。
    ///
    /// 保護目的：使用者可能直接關閉 terminal 或系統終止程序，導致關閉 hook 沒機會
    /// 執行。下次啟動必須把磁碟上最後一次 RUNNING 快照轉為 `Interrupted`，不能讓 task
    /// 面板永久顯示不存在的工作，也不能自動重複覆寫目的檔案。
    fn app_startup_recovers_unclean_running_task_history() {
        let dir = tempdir().expect("tempdir");
        let history_path = super::task_history_file_path(dir.path(), None);
        super::save_task_history(
            &history_path,
            &[TaskRecord {
                id: 12,
                pane_id: 8,
                kind: String::from("paste"),
                title: String::from("copy build"),
                detail: String::from("destination: share"),
                source_locations: vec![String::from("/source/build")],
                destination_location: Some(String::from("/destination/share")),
                state: TaskState::Running,
                progress_percent: Some(37),
                completed_bytes: Some(37),
                total_bytes: Some(100),
                started_at_unix_ms: 1_700_000_000_000,
                finished_at_unix_ms: None,
            }],
        )
        .expect("seed history");

        let app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let task = app.task_log.last().expect("recovered task");

        assert_eq!(task.state, TaskState::Interrupted);
        assert_eq!(
            task.pane_id, 1,
            "舊 session 的 task 必須出現在目前可見 panel"
        );
        assert_eq!(app.next_task_id, 13);
        assert!(app.status.contains("recovered 1 interrupted task"));
    }

    #[test]
    /// 驗證來源只要是目錄就必須直接判定為背景貼上，不能用目錄本身的 metadata 大小
    /// 代表內部內容。測試以 sparse file 表示超過 1 GiB 的大型 build 目錄，不實際寫入
    /// 或複製 1 GiB 資料。
    ///
    /// 保護目的：過去 `target/` 雖然包含大量資料，目錄 metadata 卻只有數百 bytes，
    /// 因而被錯放到 UI thread 同步複製，造成整個 TUI 卡死。
    fn directory_paste_is_background_even_when_directory_metadata_is_small() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("target");
        let destination_dir = dir.path().join("destination");
        fs::create_dir(&source_dir).expect("source directory");
        fs::create_dir(&destination_dir).expect("destination directory");
        fs::File::create(source_dir.join("large-build-output.bin"))
            .expect("sparse source")
            .set_len(1024 * 1024 * 1024 + 1)
            .expect("sparse source size");
        let clipboard = ClipboardState {
            entries: vec![ClipboardEntry {
                source_path: source_dir,
                display_name: String::from("target"),
            }],
            operation: ClipboardOperation::Copy,
        };

        assert!(paste_should_run_in_background(&clipboard, &destination_dir));
    }

    #[test]
    /// 驗證背景進度採用最新 byte 估算，且 task 面板不再只顯示百分比。
    ///
    /// 保護目的：目錄只走訪一次時總量會逐步增加，畫面必須能從早期估算校正成最新
    /// 比例，否則前幾個檔案完成後可能長時間錯誤停在 99%。
    fn task_progress_uses_latest_dynamic_estimate_and_is_visible_in_panel() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let task_id = app.push_task(
            1,
            "paste",
            String::from("copy large.zip"),
            String::from("destination: target"),
            vec![String::from("/source/large.zip")],
            Some(String::from("/destination/target")),
        );

        app.update_task_progress(task_id, 60, 100);
        app.update_task_progress(task_id, 20, 100);
        let lines = super::task_panel_lines(&app.task_log);

        assert_eq!(app.task_log[0].progress_percent, Some(20));
        assert_eq!(lines[0].state, "RUNNING");
        assert_eq!(app.task_log[0].completed_bytes, Some(20));
        assert_eq!(app.task_log[0].total_bytes, Some(100));
        assert_eq!(lines[0].progress, "20B / 100B");
        assert_eq!(lines[0].finished_at, "--:--:--");

        app.finish_task(task_id, TaskState::Done, String::from("completed"));
        assert_eq!(app.task_log[0].progress_percent, Some(100));
        assert_eq!(app.task_log[0].completed_bytes, Some(100));
        let lines = super::task_panel_lines(&app.task_log);
        assert_ne!(lines[0].finished_at, "--:--:--");
    }

    #[test]
    /// 驗證 task byte 會依數量級切換單位，且不再輸出百分比。
    ///
    /// 保護目的：大型本機與 SMB 傳輸即使百分比長時間不變，使用者仍要從 byte 數判斷
    /// 工作是否前進；格式重構不能把資訊退回只有 `31%`。
    fn task_progress_label_uses_compact_byte_units_without_percentage() {
        let mut task = TaskRecord {
            id: 1,
            pane_id: 1,
            kind: String::from("paste"),
            title: String::from("copy project"),
            detail: String::new(),
            source_locations: vec![String::from("/source/project")],
            destination_location: Some(String::from("/destination")),
            state: TaskState::Running,
            progress_percent: Some(31),
            completed_bytes: Some(25_589_858_714),
            total_bytes: Some(82_893_350_912),
            started_at_unix_ms: 0,
            finished_at_unix_ms: None,
        };

        assert_eq!(super::task_progress_label(&task), "23.8G / 77.2G");
        assert!(!super::task_progress_label(&task).contains('%'));
        task.completed_bytes = None;
        task.total_bytes = None;
        assert_eq!(super::task_progress_label(&task), "-");
    }

    #[test]
    /// 驗證 `ms` 會在背景遞迴計算每個直接子目錄的檔案總 byte。
    ///
    /// 保護目的：舊版只顯示直接子項目數量，而且同步讀取會拖慢 SMB；這個測試確保
    /// linemode 立即啟動 worker、主執行緒可持續 poll，最後得到真正內容大小。
    fn size_linemode_scans_recursive_directory_bytes_in_background() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        let sibling = dir.path().join("sibling");
        fs::create_dir_all(child.join("nested")).expect("nested");
        fs::create_dir(&sibling).expect("sibling");
        fs::write(child.join("one.bin"), vec![0u8; 7]).expect("first file");
        fs::write(child.join("nested/two.bin"), vec![0u8; 11]).expect("second file");
        fs::write(child.join(".hidden.bin"), vec![0u8; 5]).expect("hidden file");
        fs::write(child.join(".gitignore"), "ignored.bin\n").expect("ignore rules");
        fs::write(child.join("ignored.bin"), vec![0u8; 17]).expect("ignored file");
        fs::write(sibling.join("three.bin"), vec![0u8; 13]).expect("third file");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.apply_line_mode(1, LineMode::Size)
            .expect("enable size linemode");
        assert!(!app.directory_size_jobs.is_empty());
        assert!(
            app.panes[&1]
                .entries
                .iter()
                .filter(|entry| entry.is_dir)
                .all(|entry| entry.directory_size == Some(0) && !entry.directory_size_complete),
            "啟動背景掃描的當下就要顯示 ~0B，不可長時間停在省略號"
        );
        for _ in 0..200 {
            app.poll_background_tasks();
            if app.directory_size_jobs.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        let entry = app.panes[&1]
            .entries
            .iter()
            .find(|entry| entry.path == child)
            .expect("child entry");
        assert_eq!(entry.directory_size, Some(52));
        assert!(entry.directory_size_complete);
        let sibling_entry = app.panes[&1]
            .entries
            .iter()
            .find(|entry| entry.path == sibling)
            .expect("sibling entry");
        assert_eq!(sibling_entry.directory_size, Some(13));
        assert!(sibling_entry.directory_size_complete);
        assert!(app.directory_size_jobs.is_empty());
    }

    #[test]
    /// 驗證目錄清單分批載入期間啟用 `ms`，完整清單抵達後會以全部目錄重啟容量掃描。
    ///
    /// 保護目的：舊流程會保留只看過首批項目的同 cwd 掃描，導致稍後加入的目錄右側
    /// 永久空白。測試刻意讓舊掃描留在工作表中，再送入含新目錄的完成事件，確保舊
    /// worker 被取消、新目錄立即顯示部分值，且最後能取得正確容量。
    fn completed_directory_load_restarts_size_scan_for_late_entries() {
        let dir = tempdir().expect("tempdir");
        let early = dir.path().join("early");
        let late = dir.path().join("晚到的目錄");
        fs::create_dir(&early).expect("early directory");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.apply_line_mode(1, LineMode::Size)
            .expect("enable size linemode");
        let old_scan_cancelled = Arc::clone(&app.directory_size_jobs[&1].cancelled);

        fs::create_dir(&late).expect("late directory");
        fs::write(late.join("payload.bin"), vec![0u8; 31]).expect("late payload");
        let complete_entries = PaneState::new(dir.path().to_path_buf())
            .expect("reload complete entries")
            .entries;
        let (sender, receiver) = mpsc::channel();
        sender
            .send(DirectoryLoadEvent {
                pane_id: 1,
                cwd: dir.path().to_path_buf(),
                selected_path: None,
                result: Ok(super::DirectoryLoadProgress::Complete(complete_entries)),
            })
            .expect("complete directory event");
        app.directory_load_jobs.insert(
            1,
            DirectoryLoadJob {
                cwd: dir.path().to_path_buf(),
                receiver,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );

        app.poll_directory_load_jobs();

        assert!(old_scan_cancelled.load(Ordering::Relaxed));
        assert_eq!(app.directory_size_jobs[&1].cwd, dir.path());
        let late_entry = app.panes[&1]
            .entries
            .iter()
            .find(|entry| entry.path == late)
            .expect("late entry");
        assert_eq!(late_entry.directory_size, Some(0));
        assert!(!late_entry.directory_size_complete);

        for _ in 0..200 {
            app.poll_background_tasks();
            if app.directory_size_jobs.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let late_entry = app.panes[&1]
            .entries
            .iter()
            .find(|entry| entry.path == late)
            .expect("late entry after scan");
        assert_eq!(late_entry.directory_size, Some(31));
        assert!(late_entry.directory_size_complete);
    }

    #[test]
    /// 驗證 `ms` 啟用期間進入下一層目錄，會立即替新列表重新啟動容量掃描。
    ///
    /// 保護目的：舊流程只在首次切換 linemode 時排程；使用 `l` 進入已顯示容量的目錄後，
    /// 舊工作因 cwd 不符被取消，新目錄卻永久顯示 `...`。這個測試固定新工作必須綁定
    /// 子目錄、子目錄中的資料夾立即顯示部分值，最後取得正確 byte 數。
    fn size_linemode_restarts_scan_after_entering_directory() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        let nested = child.join("nested");
        fs::create_dir_all(&nested).expect("nested directory");
        fs::write(nested.join("payload.bin"), vec![0u8; 29]).expect("payload");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.apply_line_mode(1, LineMode::Size)
            .expect("enable size linemode");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("enter child directory");

        assert_eq!(app.panes[&1].cwd, child);
        assert!(app.directory_load_jobs.contains_key(&1));
        assert!(
            app.panes[&1].entries.is_empty(),
            "首次進入時應先交還事件迴圈，不能同步等待目錄 I/O"
        );
        for _ in 0..200 {
            app.poll_background_tasks();
            if app.directory_load_jobs.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        assert!(app.directory_load_jobs.is_empty());
        assert_eq!(app.directory_size_jobs[&1].cwd, child);
        let nested_entry = app.panes[&1]
            .entries
            .iter()
            .find(|entry| entry.path == nested)
            .expect("nested entry");
        assert_eq!(nested_entry.directory_size, Some(0));
        assert!(!nested_entry.directory_size_complete);

        for _ in 0..200 {
            app.poll_background_tasks();
            if app.directory_size_jobs.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        let nested_entry = app.panes[&1]
            .entries
            .iter()
            .find(|entry| entry.path == nested)
            .expect("nested entry after scan");
        assert_eq!(nested_entry.directory_size, Some(29));
        assert!(nested_entry.directory_size_complete);
        assert!(app.directory_size_jobs.is_empty());

        // 回到父目錄再立刻重進時，已完成的清單必須在按鍵處直接由快取恢復；背景
        // refresh 仍會校正磁碟最新狀態，但畫面不能再次短暫變成空白。
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("return to parent");
        assert!(
            app.panes[&1]
                .entries
                .iter()
                .any(|entry| entry.path == child)
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("re-enter cached child");
        assert!(
            app.panes[&1]
                .entries
                .iter()
                .any(|entry| entry.path == nested)
        );
    }

    #[test]
    /// 驗證 `ms` 背景容量在 200ms 邊界才傳回下一份部分結果。
    ///
    /// 保護目的：太慢會讓大目錄長時間看不到變化，太快則會對每個檔案發送事件並
    /// 影響鍵盤操作；測試固定 199ms 不更新、200ms 立即更新的規格。
    fn directory_size_partial_updates_use_two_hundred_millisecond_interval() {
        assert!(!super::should_report_directory_size(199, 0));
        assert!(super::should_report_directory_size(200, 0));
        assert!(!super::should_report_directory_size(399, 200));
        assert!(super::should_report_directory_size(400, 200));
    }

    #[test]
    /// 驗證離開 size linemode 會取消該 panel 的背景掃描。
    ///
    /// 保護目的：使用者可能快速切換模式或目錄；若舊 worker 不取消，會持續讀磁碟／SMB
    /// 並把晚到結果寫進新的畫面。
    fn leaving_size_linemode_cancels_panel_scan() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("child")).expect("child");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.apply_line_mode(1, LineMode::Size).expect("enable size");
        app.apply_line_mode(1, LineMode::None)
            .expect("disable size");

        assert!(app.directory_size_jobs.is_empty());
    }

    #[test]
    /// 驗證背景貼上刷新目的根目錄時，也會更新已經進入其子目錄的 panel。
    ///
    /// 保護目的：使用者可能在大型 copy 尚未完成時進入新建立的 `target/`；舊流程只
    /// 刷新目的父目錄，子目錄 panel 會一直保持空白直到整批結束。測試同時確認目的
    /// 樹以外的 panel 不會被無關進度反覆 reload。
    fn background_destination_refresh_updates_open_descendant_panels_only() {
        let dir = tempdir().expect("tempdir");
        let destination = dir.path().join("destination");
        let child = destination.join("target");
        let unrelated = dir.path().join("unrelated");
        fs::create_dir_all(&child).expect("destination child");
        fs::create_dir(&unrelated).expect("unrelated dir");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .insert(2, PaneState::new(child.clone()).expect("child panel"));
        app.panes.insert(
            3,
            PaneState::new(unrelated.clone()).expect("unrelated panel"),
        );
        fs::write(child.join("copied.txt"), b"visible").expect("copied file");
        fs::write(unrelated.join("not-reloaded.txt"), b"hidden").expect("unrelated file");

        app.reload_panes_in_tree(&destination)
            .expect("refresh destination tree");

        assert!(
            app.panes[&2]
                .entries
                .iter()
                .any(|entry| entry.name == "copied.txt")
        );
        assert!(
            app.panes[&3]
                .entries
                .iter()
                .all(|entry| entry.name != "not-reloaded.txt")
        );
    }

    #[test]
    /// 驗證 worker 會保留原始 byte 變化，而不是只在整數百分比改變時回報。
    ///
    /// 保護目的：若每個 1 MiB buffer 都排入 channel，數十 GB 傳輸會讓主執行緒忙於
    /// 處理重複資料；時間節流由 worker 負責，這裡確保完全相同的快照不會重送。
    fn progress_events_preserve_byte_changes_and_skip_exact_duplicates() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut last_progress = None;

        super::send_progress_if_changed(&sender, 7, 10, 1_000, &mut last_progress);
        super::send_progress_if_changed(&sender, 7, 10, 1_000, &mut last_progress);
        super::send_progress_if_changed(&sender, 7, 11, 1_000, &mut last_progress);

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(last_progress, Some((11, 1_000)));
        assert!(matches!(
            events.last(),
            Some(super::FileJobEvent::Progress {
                task_id: 7,
                completed_bytes: 11,
                total_bytes: 1_000,
            })
        ));
    }

    #[test]
    /// 驗證單次走訪的動態總量增加時，task 會保留新的 byte 分母。
    ///
    /// 保護目的：目錄 producer 會一邊發現檔案、一邊複製；若只允許百分比增加，前幾個
    /// 小檔完成時若只保留百分比，後續增加總量會掩蓋真實進度。
    fn progress_events_follow_dynamic_discovered_total() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut last_progress = None;

        super::send_progress_if_changed(&sender, 8, 90, 100, &mut last_progress);
        super::send_progress_if_changed(&sender, 8, 90, 1_000, &mut last_progress);

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(last_progress, Some((90, 1_000)));
        assert!(matches!(
            events.last(),
            Some(super::FileJobEvent::Progress {
                task_id: 8,
                completed_bytes: 90,
                total_bytes: 1_000,
            })
        ));
    }

    #[test]
    /// 驗證 `x` 剪下後可以用 `p` 移動檔案，且剪貼簿會在成功後清空。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_cut_and_paste_moves_file_and_clears_clipboard() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("beta.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .expect("cut");

        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.operation),
            Some(ClipboardOperation::Cut)
        );
        assert_eq!(
            app.clipboard.as_ref().map(|entry| entry.entries.len()),
            Some(1)
        );

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("paste");

        assert!(!source_file.exists());
        assert!(target_dir.join("beta.txt").exists());
        assert!(app.clipboard.is_none());
        assert_eq!(app.status, "moved: 1 item");
    }

    #[test]
    /// 驗證一般模式按下 `u` 會復原最近一次 Copy，來源保留且目的檔移入 Trash。
    ///
    /// 保護目的：確保操作歷史不只底層可用，實際快捷鍵也能完成使用者最常見的貼錯復原。
    fn app_u_undoes_latest_copy_paste() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("alpha.txt");
        let target_file = target_dir.join("alpha.txt");
        fs::write(&source_file, "hello").expect("source file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.copy_selected();
        app.current_pane_mut().expect("pane").cwd = target_dir;
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.paste_into_focused_pane().expect("paste");
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
            .expect("undo shortcut");

        assert!(source_file.exists());
        assert!(!target_file.exists());
        assert_eq!(app.status, "undid copy: 1 items");
        assert_eq!(
            app.trash_store.list_entries().expect("trash entries").len(),
            1
        );
    }

    #[test]
    /// 驗證 `:undo` 可以把 cut/paste 的目的檔搬回原來源位置。
    ///
    /// 保護目的：命令與快捷鍵必須呼叫相同 Undo 核心，避免兩套入口行為不一致。
    fn app_undo_command_reverses_cut_paste() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("beta.txt");
        let target_file = target_dir.join("beta.txt");
        fs::write(&source_file, "hello").expect("source file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.cut_selected();
        app.current_pane_mut().expect("pane").cwd = target_dir;
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.paste_into_focused_pane().expect("paste");
        app.execute_command("undo").expect("undo command");

        assert!(source_file.exists());
        assert!(!target_file.exists());
        assert_eq!(app.status, "undid move: 1 items");
    }

    #[test]
    /// 驗證覆蓋貼上後執行 Undo，會恢復目的地原內容而不是只移除新檔。
    ///
    /// 保護目的：覆蓋是最高風險操作，必須證明隱藏備份已接入 App 的完整流程。
    fn app_undo_overwrite_paste_restores_previous_target() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        fs::write(source_dir.join("same.txt"), "new").expect("new source");
        let target_file = target_dir.join("same.txt");
        fs::write(&target_file, "old").expect("old target");

        let mut app = App::new(source_dir, default_loaded_config()).expect("app");
        app.copy_selected();
        app.current_pane_mut().expect("pane").cwd = target_dir;
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.paste_into_focused_pane_with_overwrite()
            .expect("overwrite paste");
        app.undo_latest_file_operation().expect("undo overwrite");

        assert_eq!(
            fs::read_to_string(target_file).expect("restored target"),
            "old"
        );
        assert_eq!(app.status, "undid copy: 1 items");
    }

    #[test]
    /// 驗證按下 `Space` 會切換目前選取項目的標記狀態。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_space_toggles_mark_on_selected_entry() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .expect("mark selected");
        assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 1);
        assert_eq!(app.status, "marked alpha.txt");

        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .expect("unmark selected");
        assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
        assert_eq!(app.status, "unmarked alpha.txt");
    }

    #[test]
    /// 驗證 `w h/j/k/l` 會依方向在左下上右建立新的 pane。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_w_leader_splits_in_four_directions() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("open w leader");
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("split left");
        assert_eq!(app.ordered_pane_ids(), vec![2, 1]);
        assert_eq!(app.focused_pane, 2);
        assert_eq!(app.status, "split left");

        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("open w leader");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("split down");
        assert_eq!(app.focused_pane, 3);
        assert_eq!(app.status, "split down");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("open w leader");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("split up");
        assert_eq!(app.ordered_pane_ids(), vec![2, 1]);
        assert_eq!(app.focused_pane, 2);
        assert_eq!(app.status, "split up");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE))
            .expect("open w leader");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("split right");
        assert_eq!(app.ordered_pane_ids(), vec![1, 2]);
        assert_eq!(app.focused_pane, 2);
        assert_eq!(app.status, "split right");
    }

    #[test]
    /// 驗證按下 `Ctrl-r` 會反轉目前所有可見項目的標記狀態。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_r_inverts_visible_marks() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .expect("invert marks");
        assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);
        assert_eq!(app.status, "inverted visible marks (+2, -0, total 2)");

        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
            .expect("invert marks again");
        assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 0);
        assert_eq!(app.status, "inverted visible marks (+0, -2, total 0)");
    }

    #[test]
    /// 驗證按下 `Y` / `X` 可以清掉目前內部剪貼簿中的 copy / cut 狀態。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_y_and_shift_x_clear_clipboard_state() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy");
        app.handle_key(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT))
            .expect("clear copied items");
        assert!(app.clipboard.is_none());
        assert_eq!(app.status, "cleared copied items");

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .expect("cut");
        app.handle_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
            .expect("clear cut items");
        assert!(app.clipboard.is_none());
        assert_eq!(app.status, "cleared cut items");
    }

    #[test]
    /// 驗證按下 `P` 會以覆蓋模式貼上，而不是自動產生 `copy` 檔名。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_p_pastes_with_overwrite_when_clipboard_exists() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source");
        fs::create_dir(&target_dir).expect("target");
        let source_file = source_dir.join("alpha.txt");
        let target_file = target_dir.join("alpha.txt");
        fs::write(&source_file, "from source").expect("source file");
        fs::write(&target_file, "from target").expect("target file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy");

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload target");
        app.handle_key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT))
            .expect("overwrite paste");

        assert_eq!(
            fs::read_to_string(&target_file).expect("target content"),
            "from source"
        );
        assert!(!target_dir.join("alpha copy.txt").exists());
        assert_eq!(app.status, "pasted copy with overwrite: 1 item");
    }

    #[test]
    /// 驗證按下 `D` 後確認，會直接永久刪除目前選取項目而不是丟進 trash。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_d_deletes_selected_entry_permanently() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT))
            .expect("start permanent delete");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete {
                permanent: true,
                ..
            })
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm permanent delete");

        for _ in 0..100 {
            app.poll_background_tasks();
            if app.file_job_receivers.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        assert!(!file_path.exists());
        assert!(
            app.trash_store
                .list_entries()
                .expect("trash entries")
                .is_empty()
        );
        assert_eq!(app.status, "deleted permanently delete-me.txt");
    }

    #[test]
    /// 驗證 `:move <path>` 會把目前選取的檔案直接移到指定目錄。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_move_command_moves_selected_entry_to_target_dir() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("gamma.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.execute_command(&format!("move {}", target_dir.display()))
            .expect("move command");

        assert!(!source_file.exists());
        assert!(target_dir.join("gamma.txt").exists());
        assert_eq!(
            app.status,
            format!("moved 1 item -> {}", target_dir.display())
        );
    }

    #[test]
    /// 驗證 `:move-panel <id>` 會把目前選取的檔案移到指定 pane 的目錄。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_move_panel_command_moves_selected_entry_to_target_pane_dir() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source dir");
        fs::create_dir(&target_dir).expect("target dir");
        let source_file = source_dir.join("delta.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");
        app.focus_pane_by_id(1);

        app.execute_command("move-panel 2").expect("move panel");

        assert!(!source_file.exists());
        assert!(target_dir.join("delta.txt").exists());
        assert_eq!(
            app.status,
            format!("moved 1 item -> {}", target_dir.display())
        );
    }

    #[test]
    /// 驗證 `:compress` 會把目前選取項目壓成 zip，並把游標帶到新壓縮檔。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_compress_command_creates_zip_and_reveals_result() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("notes.txt");
        fs::write(&file_path, "hello zip").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("compress").expect("compress");

        let archive_path = dir.path().join("notes.txt.zip");
        assert!(archive_path.exists());
        assert_eq!(app.status, "compressed notes.txt -> notes.txt.zip");
        assert_eq!(
            app.current_pane_mut()
                .expect("pane")
                .selected_entry()
                .expect("selected")
                .name,
            "notes.txt.zip"
        );
    }

    #[test]
    /// 驗證 `:extract` 會解開目前選取的 zip，並將游標帶到輸出目錄。
    /// 保護目的：同時確認資料夾壓縮會先排入背景 task，完成後仍能選中新 ZIP 並接續
    /// 解壓，避免非阻塞重構破壞原本的完整操作流程。
    fn app_extract_command_unpacks_zip_and_reveals_output() {
        let dir = tempdir().expect("tempdir");
        let folder = dir.path().join("demo");
        fs::create_dir(&folder).expect("dir");
        fs::write(folder.join("alpha.txt"), "hello").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("compress").expect("compress dir");
        assert!(
            !app.file_job_receivers.is_empty(),
            "directory compression must not block the TUI thread"
        );
        wait_for_file_jobs(&mut app);

        let archive_path = dir.path().join("demo.zip");
        assert!(archive_path.exists());

        app.execute_command("extract").expect("extract zip");

        let extracted_dir = dir.path().join("demo copy");
        assert!(extracted_dir.is_dir());
        assert!(extracted_dir.join("demo").join("alpha.txt").exists());
        assert_eq!(app.status, "extracted demo copy");
        assert_eq!(
            app.current_pane_mut()
                .expect("pane")
                .selected_entry()
                .expect("selected")
                .name,
            "demo copy"
        );
    }

    #[test]
    /// 驗證已掛載的 SMB share 可以直接經由 `goto smb://...` 切進目前 pane。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_goto_smb_location_enters_mounted_share() {
        let dir = tempdir().expect("tempdir");
        let mount_root = dir.path().join("mounts");
        let share_root = mount_root.join("shared");
        fs::create_dir_all(share_root.join("docs")).expect("share docs");
        fs::write(share_root.join("docs").join("report.txt"), "hello").expect("report");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.goto_smb_location_with_mount_root("smb://192.0.2.10/shared/docs", &mount_root)
            .expect("goto smb");

        let pane = app.current_pane_mut().expect("pane");
        assert_eq!(pane.cwd, share_root.join("docs"));
        assert_eq!(app.status, "jumped to smb: smb://192.0.2.10/shared/docs");
        assert!(app.take_full_redraw_request());
        assert!(!app.take_full_redraw_request());
    }

    #[test]
    /// 驗證尚未掛載的 SMB share 在 `goto smb://...` 時會先發出系統掛載請求。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_goto_smb_location_requests_mount_when_share_missing() {
        let dir = tempdir().expect("tempdir");
        let mount_root = dir.path().join("mounts");
        fs::create_dir_all(&mount_root).expect("mount root");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.goto_smb_location_with_mount_root("smb://192.0.2.10/shared/docs", &mount_root)
            .expect("goto smb");

        assert!(app.pending_launch.is_some());
        assert_eq!(
            app.status,
            format!(
                "已請求系統掛載 SMB：smb://192.0.2.10/shared/docs；若系統連線失敗，請檢查主機、share 名稱、網路與權限，成功後再重試。預期掛載位置：{}",
                mount_root.join("shared").join("docs").display()
            )
        );
    }

    #[test]
    /// 驗證 `:move-panel <id>` 若指定不存在的 pane，會提示目前可用的 pane 編號。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_move_panel_command_reports_available_panes_for_unknown_target() {
        let dir = tempdir().expect("tempdir");
        let source_file = dir.path().join("epsilon.txt");
        fs::write(&source_file, "hello").expect("file");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("move-panel 9").expect("move panel");

        assert!(source_file.exists());
        assert_eq!(app.status, "unknown panel 9. available: 1");
    }

    #[test]
    /// 驗證按下 `o` 後會打開建立新檔案的 inline 輸入框。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_start_create_entry_opens_inline_editor() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.start_create_entry();

        assert_eq!(
            app.pending_action,
            Some(PendingAction::CreateEntry {
                pane_id: 1,
                buffer: String::new(),
                cursor: 0,
                mode: RenameMode::Insert,
            })
        );
    }

    #[test]
    /// 驗證命令模式可以直接建立一般檔案與結尾 `/` 的資料夾。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_create_commands_create_entries_without_inline_prompt() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.execute_command("create alpha.txt")
            .expect("create file");
        assert!(dir.path().join("alpha.txt").exists());
        assert_eq!(app.status, "created file: alpha.txt");

        app.execute_command("create docs/").expect("create dir");
        assert!(dir.path().join("docs").is_dir());
        assert_eq!(app.status, "created directory: docs/");
    }

    #[test]
    /// 驗證建立流程的 inline 輸入框在 Enter 後會真的建立檔案。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_create_file_confirm_creates_entry() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::CreateEntry {
            pane_id: 1,
            buffer: String::from("draft.md"),
            cursor: 8,
            mode: RenameMode::Insert,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("create file");

        assert!(dir.path().join("draft.md").exists());
        assert_eq!(app.status, "created file: draft.md");
    }

    #[test]
    /// 驗證建立流程支援巢狀路徑，會先補齊父目錄再建立檔案。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_create_nested_file_from_inline_prompt() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::CreateEntry {
            pane_id: 1,
            buffer: String::from("test/gg.txt"),
            cursor: 11,
            mode: RenameMode::Insert,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("create nested file");

        assert!(dir.path().join("test").is_dir());
        assert!(dir.path().join("test").join("gg.txt").exists());
        assert_eq!(app.status, "created file: test/gg.txt");
    }

    #[test]
    /// 驗證 filter 第一次 Esc 只進入 Normal 模式，輸入框與過濾結果都會保留。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_filter_first_escape_enters_normal_mode() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("enter normal mode");

        let pane = app.panes.get(&1).expect("pane");
        let visible_names: Vec<String> = pane
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();

        assert_eq!(
            visible_names,
            vec![String::from("alpha.txt"), String::from("beta.txt")]
        );
        assert!(app.filter.as_ref().is_some_and(|filter| filter.editing));
        assert_eq!(app.text_input_mode, RenameMode::Normal);
    }

    #[test]
    /// 驗證一般 Filter 與 Preview Search 都會畫在其狀態所屬的左側 Panel 內。
    /// 保護目的：避免繪圖重構時重新使用全畫面 `frame.area()`，導致多 Panel 的輸入框
    /// 跑到整個 terminal 右上角，或覆蓋其他 Panel。
    fn app_filter_inputs_render_inside_their_target_panel() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.focus_pane_by_id(1);
        assert_eq!(app.focused_pane, 1);

        app.open_filter_input(FilterMode::Normal);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| {
                let _ = app.render(frame);
            })
            .expect("render panel filter");
        let filter_x = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .position(|cell| cell.symbol() == "F")
            .map(|index| index % 80)
            .expect("Filter title");
        assert!(filter_x < 40, "Filter must stay in panel 1");

        app.filter = None;
        app.open_preview_focus();
        app.open_preview_search_input();
        terminal
            .draw(|frame| {
                let _ = app.render(frame);
            })
            .expect("render preview search");
        let preview_search_x = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .enumerate()
            .find_map(|(index, cell)| {
                (cell.symbol() == "P" && index / 80 < 4).then_some(index % 80)
            })
            .expect("Preview Search title");
        assert!(preview_search_x < 40, "Preview Search must stay in panel 1");
    }

    #[test]
    /// 驗證第二次 Esc 收起 filter 輸入框，第三次 Esc 才清掉已鎖定的 filter。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_filter_escape_flow_locks_then_clears_filter() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("enter normal mode");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close input");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear filter");

        let pane = app.panes.get(&1).expect("pane");
        let visible_names: Vec<String> = pane
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();

        assert_eq!(
            visible_names,
            vec![String::from("alpha.txt"), String::from("beta.txt")]
        );
        assert!(app.filter.is_none());
        assert!(!pane.has_active_filter());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證連續重新開啟 filter 時，不會殘留上一輪輸入的關鍵字。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_reopening_filter_starts_with_empty_buffer() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close input");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear filter");

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("reopen filter");

        assert_eq!(
            app.filter,
            Some(FilterState {
                pane_id: 1,
                buffer: String::new(),
                editing: true,
                mode: FilterMode::Normal,
            })
        );
        assert_eq!(app.status, "filter [normal]: all (Tab to switch)");
    }

    #[test]
    /// 驗證 filter 輸入框中的 `Tab` 不會被當成 command 補齊，而是切換為模糊過濾模式（Fuzzy）。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_filter_input_tab_does_not_apply_command_autocomplete() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("recent.txt"), "a").expect("recent");
        fs::write(dir.path().join("rename.txt"), "b").expect("rename");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("tab in filter");

        assert_eq!(
            app.filter,
            Some(FilterState {
                pane_id: 1,
                buffer: String::from("re"),
                editing: true,
                mode: FilterMode::Fuzzy,
            })
        );
        assert_eq!(app.status, "filter [fuzzy]: re");
    }

    #[test]
    /// 驗證一般檔案列表的 `f` 預設以逐詞包含過濾，不接受不連續字元命中。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，意外讓大型目錄回到昂貴的模糊排序。
    fn app_file_list_filter_uses_all_terms_as_contiguous_substrings() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("file-manager-app.rs"), "app").expect("app");
        fs::write(dir.path().join("sample.txt"), "sample").expect("sample");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        for ch in ['f', 'i', 'l', 'e', ' ', 'a', 'p', 'p'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type filter");
        }

        let visible: Vec<String> = app
            .panes
            .get(&1)
            .expect("pane")
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();
        assert_eq!(visible, vec![String::from("file-manager-app.rs")]);
    }

    #[test]
    /// 驗證包含 `~` 符號的特殊檔名在一般過濾模式下能被精確連續字串比對，不會被誤當成模式切換語法。
    fn app_file_list_filter_matches_literal_tilde_in_filenames() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("~backup.rs"), "app").expect("app");
        fs::write(dir.path().join("sample.txt"), "sample").expect("sample");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        for ch in ['~', 'b', 'a', 'c', 'k'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type filter with tilde");
        }
        assert_eq!(app.filter.as_ref().expect("filter").buffer, "~back");

        let visible: Vec<String> = app
            .panes
            .get(&1)
            .expect("pane")
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();
        assert_eq!(visible, vec![String::from("~backup.rs")]);
    }

    #[test]
    /// 驗證按下 `.` 後會顯示隱藏檔，並可與 filter 一起使用。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_toggle_hidden_reveals_hidden_entries_and_works_with_filter() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".secret"), "s").expect("hidden");
        fs::write(dir.path().join("alpha.txt"), "a").expect("normal");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        let initial_names: Vec<String> = app
            .panes
            .get(&1)
            .expect("pane")
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();
        assert_eq!(initial_names, vec![String::from("alpha.txt")]);

        app.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
            .expect("toggle hidden");
        assert_eq!(app.status, "showing hidden files");

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open filter");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("filter hidden");

        let filtered_names: Vec<String> = app
            .panes
            .get(&1)
            .expect("pane")
            .visible_entries()
            .into_iter()
            .map(|entry| entry.display_name())
            .collect();
        assert_eq!(filtered_names, vec![String::from(".secret")]);
    }

    #[test]
    /// 驗證按下 `,` 後可以用排序面板快捷鍵套用排序模式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_sort_picker_applies_selected_mode() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("small.txt"), "a").expect("small");
        fs::write(dir.path().join("large.txt"), "abcdef").expect("large");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
            .expect("open sort picker");
        assert_eq!(
            app.pending_action,
            Some(PendingAction::SortPicker { pane_id: 1 })
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("sort by size");
        assert_eq!(app.status, "sort: size");
        assert_eq!(
            app.panes.get(&1).expect("pane").sort_mode,
            SortMode::Size { reverse: false }
        );

        app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
            .expect("open sort picker again");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE))
            .expect("sort by modified reverse");
        assert_eq!(app.status, "sort: modified (reverse)");
        assert_eq!(
            app.panes.get(&1).expect("pane").sort_mode,
            SortMode::Modified { reverse: true }
        );
    }

    #[test]
    /// 驗證 sort panel 也接受 `m + Shift` 這類終端事件，正確套用反向排序。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_sort_picker_shift_m_applies_reverse_modified() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE))
            .expect("open sort picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::SHIFT))
            .expect("sort by modified reverse");

        assert_eq!(app.status, "sort: modified (reverse)");
        assert_eq!(
            app.panes.get(&1).expect("pane").sort_mode,
            SortMode::Modified { reverse: true }
        );
    }

    #[test]
    /// 驗證進入 preview mode 後，`j/k` 會改成捲動 preview，Esc 會離開該模式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_mode_scrolls_and_exits_cleanly() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("notes.txt"),
            "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
        )
        .expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(4);

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("open preview");
        assert!(app.panes.get(&1).expect("pane").is_preview_active());
        assert_eq!(app.status, "preview mode");

        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("scroll down");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("scroll up");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("leave preview");
        assert!(!app.panes.get(&1).expect("pane").is_preview_active());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證按下 `Tab` 會切換 preview mode，再按一次同樣的鍵會回到一般列表。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_mode_toggles_with_tab() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "preview").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("open preview");
        assert!(app.panes.get(&1).expect("pane").is_preview_active());
        assert_eq!(app.status, "preview mode");

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("toggle preview off");
        assert!(!app.panes.get(&1).expect("pane").is_preview_active());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 preview mode 支援半頁捲動與 `gg/G` 的上下端跳轉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_mode_supports_paging_and_boundary_jumps() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("readme.md"),
            "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n",
        )
        .expect("readme");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(4);
        app.open_preview_focus();

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("page down");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE))
            .expect("bottom");
        let bottom_scroll = app.panes.get(&1).expect("pane").preview_scroll;
        assert!(bottom_scroll > 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("pending g");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("top");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("page down again");
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .expect("page up");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 0);
    }

    #[test]
    /// 驗證 preview mode 中的 `/` 會打開搜尋輸入框，並在輸入時立即更新搜尋結果。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_search_opens_and_tracks_matches() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("notes.txt"),
            "alpha\nbeta\ngamma\nbeta line\n",
        )
        .expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(3);
        app.open_preview_focus();

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("open preview search");
        assert!(
            app.preview_search
                .as_ref()
                .is_some_and(|search| search.editing)
        );

        for ch in ['b', 'e', 't', 'a'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type search");
        }
        assert_eq!(app.panes.get(&1).expect("pane").preview_match_count(), 2);
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);
        assert_eq!(app.status, "preview search: beta (2)");
    }

    #[test]
    /// 驗證 preview search 支援 `n/N` 跳轉命中結果，Esc 先清搜尋再離開 preview mode。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_search_navigation_and_escape_flow() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("readme.md"),
            "zero\nmatch one\nmiddle\nmatch two\nend\n",
        )
        .expect("readme");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(3);
        app.open_preview_focus();
        app.open_preview_search_input();
        for ch in ['m', 'a', 't', 'c', 'h'] {
            app.handle_preview_search_input_key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ))
            .expect("type query");
        }
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock search");

        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("next match");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("previous match by p");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("wrap to last match");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::NONE))
            .expect("previous match by N");
        assert_eq!(app.panes.get(&1).expect("pane").preview_scroll, 1);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear search");
        assert!(app.panes.get(&1).expect("pane").is_preview_active());
        assert!(!app.panes.get(&1).expect("pane").has_preview_search());
        assert_eq!(app.status, "preview search cleared");

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("leave preview");
        assert!(!app.panes.get(&1).expect("pane").is_preview_active());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 preview search 在同一行有多個命中時，`n/p` 仍會逐一輪詢每個命中位置。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_search_cycles_each_match_occurrence() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "tt line\nonly t here\n").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_preview_viewport_height(4);
        app.open_preview_focus();
        app.open_preview_search_input();
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock search");

        assert_eq!(app.panes.get(&1).expect("pane").preview_match_count(), 3);
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_current_match,
            Some(0)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("next occurrence on same line");
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_current_match,
            Some(1)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("move to next line occurrence");
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_current_match,
            Some(2)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("wrap to first occurrence");
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_current_match,
            Some(0)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("wrap back to last occurrence");
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_current_match,
            Some(2)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("move back to same-line occurrence");
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_current_match,
            Some(1)
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("move back to first occurrence on same line");
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_current_match,
            Some(0)
        );
    }

    #[test]
    /// 驗證 preview search 重新打開時，不會殘留上一次輸入的查詢字串。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_search_reopen_starts_with_empty_buffer() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "alpha\nbeta\ngamma\n").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_preview_focus();
        app.open_preview_search_input();
        for ch in ['b', 'e', 't', 'a'] {
            app.handle_preview_search_input_key(KeyEvent::new(
                KeyCode::Char(ch),
                KeyModifiers::NONE,
            ))
            .expect("type query");
        }
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock search");
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_search_query(),
            Some("beta")
        );

        app.open_preview_search_input();

        assert!(
            app.preview_search
                .as_ref()
                .is_some_and(|search| search.buffer.is_empty() && search.editing)
        );
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_search_query(),
            None
        );
        assert_eq!(app.status, "preview search: all");
    }

    #[test]
    /// 驗證 preview search 輸入框中的 `Tab` 不會誤套用 command 補齊。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_preview_search_tab_does_not_apply_command_autocomplete() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("notes.txt"), "recent\nrename\n").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_preview_focus();
        app.open_preview_search_input();
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("tab in preview search");

        assert!(
            app.preview_search
                .as_ref()
                .is_some_and(|search| search.buffer == "re" && search.editing)
        );
        assert_eq!(
            app.panes.get(&1).expect("pane").preview_search_query(),
            Some("re")
        );
        assert_eq!(app.status, "preview search: re (2)");
    }

    #[test]
    /// 驗證 `Ctrl+s` / `Ctrl+v` 仍可作為分割 alias 使用。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_split_shortcuts_create_expected_panes() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL))
            .expect("ctrl-s split");
        assert_eq!(app.panes.len(), 2);
        assert_eq!(app.focused_pane, 2);

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL))
            .expect("ctrl-v split");
        assert_eq!(app.panes.len(), 3);
        assert_eq!(app.focused_pane, 3);
    }

    #[test]
    /// 驗證三個 panel 可以各自打開 preview，且關閉其中一個不會影響另外兩個。
    /// 保護目的：防止 preview 開關退回 `App` 全域單一狀態，造成後開啟的 panel 關掉
    /// 其他 panel 已顯示的 preview。
    fn app_preview_mode_is_scoped_to_its_own_pane() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "beta").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        assert_eq!(app.focused_pane, 2);

        app.open_preview_focus();
        assert!(app.panes.get(&2).expect("panel 2").is_preview_active());

        app.focus_pane_by_id(1);
        app.open_preview_focus();
        assert!(app.panes.get(&1).expect("panel 1").is_preview_active());
        assert!(app.panes.get(&2).expect("panel 2").is_preview_active());

        app.split_current(SplitDirection::Horizontal)
            .expect("split third panel");
        app.open_preview_focus();
        assert_eq!(app.focused_pane, 3);
        assert!(app.panes.get(&1).expect("panel 1").is_preview_active());
        assert!(app.panes.get(&2).expect("panel 2").is_preview_active());
        assert!(app.panes.get(&3).expect("panel 3").is_preview_active());

        app.focus_pane_by_id(2);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("close only panel 2 preview");
        assert!(app.panes.get(&1).expect("panel 1").is_preview_active());
        assert!(!app.panes.get(&2).expect("panel 2").is_preview_active());
        assert!(app.panes.get(&3).expect("panel 3").is_preview_active());
    }

    #[test]
    /// 驗證 global search 在輸入階段不會立即掃描，按下 Enter 後才真正執行搜尋。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_global_search_filters_nested_entries() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("Readme.md"), "doc").expect("readme");
        fs::create_dir(dir.path().join("src")).expect("src");
        fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").expect("main");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");

        for ch in ['r', 'e', 'a', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        let search = app.global_search.as_ref().expect("search");
        assert!(search.editing);
        assert_eq!(search.results.len(), 0);
        assert!(!search.searched);
        assert_eq!(
            app.status,
            "global search (insert): read (press Enter to search)"
        );

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("run search");
        wait_for_global_search(&mut app);
        let search = app.global_search.as_ref().expect("search after run");
        assert!(!search.editing);
        assert!(search.searched);
        assert_eq!(search.results.len(), 1);
        assert_eq!(search.results[0].relative_path, "docs/Readme.md");
        assert_eq!(app.status, "global search (normal): read (1)");
    }

    #[test]
    /// 驗證 global search 提交查詢後，再按一次 Enter 會跳到選中的搜尋結果。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_global_search_enter_reveals_selected_file() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");
        for ch in ['g', 'u', 'i', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock search");
        wait_for_global_search(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open result");

        assert!(app.global_search.is_none());
        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.cwd, dir.path().join("docs"));
        assert_eq!(
            pane.selected_entry().map(|entry| entry.display_name()),
            Some(String::from("guide.md"))
        );
        assert_eq!(app.status, "search opened: docs/guide.md");
    }

    #[test]
    /// 驗證在 global search 執行中按下 Esc，會關閉介面並要求背景搜尋停止。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_global_search_escape_cancels_background_work() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");
        for ch in ['g', 'u', 'i', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("start search");
        let cancelled = app
            .global_search_cancelled
            .as_ref()
            .expect("cancel flag")
            .clone();

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel search");

        assert!(app.global_search.is_none());
        assert!(app.global_search_rx.is_none());
        assert!(app.global_search_cancelled.is_none());
        assert!(cancelled.load(Ordering::Relaxed));
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證在 global search 結果列表中按下 h，會安全返回一般列表。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_global_search_h_leaves_results_list() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "guide").expect("guide");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .expect("open search");
        for ch in ['g', 'u', 'i', 'd'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("start search");
        wait_for_global_search(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("leave search");

        assert!(app.global_search.is_none());
        assert!(app.global_search_rx.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證 `Shift+S` 會打開內容搜尋面板，而不是一般路徑搜尋。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_s_opens_content_search() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
            .expect("open content search");

        let search = app.global_search.as_ref().expect("search");
        assert_eq!(search.mode, SearchMode::Content);
        assert!(search.editing);
        assert_eq!(app.status, "content search (insert): type query and Enter");
    }

    #[test]
    /// 驗證 `s` 與 `S` 的結果仍在串流載入時，只要列表已有內容就能立即用游標鍵與 Vim 鍵移動。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_search_lists_move_immediately_while_loading() {
        let dir = tempdir().expect("tempdir");

        for mode in [SearchMode::Path, SearchMode::Content] {
            let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
            app.global_search = Some(GlobalSearchState {
                pane_id: 1,
                root_dir: dir.path().to_path_buf(),
                mode,
                buffer: String::from("target"),
                editing: false,
                loading: true,
                searched: true,
                selected: 0,
                results: ["alpha.txt", "beta.txt", "gamma.txt"]
                    .into_iter()
                    .map(|name| GlobalSearchEntry {
                        path: dir.path().join(name),
                        relative_path: name.to_string(),
                        is_dir: false,
                        match_line_number: None,
                        match_column: None,
                        match_preview: None,
                    })
                    .collect(),
                filter: PanelSearchState::default(),
                preview_scroll: None,
                preview_current_match: None,
                task_id: None,
            });

            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .expect("move down while loading");
            assert_eq!(app.global_search.as_ref().expect("search").selected, 1);

            app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
                .expect("vim move down while loading");
            assert_eq!(app.global_search.as_ref().expect("search").selected, 2);

            app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                .expect("move up while loading");
            assert_eq!(app.global_search.as_ref().expect("search").selected, 1);

            app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
                .expect("vim move up while loading");
            assert_eq!(app.global_search.as_ref().expect("search").selected, 0);
        }
    }

    #[test]
    /// 驗證 `s` 與 `S` 的結果面板都能按 `f` 開啟模糊 filter，並以不連續字元縮小結果。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_search_result_panels_support_fuzzy_filtering() {
        let dir = tempdir().expect("tempdir");

        for mode in [SearchMode::Path, SearchMode::Content] {
            let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
            app.global_search = Some(GlobalSearchState {
                pane_id: 1,
                root_dir: dir.path().to_path_buf(),
                mode,
                buffer: String::from("source"),
                editing: false,
                loading: false,
                searched: true,
                selected: 0,
                results: ["src/file_manager/app.rs", "docs/sample.txt", "README.md"]
                    .into_iter()
                    .map(|name| GlobalSearchEntry {
                        path: dir.path().join(name),
                        relative_path: name.to_string(),
                        is_dir: false,
                        match_line_number: None,
                        match_column: None,
                        match_preview: None,
                    })
                    .collect(),
                filter: PanelSearchState::default(),
                preview_scroll: None,
                preview_current_match: None,
                task_id: None,
            });

            app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
                .expect("open result filter");
            for ch in ['f', 'm', 'a'] {
                app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                    .expect("type fuzzy result filter");
            }

            let search = app.global_search.as_ref().expect("search");
            assert!(search.filter.editing);
            assert_eq!(search.filter.buffer, "fma");
            let visible = filtered_global_search_entries(&search.results, &search.filter.buffer);
            assert_eq!(visible.len(), 1);
            assert_eq!(visible[0].relative_path, "src/file_manager/app.rs");
        }
    }

    #[test]
    /// 驗證從模糊過濾後的搜尋列表按 Enter，會開啟目前可見結果而非原始索引項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_search_filter_opens_filtered_selection() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "alpha").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "beta").expect("beta");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.global_search = Some(GlobalSearchState {
            pane_id: 1,
            root_dir: dir.path().to_path_buf(),
            mode: SearchMode::Path,
            buffer: String::from("txt"),
            editing: false,
            loading: false,
            searched: true,
            selected: 0,
            results: ["alpha.txt", "beta.txt"]
                .into_iter()
                .map(|name| GlobalSearchEntry {
                    path: dir.path().join(name),
                    relative_path: name.to_string(),
                    is_dir: false,
                    match_line_number: None,
                    match_column: None,
                    match_preview: None,
                })
                .collect(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: None,
        });

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open result filter");
        for ch in ['b', 't'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type result filter");
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock result filter");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open filtered result");

        assert!(app.global_search.is_none());
        assert_eq!(
            app.panes
                .get(&1)
                .and_then(|pane| pane.selected_entry())
                .map(|entry| entry.display_name()),
            Some(String::from("beta.txt"))
        );
    }

    #[test]
    /// 驗證 `s` 與 `S` 收到新批次時只會追加到下方，不會重排既有列表或移動游標。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_search_stream_appends_without_reordering_existing_rows() {
        let dir = tempdir().expect("tempdir");
        for mode in [SearchMode::Path, SearchMode::Content] {
            let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
            app.global_search = Some(GlobalSearchState {
                pane_id: 1,
                root_dir: dir.path().to_path_buf(),
                mode,
                buffer: String::from("txt"),
                editing: false,
                loading: true,
                searched: true,
                selected: 0,
                results: vec![GlobalSearchEntry {
                    path: dir.path().join("beta.txt"),
                    relative_path: String::from("beta.txt"),
                    is_dir: false,
                    match_line_number: None,
                    match_column: None,
                    match_preview: None,
                }],
                filter: PanelSearchState::default(),
                preview_scroll: None,
                preview_current_match: None,
                task_id: None,
            });
            let (sender, receiver) = std::sync::mpsc::channel();
            app.global_search_rx = Some(receiver);
            sender
                .send(GlobalSearchEvent::Chunk {
                    pane_id: 1,
                    query: String::from("txt"),
                    entries: vec![GlobalSearchEntry {
                        path: dir.path().join("alpha.txt"),
                        relative_path: String::from("alpha.txt"),
                        is_dir: false,
                        match_line_number: None,
                        match_column: None,
                        match_preview: None,
                    }],
                })
                .expect("send result chunk");

            app.poll_background_tasks();

            let search = app.global_search.as_ref().expect("search");
            assert_eq!(search.selected, 0);
            assert_eq!(search.results[0].relative_path, "beta.txt");
            assert_eq!(search.results[1].relative_path, "alpha.txt");
        }
    }

    #[test]
    /// 驗證內容搜尋會依照檔案內容比對結果，並只回傳真正命中的檔案。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_content_search_matches_file_contents() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("guide.md"), "release note").expect("guide");
        fs::write(dir.path().join("todo.txt"), "buy milk").expect("todo");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
            .expect("open content search");
        for ch in ['r', 'e', 'l', 'e', 'a', 's', 'e'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("run content search");
        wait_for_global_search(&mut app);

        let search = app.global_search.as_ref().expect("search");
        assert_eq!(search.mode, SearchMode::Content);
        assert!(!search.editing);
        assert!(search.searched);
        assert_eq!(search.results.len(), 1);
        assert_eq!(search.results[0].relative_path, "docs/guide.md");
        assert_eq!(app.status, "content search (normal): release (1)");
        let task = app
            .task_log
            .iter()
            .find(|task| task.kind == "search")
            .expect("search task");
        assert_eq!(task.state, TaskState::Done);
    }

    #[test]
    /// 驗證內容搜尋按下 Enter 只會跳到檔案，不會強制切進 preview。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_content_search_enter_reveals_selected_file() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(
            dir.path().join("docs").join("notes.txt"),
            "zero\nmatch one\nmiddle\nmatch two\nend\n",
        )
        .expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE))
            .expect("open content search");
        for ch in ['m', 'a', 't', 'c', 'h'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("run content search");
        wait_for_global_search(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("open search result");

        assert!(app.global_search.is_none());
        assert!(!app.panes.get(&1).expect("pane").is_preview_active());
        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(
            pane.selected_entry().map(|entry| entry.display_name()),
            Some(String::from("notes.txt"))
        );
        assert_eq!(pane.cwd, dir.path().join("docs"));
        assert_eq!(app.status, "search opened: docs/notes.txt");
    }

    #[test]
    /// 驗證內容搜尋按下 Right 也只會跳到檔案，與 Enter / l 行為一致。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_content_search_right_reveals_selected_file() {
        let dir = tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("notes.txt"), "alpha\nbeta\n").expect("notes");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_content_search().expect("open content search");
        for ch in ['b', 'e', 't', 'a'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .expect("type query");
        }

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("run content search");
        wait_for_global_search(&mut app);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .expect("open by right");

        assert!(app.global_search.is_none());
        assert!(!app.panes.get(&1).expect("pane").is_preview_active());
        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(
            pane.selected_entry().map(|entry| entry.display_name()),
            Some(String::from("notes.txt"))
        );
        assert_eq!(pane.cwd, dir.path().join("docs"));
        assert_eq!(app.status, "search opened: docs/notes.txt");
    }

    #[test]
    /// 驗證 task 面板中的 `x` 可以取消目前正在進行的 search task。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_task_panel_x_cancels_running_search_task() {
        let dir = tempdir().expect("tempdir");
        let cancelled = Arc::new(AtomicBool::new(false));

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let task_id = app.push_task(
            1,
            "search",
            String::from("content search: needle"),
            format!("root: {}", dir.path().display()),
            vec![dir.path().display().to_string()],
            None,
        );
        app.global_search = Some(GlobalSearchState {
            pane_id: 1,
            root_dir: dir.path().to_path_buf(),
            mode: SearchMode::Content,
            buffer: String::from("needle"),
            editing: false,
            loading: true,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: Some(task_id),
        });
        app.active_global_search_task_id = Some(task_id);
        app.global_search_cancelled = Some(cancelled.clone());
        app.open_task_panel();

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .expect("cancel task");

        let task = app
            .task_log
            .iter()
            .find(|task| task.id == task_id)
            .expect("task");
        assert_eq!(task.state, TaskState::Cancelled);
        assert!(app.global_search.is_none());
        assert!(app.global_search_rx.is_none());
        assert!(app.global_search_cancelled.is_none());
        assert!(cancelled.load(Ordering::Relaxed));
        assert_eq!(app.status, format!("cancelled task {task_id}"));
    }

    #[test]
    /// 驗證 task 面板中的 `X` 會取消目前 panel 內所有可取消的任務。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_task_panel_shift_x_cancels_all_running_tasks() {
        let dir = tempdir().expect("tempdir");
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let task_id = app.push_task(
            1,
            "search",
            String::from("content search: needle"),
            format!("root: {}", dir.path().display()),
            vec![dir.path().display().to_string()],
            None,
        );
        app.global_search = Some(GlobalSearchState {
            pane_id: 1,
            root_dir: dir.path().to_path_buf(),
            mode: SearchMode::Content,
            buffer: String::from("needle"),
            editing: false,
            loading: true,
            searched: false,
            selected: 0,
            results: Vec::new(),
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: Some(task_id),
        });
        app.active_global_search_task_id = Some(task_id);
        app.global_search_cancelled = Some(cancelled.clone());
        app.open_task_panel();

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
            .expect("cancel all tasks");

        assert_eq!(
            app.task_log
                .iter()
                .find(|task| task.id == task_id)
                .map(|task| task.state),
            Some(TaskState::Cancelled)
        );
        assert!(cancelled.load(Ordering::Relaxed));
        assert_eq!(app.status, "cancelled 1 tasks");
    }

    #[test]
    /// 驗證建立項目後，所有開啟相同目錄的 panel 都會同步看到新項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_mutation_refreshes_sibling_panels_with_same_directory() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        let first_panel = app.ordered_pane_ids()[0];
        let second_panel = app.ordered_pane_ids()[1];

        app.confirm_create_entry(second_panel, "shared.txt")
            .expect("create shared file");

        for pane_id in [first_panel, second_panel] {
            let pane = app.panes.get(&pane_id).expect("pane");
            assert!(
                pane.entries
                    .iter()
                    .any(|entry| entry.display_name() == "shared.txt")
            );
        }
    }

    #[test]
    /// 驗證 Finder／Explorer 在 PaneFM 外部新增或刪除檔案後，所有顯示該目錄的
    /// panel 都會刷新，而且原本游標指向的檔案不會因排序位置改變而跳走。
    /// 保護目的：避免 watcher 只更新 active panel，或 reload 只保留舊索引而選錯檔案。
    fn external_directory_change_refreshes_every_matching_panel_and_keeps_selection() {
        let dir = tempdir().expect("tempdir");
        let original = dir.path().join("middle.txt");
        fs::write(&original, "original").expect("original file");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        for pane in app.panes.values_mut() {
            pane.select_path(&original);
        }

        let external = dir.path().join("ahead.txt");
        fs::write(&external, "created outside PaneFM").expect("external create");
        app.reload_watched_directories(&std::collections::BTreeSet::from([dir
            .path()
            .to_path_buf()]))
            .expect("watcher refresh after create");

        for pane in app.panes.values() {
            assert!(pane.entries.iter().any(|entry| entry.path == external));
            assert_eq!(
                pane.selected_entry().map(|entry| &entry.path),
                Some(&original)
            );
        }

        fs::remove_file(&external).expect("external delete");
        app.reload_watched_directories(&std::collections::BTreeSet::from([dir
            .path()
            .to_path_buf()]))
            .expect("watcher refresh after delete");
        for pane in app.panes.values() {
            assert!(!pane.entries.iter().any(|entry| entry.path == external));
            assert_eq!(
                pane.selected_entry().map(|entry| &entry.path),
                Some(&original)
            );
        }
    }

    #[test]
    /// 驗證在搜尋尚未完成前直接開啟結果，背景 search task 會被正確標記為取消。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_opening_search_result_cancels_running_search_task() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("target.txt"), "target\n").expect("target");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let task_id = app.push_task(
            1,
            "search",
            String::from("content search: target"),
            format!("root: {}", dir.path().display()),
            vec![dir.path().display().to_string()],
            None,
        );
        app.active_global_search_task_id = Some(task_id);

        let search = GlobalSearchState {
            pane_id: 1,
            root_dir: dir.path().to_path_buf(),
            mode: SearchMode::Content,
            buffer: String::from("target"),
            editing: false,
            loading: true,
            searched: true,
            selected: 0,
            results: vec![GlobalSearchEntry {
                path: dir.path().join("target.txt"),
                relative_path: String::from("target.txt"),
                is_dir: false,
                match_line_number: Some(1),
                match_column: Some(1),
                match_preview: Some(String::from("target")),
            }],
            filter: PanelSearchState::default(),
            preview_scroll: None,
            preview_current_match: None,
            task_id: Some(task_id),
        };

        app.open_global_search_result(search)
            .expect("open search result");

        let task = app
            .task_log
            .iter()
            .find(|task| task.id == task_id)
            .expect("task");
        assert_eq!(task.state, TaskState::Cancelled);
        assert_eq!(task.detail, "stopped after opening a result");
        assert!(app.global_search.is_none());
        assert!(app.global_search_rx.is_none());
        assert!(app.active_global_search_task_id.is_none());
        assert!(!app.panes.get(&1).expect("pane").is_preview_active());
        assert_eq!(
            app.panes
                .get(&1)
                .expect("pane")
                .selected_entry()
                .map(|entry| entry.display_name()),
            Some(String::from("target.txt"))
        );
        assert_eq!(app.status, "search opened: target.txt");
    }

    #[test]
    /// 驗證可以用 `V` 視覺標記多個項目，並一次放進剪貼簿。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_visual_marked_entries_copy_into_clipboard_as_batch() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy batch");

        let clipboard = app.clipboard.as_ref().expect("clipboard");
        assert_eq!(clipboard.operation, ClipboardOperation::Copy);
        assert_eq!(clipboard.entries.len(), 2);
        assert_eq!(app.status, "copied 2 items");
    }

    #[test]
    /// 驗證 `V` 視覺標記多個項目後，刪除確認會一次刪掉整批項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_visual_marked_entries_delete_as_batch() {
        let dir = tempdir().expect("tempdir");
        let alpha = dir.path().join("alpha.txt");
        let beta = dir.path().join("beta.txt");
        fs::write(&alpha, "a").expect("alpha");
        fs::write(&beta, "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.start_delete_confirmation(false);

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmDelete { ref target_name, .. }) if target_name == "2 items"
        ));

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("confirm delete batch");

        assert!(!alpha.exists());
        assert!(!beta.exists());
        assert_eq!(app.status, "trashed 2 items");
    }

    #[test]
    /// 驗證 `V` 進入 visual selection 後，移動游標再按一次 `V` 會提交整段標記。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_visual_selection_commits_range_marks() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");
        fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down again");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");

        let pane = app.panes.get(&1).expect("pane");
        assert!(app.visual_selection.is_none());
        assert_eq!(pane.marked_count(), 3);
        assert_eq!(app.status, "marked 3 items");
    }

    #[test]
    /// 驗證小寫 `v` 可以進入、移動並結束 visual selection。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_lowercase_v_controls_visual_selection() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("extend visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE))
            .expect("close visual");

        assert!(app.visual_selection.is_none());
        assert_eq!(app.panes.get(&1).expect("pane").marked_count(), 2);
    }

    #[test]
    /// 驗證某些終端把 `Shift+v` 回報成 `v + Shift` 時，也能正確進入 visual selection。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_v_opens_visual_selection() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SHIFT))
            .expect("open visual with shifted v");

        assert_eq!(
            app.visual_selection,
            Some(VisualSelectionState {
                pane_id: 1,
                anchor: 0,
                current: 0,
            })
        );
        assert_eq!(app.status, "visual: range selection");
    }

    #[test]
    /// 驗證某些終端把 `Shift+g` 回報成 `g + Shift` 時，也能正確執行 `G` 跳到列表底部。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_shift_g_jumps_to_bottom() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");
        fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::SHIFT))
            .expect("jump bottom with shifted g");

        assert_eq!(app.panes.get(&1).expect("pane").selected, 2);
        assert_eq!(app.status, "jumped to bottom");
    }

    #[test]
    /// 驗證 visual selection 按下 `Esc` 會先提交這一段範圍並離開選取模式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_visual_selection_escape_commits_current_range() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");
        fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit first range");

        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move to third");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open second visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE))
            .expect("move back");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("commit second visual");

        let pane = app.panes.get(&1).expect("pane");
        assert!(app.visual_selection.is_none());
        assert_eq!(pane.marked_count(), 3);
        assert_eq!(app.status, "marked 1 items");
    }

    #[test]
    /// 驗證離開選取模式後再按一次 `Esc`，會清掉目前所有已提交標記。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_escape_in_normal_mode_clears_all_marks() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("open visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("commit visual");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear all marks");

        let pane = app.panes.get(&1).expect("pane");
        assert!(app.visual_selection.is_none());
        assert_eq!(pane.marked_count(), 0);
        assert_eq!(app.status, "cleared 2 marks");
    }

    #[test]
    /// 驗證列表模式按下 `/` 後會即時套用 find-next，並可在 Enter 後用 `n/N` 跳轉命中項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_list_find_supports_lock_and_navigation() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("alps.txt"), "b").expect("alps");
        fs::write(dir.path().join("beta.txt"), "c").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("open list find");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type a");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("type l");

        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.selected_entry().expect("selected").name, "alpha.txt");
        assert_eq!(pane.list_find_match_indices(), vec![0, 1]);
        assert_eq!(app.status, "find next: al (2)");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock list find");
        assert!(app.list_find.is_none());
        assert_eq!(app.status, "find next locked: al (2)");

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("next match");
        assert_eq!(
            app.panes
                .get(&1)
                .expect("pane")
                .selected_entry()
                .expect("selected")
                .name,
            "alps.txt"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::SHIFT))
            .expect("previous match");
        assert_eq!(
            app.panes
                .get(&1)
                .expect("pane")
                .selected_entry()
                .expect("selected")
                .name,
            "alpha.txt"
        );
    }

    #[test]
    /// 驗證一般貼上遇到同名檔案時，會先開啟覆蓋確認視窗，使用者確認後才覆蓋。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_paste_with_conflict_requires_confirmation_before_overwrite() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source");
        fs::create_dir(&target_dir).expect("target");
        let source_file = source_dir.join("alpha.txt");
        let target_file = target_dir.join("alpha.txt");
        fs::write(&source_file, "from source").expect("source file");
        fs::write(&target_file, "from target").expect("target file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy");

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload target");

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("open overwrite confirm");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::ConfirmPasteOverwrite { .. })
        ));
        assert_eq!(
            fs::read_to_string(&target_file).expect("target content before confirm"),
            "from target"
        );

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("confirm overwrite");

        assert_eq!(
            fs::read_to_string(&target_file).expect("target content after confirm"),
            "from source"
        );
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "pasted copy with overwrite: 1 item");
    }

    #[test]
    /// 驗證一般貼上遇到同名檔案時，若使用者取消，會保留原檔案且不執行貼上。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_paste_with_conflict_can_be_cancelled() {
        let dir = tempdir().expect("tempdir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        fs::create_dir(&source_dir).expect("source");
        fs::create_dir(&target_dir).expect("target");
        let source_file = source_dir.join("alpha.txt");
        let target_file = target_dir.join("alpha.txt");
        fs::write(&source_file, "from source").expect("source file");
        fs::write(&target_file, "from target").expect("target file");

        let mut app = App::new(source_dir.clone(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .expect("copy");

        app.current_pane_mut().expect("pane").cwd = target_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload target");

        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE))
            .expect("open overwrite confirm");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel overwrite");

        assert_eq!(
            fs::read_to_string(&target_file).expect("target content after cancel"),
            "from target"
        );
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "paste cancelled: alpha.txt");
    }

    #[test]
    /// 驗證列表模式的 find-next 在鎖定後按下 `Esc`，會清除目前 pane 的高亮結果。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_list_find_escape_clears_active_query() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("open list find");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock list find");
        assert!(app.panes.get(&1).expect("pane").list_find_query().is_some());

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear list find");
        assert!(app.panes.get(&1).expect("pane").list_find_query().is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證重新按下 `/` 打開 list find 時，不會沿用上一輪輸入的查詢文字。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_reopening_list_find_starts_with_empty_buffer() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("open list find");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type query");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock query");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("clear query");

        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("reopen list find");

        assert_eq!(
            app.list_find,
            Some(ListFindState {
                pane_id: 1,
                buffer: String::new(),
            })
        );
        assert!(app.panes.get(&1).expect("pane").list_find_query().is_none());
        assert_eq!(app.status, "find next: type query");
    }

    #[test]
    /// 驗證 normal mode 支援像 Vim 一樣用數字前綴配合 `j` 一次移動多格。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_count_prefix_moves_list_cursor_by_multiple_rows() {
        let dir = tempdir().expect("tempdir");
        for index in 0..8 {
            fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
        }

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE))
            .expect("count");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");

        assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
        assert!(app.pending_count.is_none());
    }

    #[test]
    /// 驗證 count prefix 可以搭配 `gg` 與 `G` 跳到指定列表位置。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_count_prefix_supports_absolute_jumps() {
        let dir = tempdir().expect("tempdir");
        for index in 0..8 {
            fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
        }

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE))
            .expect("count for gg");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("first g");
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE))
            .expect("second g");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 4);

        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
            .expect("count for G");
        app.handle_key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE))
            .expect("shift g");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 1);
    }

    #[test]
    /// 驗證 count prefix 可以搭配 list find 的 `n` 一次跳過多個命中結果。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_count_prefix_supports_list_find_navigation() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("alps.txt"), "b").expect("alps");
        fs::write(dir.path().join("algae.txt"), "c").expect("algae");
        fs::write(dir.path().join("beta.txt"), "d").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("open list find");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type a");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("type l");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("lock find");
        assert_eq!(
            app.panes
                .get(&1)
                .expect("pane")
                .selected_entry()
                .expect("selected")
                .name,
            "algae.txt"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
            .expect("count");
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("jump matches");

        assert_eq!(
            app.panes
                .get(&1)
                .expect("pane")
                .selected_entry()
                .expect("selected")
                .name,
            "alps.txt"
        );
        assert!(app.pending_count.is_none());
    }

    #[test]
    /// 驗證按下 `z` 後會建立 `fzf` 跳轉請求，並記住目前 pane 的根目錄設定。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_jump_key_queues_fzf_request() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::create_dir(dir.path().join("docs")).expect("docs");
        fs::write(dir.path().join("docs").join("readme.md"), "b").expect("readme");
        fs::write(dir.path().join("report.txt"), "c").expect("report");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
            .expect("open jump");

        let request = app.take_pending_fzf_jump().expect("fzf request");

        assert_eq!(request.pane_id, 1);
        assert_eq!(request.root_dir, dir.path());
        assert!(request.show_hidden);
        assert!(request.follow_links);
        assert!(app.pending_fzf_jump.is_none());
        assert_eq!(app.status, "jump: fzf loading");
    }

    #[test]
    /// 驗證分割成多個 pane 後，在目前 focus 的 pane 按下 `z` 仍會建立 `fzf` 請求。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_jump_key_works_from_focused_split_pane() {
        let dir = tempdir().expect("tempdir");
        let left_dir = dir.path().join("left");
        let right_dir = dir.path().join("right");
        fs::create_dir(&left_dir).expect("left");
        fs::create_dir(&right_dir).expect("right");
        fs::write(left_dir.join("alpha.txt"), "a").expect("alpha");
        fs::write(right_dir.join("beta.txt"), "b").expect("beta");

        let mut app = App::new(left_dir.clone(), default_loaded_config()).expect("app");
        app.split_current(SplitDirection::Vertical).expect("split");
        app.current_pane_mut().expect("pane").cwd = right_dir.clone();
        app.current_pane_mut()
            .expect("pane")
            .reload()
            .expect("reload");

        assert_eq!(app.focused_pane, 2);
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
            .expect("open jump");

        let request = app.take_pending_fzf_jump().expect("fzf request");

        assert_eq!(request.pane_id, 2);
        assert_eq!(request.root_dir, right_dir);
        assert!(request.show_hidden);
        assert!(request.follow_links);
        assert_eq!(app.status, "jump: fzf loading");
    }

    #[test]
    /// 驗證 `z` 使用的 `fzf` 搜尋會固定包含 hidden 內容，不受 pane 顯示設定影響。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_jump_key_always_searches_hidden_entries() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join(".secret.txt"), "secret").expect("secret");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.current_pane_mut().expect("pane").show_hidden = false;
        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE))
            .expect("open jump");

        let request = app.take_pending_fzf_jump().expect("fzf request");
        assert!(request.show_hidden);
        assert!(request.follow_links);
    }

    #[test]
    /// 驗證套用 `fzf` 選取結果後，游標會跳到對應的檔案。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_apply_fzf_jump_selection_moves_cursor_to_match() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("readme.md"), "b").expect("readme");
        fs::write(dir.path().join("report.txt"), "c").expect("report");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_fzf_jump();
        let request = app.take_pending_fzf_jump().expect("fzf request");
        app.apply_fzf_jump_selection(request, Some("report.txt"));

        assert_eq!(
            app.panes
                .get(&1)
                .expect("pane")
                .selected_entry()
                .expect("selected")
                .name,
            "report.txt"
        );
        assert_eq!(app.status, "jumped: report.txt");
    }

    #[test]
    /// 驗證套用巢狀 `fzf` 結果後，pane 會切到檔案所在目錄並聚焦正確項目。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_apply_fzf_jump_selection_reveals_nested_file() {
        let dir = tempdir().expect("tempdir");
        let nested_dir = dir.path().join("docs");
        fs::create_dir(&nested_dir).expect("docs");
        fs::write(nested_dir.join("guide.md"), "guide").expect("guide");
        fs::write(dir.path().join("root.txt"), "root").expect("root");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_fzf_jump();
        let request = app.take_pending_fzf_jump().expect("fzf request");
        app.apply_fzf_jump_selection(request, Some("docs/guide.md"));

        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.cwd, nested_dir);
        assert_eq!(pane.selected_entry().expect("selected").name, "guide.md");
        assert_eq!(app.status, "jumped: docs/guide.md");
    }

    #[test]
    /// 驗證取消 `fzf` 選擇時，不會改動目前游標位置。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_apply_fzf_jump_selection_cancel_keeps_selection() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("readme.md"), "b").expect("readme");
        fs::write(dir.path().join("report.txt"), "c").expect("report");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move to readme");
        let original = app.panes.get(&1).expect("pane").selected;

        app.open_fzf_jump();
        let request = app.take_pending_fzf_jump().expect("fzf request");
        app.apply_fzf_jump_selection(request, None);

        assert_eq!(app.panes.get(&1).expect("pane").selected, original);
        assert_eq!(app.status, "jump cancelled");
    }

    #[test]
    /// 驗證 normal mode 按下 `Ctrl-a` 會把目前 pane 的所有可見項目全部標記起來。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_a_marks_all_visible_entries() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");
        fs::write(dir.path().join("gamma.txt"), "c").expect("gamma");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .expect("mark all");

        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.marked_count(), 3);
        assert_eq!(app.status, "marked all visible items (+3, total 3)");
    }

    #[test]
    /// 驗證 `:mark-all` 命令也能把目前 pane 的所有可見項目全部標記起來。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_mark_all_command_marks_all_visible_entries() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("mark-all").expect("mark-all command");

        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.marked_count(), 2);
        assert_eq!(app.status, "marked all visible items (+2, total 2)");
    }

    #[test]
    /// 驗證 normal mode 按下 `c` 會打開文字複製小視窗。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_c_key_opens_copy_picker() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .expect("open copy picker");

        match app.pending_action {
            Some(PendingAction::CopyPicker {
                pane_id, selected, ..
            }) => {
                assert_eq!(pane_id, 1);
                assert_eq!(selected, 0);
            }
            other => panic!("expected copy picker, got {other:?}"),
        }
        assert_eq!(app.status, "copy to clipboard: alpha.txt");
    }

    #[test]
    /// 驗證文字複製小視窗按下 `h` 會關閉並回到一般模式。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_copy_picker_h_closes_panel() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_copy_picker().expect("open copy picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("close copy picker");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證文字複製小視窗打開後，再按一次 `c` 會直接關閉。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_copy_picker_c_toggles_closed() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_copy_picker().expect("open copy picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .expect("toggle close copy picker");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證文字複製小視窗中，原本的檔案路徑複製已改成 `u`，避免和 opener `c` 衝突。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_copy_picker_u_copies_file_path() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("alpha.txt");
        fs::write(&file_path, "a").expect("alpha");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.open_copy_picker().expect("open copy picker");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE))
            .expect("copy file path");

        assert_eq!(app.status, "copied file path: alpha.txt");
    }

    #[test]
    /// 驗證 normal mode 按下 `Ctrl-Shift-A` 會清掉目前 pane 的所有標記。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_shift_a_clears_all_marks_in_focused_pane() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "a").expect("alpha");
        fs::write(dir.path().join("beta.txt"), "b").expect("beta");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.execute_command("mark-all").expect("mark-all command");
        app.handle_key(KeyEvent::new(
            KeyCode::Char('A'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
        .expect("clear marks");

        let pane = app.panes.get(&1).expect("pane");
        assert_eq!(pane.marked_count(), 0);
        assert_eq!(app.status, "cleared 2 marks");
    }

    #[test]
    /// 驗證 normal mode 的 `Ctrl-d / Ctrl-u` 會依照目前列表 viewport 高度做半頁移動。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_d_and_ctrl_u_move_by_half_page() {
        let dir = tempdir().expect("tempdir");
        for index in 0..10 {
            fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
        }

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_list_viewport_height(6);

        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("page down");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 3);
        assert_eq!(app.status, "half page down: 3");

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .expect("page up");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
        assert_eq!(app.status, "half page up: 3");
    }

    #[test]
    /// 驗證 normal mode 的 `Ctrl-f / Ctrl-b` 會依照目前列表 viewport 高度做整頁移動。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_ctrl_f_and_ctrl_b_move_by_full_page() {
        let dir = tempdir().expect("tempdir");
        for index in 0..12 {
            fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
        }

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_list_viewport_height(5);

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL))
            .expect("full page down");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 5);
        assert_eq!(app.status, "page down: 5");

        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL))
            .expect("full page up");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
        assert_eq!(app.status, "page up: 5");
    }

    #[test]
    /// 驗證 visual selection 中的 `Ctrl-d / Ctrl-u` 也會用半頁步長移動，並同步更新範圍。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn app_visual_selection_ctrl_d_and_ctrl_u_follow_half_page() {
        let dir = tempdir().expect("tempdir");
        for index in 0..10 {
            fs::write(dir.path().join(format!("file-{index}.txt")), "x").expect("file");
        }

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.panes
            .get_mut(&1)
            .expect("pane")
            .set_list_viewport_height(6);

        app.handle_key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::NONE))
            .expect("visual");
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL))
            .expect("visual page down");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 3);
        assert_eq!(
            app.visual_selection,
            Some(VisualSelectionState {
                pane_id: 1,
                anchor: 0,
                current: 3,
            })
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
            .expect("visual page up");
        assert_eq!(app.panes.get(&1).expect("pane").selected, 0);
        assert_eq!(
            app.visual_selection,
            Some(VisualSelectionState {
                pane_id: 1,
                anchor: 0,
                current: 0,
            })
        );
    }

    #[test]
    /// 驗證 trash 確認視窗會記住原本所屬的 panel，讓 UI 能畫回同一個列表內。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn trash_confirm_panel_id_returns_source_panel() {
        let action = TrashConfirmAction::DeleteFromPanel {
            pane_id: 7,
            target_ids: vec![String::from("trash-id")],
            search: PanelSearchState {
                buffer: String::from("demo"),
                editing: false,
            },
            selected: 2,
        };

        assert_eq!(trash_confirm_panel_id(&action), Some(7));
    }

    #[test]
    /// 驗證 trash 確認視窗也能還原底層列表需要的搜尋與標記狀態。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn trash_confirm_overlay_state_preserves_trash_context() {
        let pending = PendingAction::ConfirmTrashAction {
            action: TrashConfirmAction::RestoreFromPanel {
                pane_id: 3,
                target_ids: vec![String::from("trash-id")],
                search: PanelSearchState {
                    buffer: String::from("abc"),
                    editing: false,
                },
                selected: 2,
            },
            target_name: String::from("alpha.txt"),
            entry_count: 1,
            marked_ids: vec![String::from("trash-id"), String::from("trash-id-2")],
            visual_anchor: Some(1),
        };

        let (selected, search, marked_ids, visual_anchor) =
            trash_panel_overlay_state_from_pending_action(&Some(pending), 3)
                .expect("overlay state");

        assert_eq!(selected, 2);
        assert_eq!(search.buffer, "abc");
        assert!(!search.editing);
        assert_eq!(marked_ids.len(), 2);
        assert_eq!(visual_anchor, Some(1));
    }

    #[test]
    /// 驗證 regex rename 使用的 command UI 可以切到 Normal 模式移動游標，再回到 Insert 修正中間文字。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn command_input_supports_vim_normal_and_insert_modes() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_prefilled_command("rename-regex foo baz");
        app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("enter normal mode");
        assert!(app.command_mode);
        assert_eq!(app.text_input_mode, RenameMode::Normal);
        assert_eq!(app.rename_cursor_mode(), Some(RenameMode::Normal));

        for _ in 0..2 {
            app.handle_command_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
                .expect("move left");
        }
        app.handle_command_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
            .expect("enter insert mode");
        app.handle_command_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
            .expect("insert correction");

        assert_eq!(app.command_buffer, "rename-regex foo Xbaz");
        assert_eq!(app.text_input_mode, RenameMode::Insert);
    }

    #[test]
    /// 驗證一般 filter 第一次 Esc 只切換模式，Normal 模式第二次 Esc 才鎖定並離開輸入框。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn filter_input_uses_two_stage_escape_and_supports_cursor_editing() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("aXbc.txt"), "demo").expect("file");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_filter_input(FilterMode::Normal);
        for character in ['a', 'b', 'c'] {
            app.handle_filter_input_key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            ))
            .expect("type filter");
        }
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("normal mode");
        assert!(app.filter.as_ref().is_some_and(|filter| filter.editing));

        app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("move left");
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
            .expect("insert mode");
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::SHIFT))
            .expect("insert middle");
        assert_eq!(app.filter.as_ref().expect("filter").buffer, "aXbc");

        app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("normal mode again");
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("leave editor");
        assert!(!app.filter.as_ref().expect("filter").editing);
    }

    #[test]
    /// 驗證 help、trash、task、bookmark 與 zoxide 共用的面板搜尋器會攔截 Normal 模式按鍵，不會誤關面板。
    /// 保護目的：避免快捷鍵、模式或狀態分派重構後，破壞上述使用者可觀察的操作流程。
    fn panel_search_uses_shared_vim_editor_before_panel_actions() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_help_panel();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open help filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE))
            .expect("type filter");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("normal mode");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("move cursor instead of closing panel");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel {
                search: PanelSearchState { editing: true, .. },
                ..
            })
        ));
        assert_eq!(app.text_input_mode, RenameMode::Normal);

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close panel search");
        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel {
                search: PanelSearchState { editing: false, .. },
                ..
            })
        ));
    }

    #[test]
    /// 驗證空白 command UI 在 Insert 模式按第一次 Esc 就會直接關閉。
    /// 保護目的：空輸入沒有文字需要進入 Vim Normal 模式修正，不應要求使用者連按兩次。
    fn empty_command_input_closes_on_first_escape() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_prefilled_command("");
        app.handle_command_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close empty command input");

        assert!(!app.command_mode);
        assert!(app.command_buffer.is_empty());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證剛開啟且尚未輸入內容的 filter，第一次 Esc 會完整清除 filter 狀態。
    /// 保護目的：避免輸入框雖消失，內部卻殘留一個不可見的空 filter，造成後續 Esc 流程混亂。
    fn empty_filter_input_closes_on_first_escape() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_filter_input(FilterMode::Normal);
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close empty filter input");

        assert!(app.filter.is_none());
        assert!(!app.panes.get(&1).expect("pane").has_active_filter());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證空白 Preview Search 在第一次 Esc 就會關閉，並清除 pane 上的搜尋條件。
    /// 保護目的：所有共用文字輸入 UI 都必須遵守相同的空輸入快速離開規則。
    fn empty_preview_search_closes_on_first_escape() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "alpha").expect("file");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_preview_focus();
        app.open_preview_search_input();
        app.handle_preview_search_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close empty preview search");

        assert!(app.preview_search.is_none());
        assert_eq!(app.status, "preview mode");
    }

    #[test]
    /// 驗證建立檔案的 inline 輸入框尚未輸入名稱時，第一次 Esc 就會取消建立。
    /// 保護目的：rename/create 使用獨立編輯流程，也必須與共用輸入器保持一致。
    fn empty_create_input_closes_on_first_escape() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.start_create_entry();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel empty create input");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "create cancelled");
    }

    #[test]
    /// 驗證空白 rename 輸入框在 Insert 模式按第一次 Esc 就會取消改名。
    /// 保護目的：rename 使用獨立編輯流程，清空原檔名後也不可殘留在無意義的 Normal 模式。
    fn empty_rename_input_closes_on_first_escape() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.pending_action = Some(PendingAction::Rename {
            pane_id: 1,
            original_name: String::from("alpha.txt"),
            buffer: String::new(),
            cursor: 0,
            mode: RenameMode::Insert,
        });

        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("cancel empty rename input");

        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "rename cancelled: alpha.txt");
    }

    #[test]
    /// 驗證空白 list find 與 global search 都能用第一次 Esc 直接回到一般列表。
    /// 保護目的：兩種搜尋使用不同外層狀態機，但都必須遵守共用輸入器的快速離開規則。
    fn empty_list_and_global_search_close_on_first_escape() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("alpha.txt"), "demo").expect("file");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_list_find_input();
        app.handle_list_find_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close empty list find");
        assert!(app.list_find.is_none());

        app.open_global_search().expect("open global search");
        app.handle_global_search_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close empty global search");
        assert!(app.global_search.is_none());
        assert_eq!(app.status, "normal mode");
    }

    #[test]
    /// 驗證列表面板內剛開啟的空白搜尋框，第一次 Esc 就會收起輸入框。
    /// 保護目的：Help、Trash、Task、Bookmark、Zoxide 共用此流程，修正一次即可保持一致。
    fn empty_panel_search_closes_on_first_escape() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.open_help_panel();
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("open panel search");
        app.handle_pending_action_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("close empty panel search");

        assert!(matches!(
            app.pending_action,
            Some(PendingAction::HelpPanel {
                search: PanelSearchState {
                    editing: false,
                    ref buffer,
                },
                ..
            }) if buffer.is_empty()
        ));
    }

    #[test]
    /// 驗證在目錄背景載入尚未完成時按 h 離開，會即時取消舊工作且不把空列表寫入快取。
    /// 保護目的：確保使用者在大型目錄快速切出時不卡死，且不會把未完成的空清單當作快取。
    fn navigating_away_during_load_cancels_load_and_guards_cache() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        fs::create_dir_all(&child).expect("child dir");
        fs::write(child.join("payload.txt"), "data").expect("payload");

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        // 進入 child 目錄，此時啟動背景載入
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("enter child");
        assert_eq!(app.panes[&1].cwd, child);
        assert!(app.directory_load_jobs.contains_key(&1));

        // 尚未 poll 完成前立刻按 h 離開回到 parent
        app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
            .expect("return to parent");
        assert_eq!(app.panes[&1].cwd, dir.path());

        // 快取中不能存入 child 的空清單
        assert_ne!(
            app.directory_entry_cache.get(&child),
            Some(&Vec::new()),
            "載入中離開目錄不得將空陣列存入快取"
        );
    }

    #[test]
    /// 驗證建立新檔案與刪除檔案後，快取會同步更新，重新進入目錄不會讀到陳舊快取。
    /// 保護目的：防止使用者在目錄操作後離開再進入時，出現已刪除檔案回魂或新檔案消失的現象。
    fn file_creation_and_deletion_updates_cache_synchronously() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        // 建立檔案
        app.create_entry_from_command("new_item.txt")
            .expect("create file");

        let cached = app
            .directory_entry_cache
            .get(dir.path())
            .expect("must have cache");
        assert!(
            cached.iter().any(|entry| entry.name == "new_item.txt"),
            "快取必須包含剛建立的檔案"
        );

        // 刪除檔案
        app.start_delete_confirmation(true);
        app.confirm_delete(1, "new_item.txt", true)
            .expect("delete file");
        for _ in 0..100 {
            app.poll_background_tasks();
            if app.file_job_receivers.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let cached_after = app
            .directory_entry_cache
            .get(dir.path())
            .expect("must have cache after delete");
        assert!(
            !cached_after
                .iter()
                .any(|entry| entry.name == "new_item.txt"),
            "快取中不能殘留已刪除的檔案"
        );
    }

    #[test]
    /// 驗證檔案系統 watcher 觸發重新整理時，快取會同步更新為磁碟最新狀態。
    /// 保護目的：確保外部編輯器或 Git 操作產生變更後，快取與畫面保持一致。
    fn watcher_reload_synchronizes_cache() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        fs::write(dir.path().join("external.txt"), "external content").expect("external write");
        let mut set = BTreeSet::new();
        set.insert(dir.path().to_path_buf());

        app.reload_watched_directories(&set)
            .expect("reload watched");

        let cached = app
            .directory_entry_cache
            .get(dir.path())
            .expect("must have cache");
        assert!(
            cached.iter().any(|entry| entry.name == "external.txt"),
            "watcher 刷新後快取必須包含外部新增的檔案"
        );
    }

    #[test]
    /// 驗證大型目錄以串流載入時，首批清單到達後游標即可立刻以 j/k 移動，不需等待全量掃描結束。
    /// 保護目的：確保使用者在幾萬個檔案的目錄中，畫面在毫秒級反應，游標絕不被背景 I/O 凍結。
    fn streaming_directory_load_allows_cursor_movement_before_completion() {
        let dir = tempdir().expect("tempdir");
        let child = dir.path().join("child");
        fs::create_dir_all(&child).expect("child");
        for i in 0..300 {
            fs::write(child.join(format!("file_{i:03}.txt")), b"test").expect("write");
        }

        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("enter child");

        // 等待首批快速 chunk 到達
        for _ in 0..100 {
            app.poll_background_tasks();
            if !app.panes[&1].entries.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        // 首批項目已在畫面上，游標在第 0 筆
        assert!(!app.panes[&1].entries.is_empty());
        assert_eq!(app.panes[&1].selected, 0);

        // 立刻按下 j 移動游標，游標必須成功移動到第 1 筆
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("move down");
        assert_eq!(app.panes[&1].selected, 1, "游標必須在載入中立即響應移動");

        // 等待背景全量載入結束
        for _ in 0..100 {
            app.poll_background_tasks();
            if app.directory_load_jobs.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(app.panes[&1].entries.len(), 300);
        assert_eq!(
            app.panes[&1].selected, 1,
            "全量完成後仍必須保留使用者剛剛移動的游標位置"
        );
    }

    #[test]
    /// 驗證大型目錄的 task badge 查詢只接觸 viewport，不會走訪完整檔案列表。
    ///
    /// 保護目的：舊版 render 會對每個項目執行 `canonicalize()`；實際 `deps` 超過六萬筆
    /// 時，每次 j/k 都被同步檔案 I/O 阻塞約半秒。此測試建立 200 筆並固定 viewport
    /// 為 18 列，確保未來重構不能再次把完整列表送進 badge 查詢。
    fn task_badge_paths_are_limited_to_visible_viewport() {
        let dir = tempdir().expect("tempdir");
        for index in 0..200 {
            fs::write(dir.path().join(format!("file_{index:03}.txt")), b"x").expect("write");
        }
        let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
        pane.selected = 150;

        let paths = visible_job_badge_paths(&pane, 18);

        assert_eq!(paths.len(), 18);
        assert!(paths.iter().all(|path| path.starts_with(dir.path())));
    }

    #[test]
    /// 驗證當背景有檔案傳輸或寫入進行時，使用者不可進入該正在寫入的目錄。
    fn cannot_enter_directory_while_transfer_is_in_progress() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        let child = app.panes[&1].cwd.join("copying_target");
        fs::create_dir(&child).expect("create child");
        app.panes.get_mut(&1).unwrap().reload().expect("reload");

        // 模擬一個正在進行中的背景傳輸工作綁定到 child
        let task_id = app.push_task(
            1,
            "paste",
            "copy files".to_string(),
            "dest".to_string(),
            vec![child.display().to_string()],
            Some(dir.path().display().to_string()),
        );
        app.active_file_job_busy_paths
            .insert(task_id, vec![child.clone()]);
        app.update_task_progress(task_id, 45, 100);

        // 選中該目錄並嘗試進入
        app.panes.get_mut(&1).unwrap().select_path(&child);
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("press l");

        // 工作目錄不可被切換，且狀態列必須提示傳輸進行中
        assert_eq!(app.panes[&1].cwd, child.parent().unwrap());
        assert!(app.status.contains("cannot enter 'copying_target/'"));
        assert!(app.status.contains("transfer in progress"));
        assert!(app.status.contains("45%"));
    }

    #[test]
    /// 驗證永久刪除大型目錄時使用背景 worker 執行，主執行緒不卡死且完成後正確移除。
    fn background_delete_removes_directory_and_reports_done() {
        let dir = tempdir().expect("tempdir");
        let to_delete = dir.path().join("large_dir");
        fs::create_dir_all(to_delete.join("nested")).expect("create nested");
        fs::write(to_delete.join("nested/a.bin"), "payload").expect("write payload");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.panes.get_mut(&1).unwrap().select_path(&to_delete);
        app.start_delete_confirmation(true);
        app.confirm_delete(1, "large_dir", true)
            .expect("confirm delete");

        assert!(
            !app.file_job_receivers.is_empty(),
            "刪除工作必須進入背景佇列"
        );
        for _ in 0..100 {
            app.poll_background_tasks();
            if app.file_job_receivers.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        assert!(!to_delete.exists(), "背景刪除完成後目錄必須已從磁碟移除");
        assert!(app.status.contains("deleted permanently"));
        let task = app.task_log.last().expect("background delete task");
        assert_eq!(task.title, "delete 1 item(s)");
        assert_eq!(task.source_locations, vec![to_delete.display().to_string()]);
        assert_eq!(task.destination_location, None);
    }

    #[test]
    /// 驗證當子目錄或檔案在進行背景傳輸時，其父目錄依然可以正常進入，且進入後該子項目會顯示進度標籤。
    fn parent_directory_is_enterable_while_child_is_being_transferred() {
        let dir = tempdir().expect("tempdir");
        let parent = dir.path().join("AB_Demo");
        let child = parent.join("terminal-file-manager");
        fs::create_dir_all(&child).expect("create child dir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        // 模擬背景傳輸工作鎖定子項目 child
        let task_id = app.push_task(
            1,
            "paste",
            "copy 1 item(s)".to_string(),
            "dest".to_string(),
            vec![child.display().to_string()],
            Some(parent.display().to_string()),
        );
        app.active_file_job_busy_paths
            .insert(task_id, vec![child.clone()]);
        app.update_task_progress(task_id, 99, 100);

        // 父目錄 parent 必須不受影響，不可被視為 busy
        assert!(
            app.active_file_job_for_path(&parent).is_none(),
            "父目錄不可被子項目的傳輸鎖定"
        );
        // 子項目 child 必須被鎖定並能提供 progress badge
        assert!(app.active_file_job_for_path(&child).is_some());
        assert_eq!(
            app.active_job_badge_for_path(&child).as_deref(),
            Some("[copying 99%]")
        );

        // 嘗試進入父目錄 parent
        app.panes.get_mut(&1).unwrap().select_path(&parent);
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE))
            .expect("enter parent dir");

        // 必須成功進入父目錄
        assert_eq!(
            app.panes[&1].cwd.canonicalize().unwrap(),
            parent.canonicalize().unwrap()
        );
    }

    #[test]
    /// 驗證背景刪除任務在執行期間與完成後，Task 紀錄會包含實際刪除的 byte 進度資訊，而非未知的 `-`。
    fn background_delete_reports_byte_progress_in_task_record() {
        let dir = tempdir().expect("tempdir");
        let to_delete = dir.path().join("data_folder");
        fs::create_dir_all(&to_delete).expect("create dir");
        fs::write(to_delete.join("file1.dat"), vec![0u8; 1024 * 10]).expect("write file1");
        fs::write(to_delete.join("file2.dat"), vec![0u8; 1024 * 20]).expect("write file2");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        app.panes.get_mut(&1).unwrap().select_path(&to_delete);
        app.start_delete_confirmation(true);
        app.confirm_delete(1, "data_folder", true)
            .expect("confirm delete");

        // 輪詢等待背景刪除工作完成
        for _ in 0..100 {
            app.poll_background_tasks();
            if app.file_job_receivers.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }

        let task = app.task_log.last().expect("task record");
        assert_eq!(task.state, TaskState::Done);
        assert!(
            task.completed_bytes.unwrap_or(0) >= 1024 * 30,
            "必須記錄實際刪除的 byte 數: {:?}",
            task.completed_bytes
        );
        assert_eq!(task.completed_bytes, task.total_bytes);
        let progress_label = task_progress_label(task);
        assert_ne!(progress_label, "-", "進度標籤不可為未知的 `-`");
        assert!(
            progress_label.contains("30K")
                || progress_label.contains("K")
                || progress_label.contains("M")
        );
    }

    #[test]
    /// 驗證按下 `f` 開啟一般過濾、按下 `F` 開啟模糊搜尋過濾，且在過濾輸入框內按 `Tab` 可無縫切換模式。
    fn filter_f_and_shift_f_open_normal_and_fuzzy_modes_and_tab_toggles() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("libpanefm-6510b5220d8becac.rlib"), "bin").expect("write1");
        fs::write(dir.path().join("terminal_file_manager.d"), "bin").expect("write2");
        fs::write(dir.path().join("other_file.txt"), "bin").expect("write3");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");

        // 1. 按下 'f' 開啟一般過濾模式
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("press f");
        assert!(app.filter.is_some());
        assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Normal);
        assert!(app.status.contains("filter [normal]"));

        // 輸入 "pnefm"（非連續子字串，一般模式下不會命中）
        for c in ['p', 'n', 'e', 'f', 'm'] {
            app.handle_filter_input_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
                .expect("type char");
        }
        assert_eq!(app.panes[&1].visible_indices.len(), 0, "一般模式連續子字串比對不應命中");

        // 2. 按下 Tab 切換為模糊過濾模式（Fuzzy）
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("press tab");
        assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Fuzzy);
        assert!(app.status.contains("filter [fuzzy]"));
        assert_eq!(app.panes[&1].visible_indices.len(), 1, "模糊搜尋應命中 libpanefm-*.rlib");
        assert_eq!(
            app.panes[&1].entries[app.panes[&1].visible_indices[0]].name,
            "libpanefm-6510b5220d8becac.rlib"
        );

        // 3. 再次按 Tab 切回一般模式
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .expect("press tab back");
        assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Normal);
        assert_eq!(app.panes[&1].visible_indices.len(), 0);

        // 關閉當前 filter
        app.handle_filter_input_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .expect("esc");
        app.panes.get_mut(&1).unwrap().clear_filter();
        app.filter = None;

        // 4. 按下 'F' (Shift+F) 直接開啟模糊搜尋過濾模式
        app.handle_key(KeyEvent::new(KeyCode::Char('F'), KeyModifiers::SHIFT))
            .expect("press Shift+F");
        assert!(app.filter.is_some());
        assert_eq!(app.filter.as_ref().unwrap().mode, FilterMode::Fuzzy);
        assert!(app.status.contains("filter [fuzzy]"));

        // 輸入 "tfm" 模糊搜尋
        for c in ['t', 'f', 'm'] {
            app.handle_filter_input_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
                .expect("type char");
        }
        assert!(
            app.panes[&1].visible_indices.iter().any(|idx| {
                app.panes[&1].entries[*idx].name == "terminal_file_manager.d"
            }),
            "模糊搜尋 'tfm' 必須命中 terminal_file_manager.d"
        );
    }

    #[test]
    fn diff_command_requires_two_panels() {
        let dir = tempdir().expect("tempdir");
        let mut app = App::new(dir.path().to_path_buf(), default_loaded_config()).expect("app");
        // 只有 1 個 panel 時執行 :diff
        app.execute_command("diff").expect("diff cmd");
        assert!(app.status.contains("diff requires at least 2 open panels"));
        assert!(app.pending_action.is_none());
    }

    #[test]
    fn diff_command_opens_and_navigates_matrix() {
        let dir1 = tempdir().expect("dir1");
        let dir2 = tempdir().expect("dir2");
        let dir3 = tempdir().expect("dir3");

        fs::write(dir1.path().join("a.txt"), b"aaa").expect("write a");
        fs::write(dir2.path().join("a.txt"), b"aaa").expect("write a");
        fs::write(dir3.path().join("a.txt"), b"different").expect("write a");

        fs::write(dir1.path().join("only1.txt"), b"111").expect("write 1");
        fs::write(dir2.path().join("only2.txt"), b"222").expect("write 2");

        let mut app = App::new(dir1.path().to_path_buf(), default_loaded_config()).expect("app");
        // 分割出 panel 2 與 panel 3
        app.split_current(SplitDirection::Vertical).expect("split 1");
        app.change_directory_from_command(&dir2.path().to_string_lossy()).expect("goto dir2");

        app.split_current(SplitDirection::Vertical).expect("split 2");
        app.change_directory_from_command(&dir3.path().to_string_lossy()).expect("goto dir3");

        // 執行 :diff 比對全部 3 個 Panel
        app.execute_command("diff").expect("execute diff");
        assert!(matches!(app.pending_action, Some(PendingAction::DiffMatrix(_))));

        // 輪詢等待背景 diff 完成
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.poll_background_tasks();
            if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
                if !state.loading {
                    break;
                }
            }
        }

        // 測試按鍵導航
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))
            .expect("down");
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            assert_eq!(state.selected_index, 1);
        }

        // 測試篩選切換 (f)
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE))
            .expect("filter cycle");
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            assert_eq!(state.filter_mode, crate::file_manager::diff::DiffFilterMode::DiffOnly);
        }

        // 測試 gitignore 切換 (i)
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE))
            .expect("toggle gitignore");
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            assert!(!state.git_ignore);
        }
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.poll_background_tasks();
            if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
                if !state.loading {
                    break;
                }
            }
        }

        // 測試隱藏檔切換 (.)
        app.handle_key(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE))
            .expect("toggle hidden");
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            assert!(!state.include_hidden);
        }
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.poll_background_tasks();
            if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
                if !state.loading {
                    break;
                }
            }
        }

        // 測試搜尋 (/)
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .expect("search start");
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
            .expect("type o");
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE))
            .expect("type n");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .expect("confirm search");

        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            assert!(!state.search_active);
            assert_eq!(state.search_query, "on");
            assert_eq!(state.filtered_indices.len(), 2); // only1.txt, only2.txt
        }

        // 測試退出比對 (q)
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
            .expect("quit diff");
        assert!(app.pending_action.is_none());
        assert_eq!(app.status, "diff matrix closed");

        // 測試快速指令別名 :d 1 2
        app.execute_command("d 1 2").expect("execute d 1 2");
        assert!(matches!(app.pending_action, Some(PendingAction::DiffMatrix(_))));
        if let Some(PendingAction::DiffMatrix(state)) = &app.pending_action {
            assert_eq!(state.panel_ids, vec![1, 2]);
        }

        // 退出
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).expect("esc");
        assert!(app.pending_action.is_none());

        // 測試快捷鍵 wd (WindowPicker -> d)
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)).expect("w");
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)).expect("d");
        assert!(matches!(app.pending_action, Some(PendingAction::DiffMatrix(_))));

        // 退出
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).expect("esc");
        assert!(app.pending_action.is_none());

        // 測試快捷鍵 wD (WindowPicker -> D: prefilled :diff )
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE)).expect("w");
        app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT)).expect("D");
        assert!(app.command_mode);
        assert_eq!(app.command_buffer, "diff ");
    }
}
