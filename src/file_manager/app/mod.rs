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
        default_extract_output_path, detect_archive_format, extract_entries,
        extract_entries_with_progress,
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
        TransferProgress,
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
pub(crate) enum TextEditResult {
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
pub(crate) struct NetworkGotoEvent {
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
pub(crate) enum FileJobEvent {
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
pub(crate) enum DirectorySizeEvent {
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
pub(crate) struct DirectorySizeJob {
    cwd: PathBuf,
    receiver: Receiver<DirectorySizeEvent>,
    cancelled: Arc<AtomicBool>,
}

/// 保存單一 panel 的目錄清單背景載入工作。
///
/// `cwd` 用來比對當前目錄；`cancelled` 讓新導航發生時，舊 worker 能立即在分塊邊界停止，
/// 避免背景磁碟 I/O 阻塞主事件迴圈。
#[derive(Debug)]
pub(crate) struct DirectoryLoadJob {
    cwd: PathBuf,
    receiver: Receiver<DirectoryLoadEvent>,
    cancelled: Arc<AtomicBool>,
}

/// 大型目錄背景讀取完成或分批串流送回主迴圈的資料。
#[derive(Debug)]
pub(crate) struct DirectoryLoadEvent {
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
pub(crate) struct PasteJobResult {
    history_items: Vec<OperationItem>,
    pasted_count: usize,
    failure: Option<PasteJobFailure>,
}

/// 記錄背景 paste 的失敗項目，供主執行緒顯示完整來源、目的與 OS error。
#[derive(Debug)]
pub(crate) struct PasteJobFailure {
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
        warning_message: Option<String>,
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
        custom_title: Option<String>,
        custom_entries: Option<Vec<HelpEntry>>,
    },
    TaskPanel {
        pane_id: usize,
        selected: usize,
        search: PanelSearchState,
        marked_ids: Vec<usize>,
        visual_anchor: Option<usize>,
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

mod commands;
mod file_ops;
mod keys;
mod navigation;
mod polling;
mod selection;
#[cfg(test)]
mod tests;

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

    /// 回傳目前 active panel 的目錄；供 OSC 7 終端同步或外部查詢使用。
    pub(crate) fn active_pane_cwd(&self) -> Option<&Path> {
        self.panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.as_path())
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
                    marked_ids,
                    visual_anchor,
                    selected,
                }),
            ) = (task_records.as_ref(), self.pending_action.as_ref())
            {
                if *action_pane_id == pane_id {
                    let filtered = filtered_task_entries(records, &search.buffer);
                    let mut effective_marked = marked_ids.clone();
                    if let Some(anchor) = visual_anchor {
                        let start = (*anchor).min(*selected);
                        let end = (*anchor).max(*selected);
                        for task in filtered
                            .iter()
                            .skip(start)
                            .take(end.saturating_sub(start) + 1)
                        {
                            if !effective_marked.contains(&task.id) {
                                effective_marked.push(task.id);
                            }
                        }
                    }
                    Some(task_panel_lines(&filtered, &effective_marked))
                } else {
                    None
                }
            } else {
                None
            };
            let help_lines = if let Some(PendingAction::HelpPanel {
                pane_id: action_pane_id,
                search,
                custom_entries,
                ..
            }) = &self.pending_action
            {
                if *action_pane_id == pane_id {
                    Some(if let Some(custom) = custom_entries {
                        filter_custom_help_entries(custom, &search.buffer)
                            .into_iter()
                            .map(|e| e.line)
                            .collect()
                    } else {
                        help_panel_lines(&search.buffer)
                    })
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
                    ..
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
                    custom_title,
                    ..
                }) = &self.pending_action
                {
                    if *action_pane_id == pane_id {
                        Some(PaneListState::Help {
                            lines: help_lines.as_deref().unwrap_or(&[]),
                            selected: *selected,
                            search: &search.buffer,
                            editing: search.editing,
                            cursor: self.text_input_cursor,
                            custom_title: custom_title.as_deref(),
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

        let shortcut_hints = self.active_status_shortcut_hints();
        let help = Paragraph::new(status_shortcut_line(
            outer[1].width,
            self.theme,
            &shortcut_hints,
        ))
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
                warning_message,
                ..
            }) => {
                render_confirm_dialog(
                    frame,
                    frame.area(),
                    target_name,
                    *permanent,
                    warning_message.as_deref(),
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

    /// 取出並清除下一幀的完整重畫需求。
    ///
    /// 參數：`self: &mut App`，目前應用程式狀態。
    /// 回傳：`bool`；`true` 代表事件迴圈必須先呼叫 `Terminal::clear()`。
    pub(crate) fn take_full_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.full_redraw_requested)
    }

    /// 取出目前排隊中的外部開啟請求，交給主事件迴圈處理。
    pub(crate) fn take_pending_launch(&mut self) -> Option<QueuedLaunch> {
        self.pending_launch.take()
    }

    /// 取出目前排隊中的 `fzf` 跳轉請求，交給主事件迴圈處理。
    pub(crate) fn take_pending_fzf_jump(&mut self) -> Option<FzfJumpRequest> {
        self.pending_fzf_jump.take()
    }
}

/// 將設定檔中的啟動偏好套用到新建立的 pane。
///
/// 參數：
/// - `config: &AppConfig`，目前啟動所使用的設定。
/// - `pane: &mut PaneState`，要被套用預設值的 pane。
///
/// 回傳：`()`
pub(crate) fn apply_config_to_pane(config: &AppConfig, pane: &mut PaneState) {
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
pub(crate) fn visible_job_badge_paths(pane: &PaneState, viewport_height: usize) -> Vec<PathBuf> {
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
pub(crate) fn sort_mode_from_config(sort: StartupSort, reverse: bool) -> SortMode {
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
pub(crate) fn bookmark_panel_lines(entries: Vec<BookmarkEntry>) -> Vec<BookmarkPanelLine> {
    entries
        .into_iter()
        .map(|entry| BookmarkPanelLine {
            key: format!("[{}]", entry.key),
            path: entry.target.display_text(),
        })
        .collect()
}

/// 依照搜尋字串過濾書籤清單，讓書籤列表也能使用 `f` 做即時篩選。
pub(crate) fn filtered_bookmark_entries(
    entries: Vec<BookmarkEntry>,
    query: &str,
) -> Vec<BookmarkEntry> {
    fuzzy_matched_indices_by_fields(&entries, query, |entry| {
        vec![entry.key.to_string(), entry.target.display_text()]
    })
    .into_iter()
    .map(|index| entries[index].clone())
    .collect()
}

/// 依書籤列表模式回傳彈窗標題與空狀態訊息。
pub(crate) fn bookmark_picker_copy(mode: BookmarkListMode) -> (&'static str, &'static str) {
    match mode {
        BookmarkListMode::Jump => (" Bookmarks ", "沒有書籤，按 b 再按 s 新增"),
        BookmarkListMode::Delete => (" Delete Bookmark ", "沒有可刪除的書籤"),
    }
}

/// 將 zoxide 目錄清單轉成彈窗可直接顯示的列內容。
pub(crate) fn zoxide_panel_lines(entries: Vec<PathBuf>) -> Vec<ZoxidePanelLine> {
    entries
        .into_iter()
        .map(|path| ZoxidePanelLine {
            path: path.display().to_string(),
        })
        .collect()
}

/// 依照搜尋字串過濾 zoxide 回傳的目錄列表，保留路徑中包含關鍵字的項目。
pub(crate) fn filtered_zoxide_entries(entries: &[PathBuf], query: &str) -> Vec<PathBuf> {
    fuzzy_matched_indices(entries, query, |path| path.display().to_string().into())
        .into_iter()
        .map(|index| entries[index].clone())
        .collect()
}

/// 依照搜尋字串過濾 task 清單，方便在任務很多時快速縮小範圍。
///
/// 參數：`tasks: &[TaskRecord]` 是原始任務；`query: &str` 是面板搜尋文字。
/// 回傳：`Vec<TaskRecord>`，會比對狀態、操作、結果、種類、所有來源與目的地。
pub(crate) fn filtered_task_entries(tasks: &[TaskRecord], query: &str) -> Vec<TaskRecord> {
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
pub(crate) fn filtered_global_search_entries(
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
pub(crate) fn parse_bookmark_argument(args: &str) -> Option<char> {
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
pub(crate) fn parse_pane_id_argument(args: &str) -> Option<usize> {
    let trimmed = args.trim();
    let id = trimmed.parse::<usize>().ok()?;
    (id > 0).then_some(id)
}

/// 判斷目前 command mode 輸入看起來是不是一條目錄或檔案路徑。
pub(crate) fn looks_like_navigation_path(input: &str) -> bool {
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
pub(crate) fn is_windows_drive_path(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

/// 判斷字串是否為 UNC 路徑，例如 `\\\\server\\share` 或 `//server/share`。
pub(crate) fn is_unc_path(input: &str) -> bool {
    input.starts_with("\\\\") || input.starts_with("//")
}

/// 展開 `~` 開頭的家目錄路徑，讓 command mode 也能直接輸入家目錄捷徑。
pub(crate) fn expand_tilde_path(input: &str) -> Option<String> {
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
pub(crate) fn command_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 描述 `g` 面板可快速跳轉的系統常用目錄。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GoSpecialDirectory {
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
pub(crate) fn special_directory_path(directory: GoSpecialDirectory) -> Option<PathBuf> {
    let home = command_home_dir()?;
    Some(home.join(directory.relative_name()))
}

/// 判斷 regex 批次改名某一列目前屬於可改名、無變化還是無效名稱。
pub(crate) fn classify_regex_rename_preview(
    original_name: &str,
    new_name: &str,
) -> RegexRenameOutcome {
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
pub(crate) fn regex_rename_panel_lines(
    previews: &[RegexRenamePreview],
) -> Vec<RegexRenamePanelLine> {
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
pub(crate) fn regex_rename_status(
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
pub(crate) fn unique_regex_rename_temp_path(
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
pub(crate) fn typed_char_from_key(key: &KeyEvent) -> Option<char> {
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
pub(crate) fn key_matches_plain_letter(key: &KeyEvent, lowercase: char) -> bool {
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
pub(crate) fn key_matches_shifted_letter(key: &KeyEvent, uppercase: char) -> bool {
    let lower = uppercase.to_ascii_lowercase();
    key.code == KeyCode::Char(uppercase)
        || (key.code == KeyCode::Char(lower) && key.modifiers.contains(KeyModifiers::SHIFT))
}

/// 判斷目前按鍵是否應視為 `~`，支援不同終端可能回報的格式差異。
///
/// 常見情況：
/// - 直接回報 `Char('~')`
/// - 回報 `Char('`') + Shift`
pub(crate) fn key_matches_tilde(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('~')
        || (key.code == KeyCode::Char('`') && key.modifiers.contains(KeyModifiers::SHIFT))
}

/// 判斷某個英文字母命令是否要接受大小寫等價輸入。
///
/// 這主要用在 `y/n` 這種確認提示，或某些不區分大小寫的互動按鍵。
pub(crate) fn key_matches_letter_any_case(key: &KeyEvent, letter: char) -> bool {
    let lower = letter.to_ascii_lowercase();
    let upper = letter.to_ascii_uppercase();
    key_matches_plain_letter(key, lower) || key_matches_shifted_letter(key, upper)
}

/// 判斷 `Ctrl+字母` 指令，支援不同終端可能送出的大小寫字元格式。
pub(crate) fn key_matches_ctrl_letter(key: &KeyEvent, letter: char) -> bool {
    let lower = letter.to_ascii_lowercase();
    let upper = letter.to_ascii_uppercase();
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c == lower || c == upper)
}

/// 判斷 `Ctrl+Shift+字母` 指令，支援不同終端可能送出的大小寫字元格式。
pub(crate) fn key_matches_ctrl_shift_letter(key: &KeyEvent, letter: char) -> bool {
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
pub(crate) fn ctrl_digit_target_pane_id(key: &KeyEvent) -> Option<usize> {
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
pub(crate) fn plain_digit_target_pane_id(key: &KeyEvent) -> Option<usize> {
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
pub(crate) enum SuggestionNavigation {
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
pub(crate) fn command_suggestion_navigation(key: &KeyEvent) -> Option<SuggestionNavigation> {
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
pub(crate) fn bookmark_list_status(
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
pub(crate) fn zoxide_list_status(
    query: &str,
    count: usize,
    selected: usize,
    editing: bool,
) -> String {
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
pub(crate) fn paste_should_run_in_background(
    clipboard: &ClipboardState,
    target_dir: &Path,
) -> bool {
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
pub(crate) fn entries_should_run_in_background(
    entries: &[crate::file_manager::entry::FileEntry],
) -> bool {
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
pub(crate) fn is_probably_network_or_external_path(path: &Path) -> bool {
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
pub(crate) fn perform_paste_job<F>(
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
pub(crate) fn scan_directory_sizes(
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
pub(crate) fn scan_one_directory_size(
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
pub(crate) fn should_report_directory_size(elapsed_ms: u64, previous_update_ms: u64) -> bool {
    elapsed_ms.saturating_sub(previous_update_ms) >= DIRECTORY_SIZE_UPDATE_INTERVAL_MS
}

/// byte 進度發生變化時送出事件；呼叫端負責以時間節流，避免大量 channel 訊息。
///
/// 參數：`sender` 為 worker event channel；`task_id` 為工作編號；`completed_bytes` 與
/// `total_bytes` 為累計量；`last_progress` 保存上次送出的 byte 組合。
/// 回傳：`() `；channel 已關閉時安靜停止回報，不影響檔案工作的錯誤處理。
pub(crate) fn send_progress_if_changed(
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

pub(crate) fn ensure_path_writable(path: &Path) {
    if let Ok(mut perms) = fs::metadata(path).map(|m| m.permissions()) {
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

/// 移除單一檔案或符號連結，遇到權限受阻時自動嘗試解除唯讀後重試，並回傳釋放的 byte 數。
pub(crate) fn remove_file_or_symlink_with_retry(path: &Path) -> io::Result<u64> {
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
pub(crate) fn remove_dir_all_fast_recursive<F>(path: &Path, on_progress: &mut F) -> io::Result<()>
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
pub(crate) fn remove_dir_all_parallel_with_progress<F>(
    path: &Path,
    on_progress: &mut F,
) -> io::Result<()>
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
pub(crate) fn paste_success_status(
    operation: ClipboardOperation,
    overwrite: bool,
    count: usize,
) -> String {
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
pub(crate) fn paste_failure_status(
    source_name: &str,
    destination: &Path,
    error: &io::Error,
) -> String {
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
pub(crate) struct StatusShortcutHint {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
}

impl App {
    /// 依目前開啟的畫面、面板或互動模式，動態產出最相關且有用的快捷鍵清單。
    /// 第一個項目永遠固定為 Help。
    pub(crate) fn active_status_shortcut_hints(&self) -> Vec<StatusShortcutHint> {
        let mut hints = Vec::new();
        // 第一個項目永遠固定為 Help，第二個固定為當前面板的 Cheatsheet
        hints.push(StatusShortcutHint {
            key: "~/F1",
            label: "help",
        });
        hints.push(StatusShortcutHint {
            key: "?",
            label: "cheat",
        });

        if self.command_mode {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "execute",
                },
                StatusShortcutHint {
                    key: "Tab",
                    label: "complete",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(filter) = &self.filter
            && filter.editing
        {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "confirm",
                },
                StatusShortcutHint {
                    key: "Tab",
                    label: "fuzzy/normal",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(search) = &self.preview_search
            && search.editing
        {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "confirm",
                },
                StatusShortcutHint {
                    key: "n/N",
                    label: "match",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(_find) = &self.list_find {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "Enter",
                    label: "confirm",
                },
                StatusShortcutHint {
                    key: "n/N",
                    label: "match",
                },
                StatusShortcutHint {
                    key: "Esc",
                    label: "cancel",
                },
            ]);
            return hints;
        }

        if let Some(search) = &self.global_search {
            if search.editing {
                hints.extend_from_slice(&[
                    StatusShortcutHint {
                        key: "Enter",
                        label: "start search",
                    },
                    StatusShortcutHint {
                        key: "Esc",
                        label: "cancel",
                    },
                ]);
            } else if search.filter.editing {
                hints.extend_from_slice(&[StatusShortcutHint {
                    key: "Enter/Esc",
                    label: "done filter",
                }]);
            } else {
                hints.extend_from_slice(&[
                    StatusShortcutHint {
                        key: "j/k",
                        label: "move",
                    },
                    StatusShortcutHint {
                        key: "Enter/l",
                        label: "jump",
                    },
                    StatusShortcutHint {
                        key: "f",
                        label: "filter",
                    },
                    StatusShortcutHint {
                        key: "i/s",
                        label: "edit query",
                    },
                    StatusShortcutHint {
                        key: "q/Esc",
                        label: "exit",
                    },
                ]);
            }
            return hints;
        }

        if let Some(action) = &self.pending_action {
            match action {
                PendingAction::TaskPanel {
                    search,
                    marked_ids,
                    visual_anchor,
                    ..
                } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else if visual_anchor.is_some() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "select range",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "commit visual",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "cancel visual",
                            },
                        ]);
                    } else if !marked_ids.is_empty() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "d",
                                label: "delete marked",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "unmark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "clear marks",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "visual",
                            },
                            StatusShortcutHint {
                                key: "x/c",
                                label: "cancel task",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "clear marks",
                            },
                        ]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "visual",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "mark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "mark all",
                            },
                            StatusShortcutHint {
                                key: "d",
                                label: "delete",
                            },
                            StatusShortcutHint {
                                key: "D",
                                label: "clear all",
                            },
                            StatusShortcutHint {
                                key: "x/c",
                                label: "cancel task",
                            },
                            StatusShortcutHint {
                                key: "X",
                                label: "cancel all",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::TrashPanel {
                    search,
                    marked_ids,
                    visual_anchor,
                    ..
                } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else if visual_anchor.is_some() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "select range",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "commit visual",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "cancel visual",
                            },
                        ]);
                    } else if !marked_ids.is_empty() {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "u",
                                label: "restore marked",
                            },
                            StatusShortcutHint {
                                key: "d",
                                label: "delete marked",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "unmark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "clear marks",
                            },
                            StatusShortcutHint {
                                key: "Esc",
                                label: "clear marks",
                            },
                        ]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "v",
                                label: "visual",
                            },
                            StatusShortcutHint {
                                key: "Space",
                                label: "mark",
                            },
                            StatusShortcutHint {
                                key: "a",
                                label: "mark all",
                            },
                            StatusShortcutHint {
                                key: "u",
                                label: "restore",
                            },
                            StatusShortcutHint {
                                key: "U",
                                label: "restore all",
                            },
                            StatusShortcutHint {
                                key: "d",
                                label: "delete",
                            },
                            StatusShortcutHint {
                                key: "D",
                                label: "clear all",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::DiffMatrix(diff_state) => {
                    if diff_state.search_active {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "Enter",
                                label: "diff file",
                            },
                            StatusShortcutHint {
                                key: "i",
                                label: "gitignore",
                            },
                            StatusShortcutHint {
                                key: ".",
                                label: "hidden",
                            },
                            StatusShortcutHint {
                                key: "r",
                                label: "rescan",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::HelpPanel { search, .. } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "Enter",
                                label: "execute",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::BookmarkList { search, mode, .. } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        match mode {
                            BookmarkListMode::Jump => {
                                hints.extend_from_slice(&[
                                    StatusShortcutHint {
                                        key: "j/k",
                                        label: "move",
                                    },
                                    StatusShortcutHint {
                                        key: "Enter",
                                        label: "jump",
                                    },
                                    StatusShortcutHint {
                                        key: "f",
                                        label: "search",
                                    },
                                    StatusShortcutHint {
                                        key: "q/Esc",
                                        label: "close",
                                    },
                                ]);
                            }
                            BookmarkListMode::Delete => {
                                hints.extend_from_slice(&[
                                    StatusShortcutHint {
                                        key: "j/k",
                                        label: "move",
                                    },
                                    StatusShortcutHint {
                                        key: "d/Enter",
                                        label: "delete",
                                    },
                                    StatusShortcutHint {
                                        key: "D",
                                        label: "clear all",
                                    },
                                    StatusShortcutHint {
                                        key: "f",
                                        label: "search",
                                    },
                                    StatusShortcutHint {
                                        key: "q/Esc",
                                        label: "close",
                                    },
                                ]);
                            }
                        }
                    }
                }
                PendingAction::ZoxideList { search, .. } => {
                    if search.editing {
                        hints.extend_from_slice(&[StatusShortcutHint {
                            key: "Enter/Esc",
                            label: "done search",
                        }]);
                    } else {
                        hints.extend_from_slice(&[
                            StatusShortcutHint {
                                key: "j/k",
                                label: "move",
                            },
                            StatusShortcutHint {
                                key: "Enter",
                                label: "jump",
                            },
                            StatusShortcutHint {
                                key: "f",
                                label: "search",
                            },
                            StatusShortcutHint {
                                key: "q/Esc",
                                label: "close",
                            },
                        ]);
                    }
                }
                PendingAction::BookmarkPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "a",
                            label: "add",
                        },
                        StatusShortcutHint {
                            key: "g",
                            label: "jump list",
                        },
                        StatusShortcutHint {
                            key: "d",
                            label: "delete list",
                        },
                        StatusShortcutHint {
                            key: "D",
                            label: "clear all",
                        },
                        StatusShortcutHint {
                            key: "b/Esc",
                            label: "close",
                        },
                    ]);
                }
                PendingAction::WindowPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "s/v",
                            label: "split h/v",
                        },
                        StatusShortcutHint {
                            key: "q",
                            label: "close",
                        },
                        StatusShortcutHint {
                            key: "o",
                            label: "only",
                        },
                        StatusShortcutHint {
                            key: "d",
                            label: "diff",
                        },
                        StatusShortcutHint {
                            key: "t",
                            label: "terminal",
                        },
                        StatusShortcutHint {
                            key: "1..9",
                            label: "focus",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "close",
                        },
                    ]);
                }
                PendingAction::SortPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "n",
                            label: "name",
                        },
                        StatusShortcutHint {
                            key: "s",
                            label: "size",
                        },
                        StatusShortcutHint {
                            key: "m",
                            label: "mtime",
                        },
                        StatusShortcutHint {
                            key: "e",
                            label: "ext",
                        },
                        StatusShortcutHint {
                            key: "r",
                            label: "reverse",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::GoPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "g",
                            label: "top",
                        },
                        StatusShortcutHint {
                            key: "d",
                            label: "documents",
                        },
                        StatusShortcutHint {
                            key: "k",
                            label: "desktop",
                        },
                        StatusShortcutHint {
                            key: "h",
                            label: "home",
                        },
                        StatusShortcutHint {
                            key: "t",
                            label: "goto path",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::LineModePicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "s",
                            label: "size",
                        },
                        StatusShortcutHint {
                            key: "m",
                            label: "mtime",
                        },
                        StatusShortcutHint {
                            key: "p",
                            label: "perms",
                        },
                        StatusShortcutHint {
                            key: "n",
                            label: "none",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::ThemePicker { .. } | PendingAction::ThemeCommandPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "preview",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "apply",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::OpenPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "open with",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::CopyPicker { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "copy text",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::Rename { .. } | PendingAction::CreateEntry { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "Enter",
                            label: "confirm",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::RegexRename { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "Enter",
                            label: "apply",
                        },
                        StatusShortcutHint {
                            key: "Esc",
                            label: "cancel",
                        },
                    ]);
                }
                PendingAction::ToolPanel { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "j/k",
                            label: "move",
                        },
                        StatusShortcutHint {
                            key: "q/Esc",
                            label: "close",
                        },
                    ]);
                }
                PendingAction::ConfirmDelete { .. }
                | PendingAction::ConfirmPasteOverwrite { .. }
                | PendingAction::ConfirmTrashAction { .. } => {
                    hints.extend_from_slice(&[
                        StatusShortcutHint {
                            key: "y",
                            label: "confirm",
                        },
                        StatusShortcutHint {
                            key: "n/Esc",
                            label: "cancel",
                        },
                    ]);
                }
            }
            return hints;
        }

        if self.visual_selection.is_some() {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "j/k",
                    label: "select range",
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
                    key: "d",
                    label: "trash",
                },
                StatusShortcutHint {
                    key: "D",
                    label: "delete",
                },
                StatusShortcutHint {
                    key: "C",
                    label: "compress",
                },
                StatusShortcutHint {
                    key: "Space",
                    label: "mark",
                },
                StatusShortcutHint {
                    key: "v/Esc",
                    label: "exit visual",
                },
            ]);
            return hints;
        }

        if let Some(pane) = self.panes.get(&self.focused_pane)
            && pane.is_preview_active()
        {
            hints.extend_from_slice(&[
                StatusShortcutHint {
                    key: "j/k",
                    label: "scroll",
                },
                StatusShortcutHint {
                    key: "Ctrl+d/u",
                    label: "page scroll",
                },
                StatusShortcutHint {
                    key: "Tab",
                    label: "close preview",
                },
                StatusShortcutHint {
                    key: "q",
                    label: "close preview",
                },
            ]);
            return hints;
        }

        // 預設（一般列表瀏覽模式）
        hints.extend_from_slice(status_shortcut_hints());
        // 移除重複的 help (因為 status_shortcut_hints() 本身第一項也是 help)
        hints.dedup();
        hints
    }
}

/// 回傳底部 status bar 允許顯示的標準預設快捷鍵。第一筆固定為 Help。
pub(crate) fn status_shortcut_hints() -> &'static [StatusShortcutHint] {
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
            key: "d/D",
            label: "delete",
        },
        StatusShortcutHint {
            key: "u",
            label: "undo",
        },
        StatusShortcutHint {
            key: "w",
            label: "panel",
        },
        StatusShortcutHint {
            key: "T",
            label: "tasks",
        },
    ]
}

/// 依 terminal 寬度與目前情境快捷鍵清單建立底部快捷鍵列，並把版本固定在最右側。
///
/// 參數：
/// - `width: u16`，目前快捷鍵列可使用的 terminal cell 寬度。
/// - `theme: Theme`，用來替按鍵套用目前主題的 accent 顏色。
/// - `hints: &[StatusShortcutHint]`，依照目前畫面或模式產生的快捷鍵清單。
///
/// 回傳：`Line<'static>`，可直接交給 ratatui `Paragraph` 繪製的單行內容。
pub(crate) fn status_shortcut_line(
    width: u16,
    theme: Theme,
    hints: &[StatusShortcutHint],
) -> Line<'static> {
    let available_width = usize::from(width);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let version_width = version.len();
    let version_gap = 2usize;
    let mut used_width = 0usize;
    let mut spans = Vec::new();

    for (index, hint) in hints.iter().enumerate() {
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
pub(crate) fn wrap_status_text(status: &str, width: u16) -> String {
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
pub(crate) fn status_area_height(wrapped_status: &str, max_height: u16) -> u16 {
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
pub(crate) fn status_is_error(status: &str) -> bool {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelpAction {
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelpEntry {
    line: HelpPanelLine,
    action: HelpAction,
}

/// 先依搜尋條件過濾 trash 原始資料，再提供給面板使用。
pub(crate) fn trash_panel_entries(
    trash_store: &TrashStore,
    query: &str,
) -> io::Result<Vec<TrashListEntry>> {
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
pub(crate) fn trash_panel_lines(
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
pub(crate) fn help_entries(query: &str) -> Vec<HelpEntry> {
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
            "打開目前 panel 的任務面板；支援 v 範圍選取、Space 標記、d/D 刪除與清空、x/X 取消任務",
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

/// 依照關鍵字過濾自訂的 HelpEntry 清單（如 Cheatsheet）。
pub(crate) fn filter_custom_help_entries(entries: &[HelpEntry], query: &str) -> Vec<HelpEntry> {
    if query.trim().is_empty() {
        return entries.to_vec();
    }
    fuzzy_matched_indices_by_fields(entries, query, |entry| {
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

/// 定義不同畫面或面板對應的情境種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextHelpKind {
    Normal,
    GlobalSearch,
    ListFind,
    TaskPanel,
    TrashPanel,
    DiffMatrix,
    VisualSelection,
    BookmarkPicker,
    BookmarkList,
    ZoxideList,
    WindowPicker,
    SortPicker,
    GoPicker,
    LineModePicker,
    ThemePicker,
    CommandMode,
    Filter,
    Preview,
    ToolPanel,
    RegexRename,
    Rename,
    CreateEntry,
    ConfirmAction,
    CopyPicker,
    OpenPicker,
}

impl App {
    /// 偵測目前焦點所在的互動情境或子面板種類。
    pub(crate) fn active_context_help_kind(&self) -> ContextHelpKind {
        if self.command_mode {
            return ContextHelpKind::CommandMode;
        }
        if let Some(filter) = &self.filter
            && filter.editing
        {
            return ContextHelpKind::Filter;
        }
        if let Some(search) = &self.preview_search
            && search.editing
        {
            return ContextHelpKind::Preview;
        }
        if self.global_search.is_some() {
            return ContextHelpKind::GlobalSearch;
        }
        if self.list_find.is_some() {
            return ContextHelpKind::ListFind;
        }
        if self.visual_selection.is_some() {
            return ContextHelpKind::VisualSelection;
        }
        if let Some(action) = &self.pending_action {
            match action {
                PendingAction::TaskPanel { .. } => ContextHelpKind::TaskPanel,
                PendingAction::TrashPanel { .. } => ContextHelpKind::TrashPanel,
                PendingAction::ConfirmTrashAction { .. }
                | PendingAction::ConfirmDelete { .. }
                | PendingAction::ConfirmPasteOverwrite { .. } => ContextHelpKind::ConfirmAction,
                PendingAction::DiffMatrix { .. } => ContextHelpKind::DiffMatrix,
                PendingAction::BookmarkPicker { .. } => ContextHelpKind::BookmarkPicker,
                PendingAction::BookmarkList { .. } => ContextHelpKind::BookmarkList,
                PendingAction::ZoxideList { .. } => ContextHelpKind::ZoxideList,
                PendingAction::WindowPicker { .. } => ContextHelpKind::WindowPicker,
                PendingAction::SortPicker { .. } => ContextHelpKind::SortPicker,
                PendingAction::GoPicker { .. } => ContextHelpKind::GoPicker,
                PendingAction::LineModePicker { .. } => ContextHelpKind::LineModePicker,
                PendingAction::ThemePicker { .. } | PendingAction::ThemeCommandPicker { .. } => {
                    ContextHelpKind::ThemePicker
                }
                PendingAction::ToolPanel { .. } => ContextHelpKind::ToolPanel,
                PendingAction::RegexRename { .. } => ContextHelpKind::RegexRename,
                PendingAction::Rename { .. } => ContextHelpKind::Rename,
                PendingAction::CreateEntry { .. } => ContextHelpKind::CreateEntry,
                PendingAction::CopyPicker { .. } => ContextHelpKind::CopyPicker,
                PendingAction::OpenPicker { .. } => ContextHelpKind::OpenPicker,
                _ => ContextHelpKind::Normal,
            }
        } else if let Some(pane) = self.panes.get(&self.focused_pane)
            && pane.is_preview_active()
        {
            ContextHelpKind::Preview
        } else {
            ContextHelpKind::Normal
        }
    }
}

/// 依情境種類產出對應的 Cheatsheet 標題與專屬功能快捷鍵清單。
pub(crate) fn context_cheatsheet_entries(kind: ContextHelpKind) -> (String, Vec<HelpEntry>) {
    match kind {
        ContextHelpKind::Normal => (
            String::from("Cheatsheet: Normal Mode (檔案列表)"),
            vec![
                help_entry("move", "j / k", "向下 / 向上移動檔案游標", HelpAction::QuitHint),
                help_entry("navigate", "h / l", "返回上一層目錄 / 進入資料夾或開啟檔案", HelpAction::QuitHint),
                help_entry("open", "Enter / o", "開啟所選檔案或進入資料夾", HelpAction::QuitHint),
                help_entry("copy", "y / Y", "複製目前檔案或所有標記項目 / 清除剪貼簿", HelpAction::Command("copy")),
                help_entry("cut", "x / X", "剪下目前檔案或所有標記項目 / 清除剪貼簿", HelpAction::Command("cut")),
                help_entry("paste", "p / P", "貼上檔案 / 強制覆蓋貼上", HelpAction::Command("paste")),
                help_entry("rename", "r", "重新命名目前選取的檔案或資料夾", HelpAction::Command("rename")),
                help_entry("regex-rename", "R / :reg", "開啟 Regex 批次改名預覽面板", HelpAction::Command("rename-regex")),
                help_entry("create", "a", "建立新檔案或目錄（以 / 結尾為資料夾）", HelpAction::Command("create")),
                help_entry("trash", "d", "將檔案移至垃圾桶 (需確認)", HelpAction::Command("trash")),
                help_entry("delete!", "D", "永久直接刪除檔案或資料夾 (不進垃圾桶)", HelpAction::Command("delete!")),
                help_entry("undo", "u", "復原上一步貼上或搬移操作 (Undo)", HelpAction::Command("undo")),
                help_entry("visual", "v / V", "開啟視覺連續多選模式 (Visual Selection)", HelpAction::Visual),
                help_entry("mark", "Space", "單檔切換標記 / 取消標記", HelpAction::QuitHint),
                help_entry("mark-all", "Ctrl+a", "全選目前目錄所有檔案與資料夾", HelpAction::QuitHint),
                help_entry("unmark-all", "Ctrl+Shift+a", "清除目前目錄所有標記", HelpAction::QuitHint),
                help_entry("invert-marks", "Ctrl+r", "反向切換目前目錄標記狀態", HelpAction::QuitHint),
                help_entry("preview", "Tab", "切換右側檔案預覽 / 進入預覽模式", HelpAction::Command("preview")),
                help_entry("hidden", ".", "切換顯示 / 隱藏以點開頭之隱藏檔", HelpAction::Hidden),
                help_entry("compress", "C", "將所選項目壓縮為 ZIP 壓縮檔", HelpAction::QuitHint),
                help_entry("extract", "E", "解壓縮所選壓縮檔（支援 zip, tar.gz, tar 等）", HelpAction::QuitHint),
                help_entry("sort", ",", "打開排序選單 (Name, Size, MTime, Ext, Reverse)", HelpAction::Sort),
                help_entry("search-name", "s", "啟動 fd 檔名全域即時搜尋", HelpAction::Command("search")),
                help_entry("search-content", "S", "啟動 rg 檔案內容全文即時搜尋", HelpAction::Command("search")),
                help_entry("jump", "z", "啟動 fzf 目錄樹模糊搜尋跳轉", HelpAction::Command("jump")),
                help_entry("zoxide", "Z", "打開 zoxide 常用歷史目錄跳轉清單", HelpAction::Command("zoxide")),
                help_entry("list-find", "/", "快速檔名跳轉搜尋 (List Find)", HelpAction::QuitHint),
                help_entry("filter", "f / F", "即時過濾目前目錄檔案 (Normal / Fuzzy Filter)", HelpAction::Filter),
                help_entry("linemode", "m", "打開 Linemode 選單 (Size, Perms, BTime, MTime, None)", HelpAction::Command("linemode ")),
                help_entry("bookmark", "b", "打開書籤快捷選單 (ba 新增, bg 跳轉, bd 刪除, bD 清空)", HelpAction::Command("bookmark")),
                help_entry("window", "w", "打開視窗分割與焦點選單 (wv 垂直, ws 水平, wh/j/k/l 切換, wc 關閉)", HelpAction::Command("window")),
                help_entry("theme", "t", "打開佈景主題快速切換選單", HelpAction::Command("theme list")),
                help_entry("tasks", "T", "打開任務管理面板 (檢視背景傳輸與執行進度)", HelpAction::Command("tasks")),
                help_entry("trash-panel", "gt", "打開垃圾桶面板 (檢視與還原已刪除項目)", HelpAction::Command("trash")),
                help_entry("diff", "Alt+d / :diff", "開啟全螢幕多 Panel 目錄矩陣與檔案內容比對", HelpAction::Command("diff")),
                help_entry("command", ":", "開啟底端命令列模式 (Command Mode)", HelpAction::Command("")),
                help_entry("help", "~/F1", "打開全局完整說明手冊 (Help Dictionary)", HelpAction::Command("help")),
                help_entry("cheatsheet", "?", "開啟當前面板快捷鍵指南 (Cheatsheet)", HelpAction::Command("cheatsheet")),
            ],
        ),
        ContextHelpKind::GlobalSearch => (
            String::from("Cheatsheet: Global Search (全域搜尋)"),
            vec![
                help_entry("move", "j / k / Down / Up", "在搜尋結果清單中上下移動", HelpAction::QuitHint),
                help_entry("page", "Ctrl+d / Ctrl+u", "快速半頁向下 / 向上翻頁", HelpAction::QuitHint),
                help_entry("jump-large", "J / K", "大步快速移動游標", HelpAction::QuitHint),
                help_entry("top/bottom", "gg / G", "跳至搜尋結果頂部 / 底部", HelpAction::QuitHint),
                help_entry("open", "Enter / l / Right", "跳轉並定位至所選檔案位置", HelpAction::QuitHint),
                help_entry("filter", "f", "在目前搜尋結果中進行即時模糊過濾", HelpAction::QuitHint),
                help_entry("re-edit", "i / s", "重新回到搜尋關鍵字輸入框重新搜尋", HelpAction::QuitHint),
                help_entry("preview", "Tab", "（內容搜尋模式）切換進入 / 離開右側檔案內容預覽", HelpAction::QuitHint),
                help_entry("preview-match", "n / p / N", "在預覽中跳至下一個 / 上一個內容比對匹配行", HelpAction::QuitHint),
                help_entry("preview-scroll", "j / k", "在預覽中上下捲動檔案內容", HelpAction::QuitHint),
                help_entry("exit", "Esc / q / h", "退出搜尋結果面板，返回檔案列表", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::ListFind => (
            String::from("Cheatsheet: List Find (檔名尋找)"),
            vec![
                help_entry("type", "Characters", "輸入要尋找的檔名關鍵字", HelpAction::QuitHint),
                help_entry("confirm", "Enter", "確認尋找並鎖定目標項目", HelpAction::QuitHint),
                help_entry("next/prev", "n / N", "在檔案列表中跳至下一個 / 上一個匹配項目", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "取消檔名尋找並退出", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::TaskPanel => (
            String::from("Cheatsheet: Task Panel (任務管理)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動選取任務", HelpAction::QuitHint),
                help_entry("page", "Ctrl+d / Ctrl+u", "快速半頁向下 / 向上翻頁", HelpAction::QuitHint),
                help_entry("top/bottom", "gg / G", "跳至任務清單頂部 / 底部", HelpAction::QuitHint),
                help_entry("visual", "v / V", "開啟視覺連續多選模式（連續標記多個任務）", HelpAction::QuitHint),
                help_entry("mark", "Space", "標記 / 取消標記目前任務", HelpAction::QuitHint),
                help_entry("mark-all", "a", "全選所有任務 / 清除所有標記", HelpAction::QuitHint),
                help_entry("delete", "d", "直接刪除所選或所有已標記的任務記錄（不彈窗）", HelpAction::QuitHint),
                help_entry("clear-all", "D", "直接清空面板中所有任務記錄", HelpAction::QuitHint),
                help_entry("cancel", "x / c", "取消目前正在執行的背景任務", HelpAction::QuitHint),
                help_entry("cancel-all", "X / C", "取消所有正在執行的背景任務", HelpAction::QuitHint),
                help_entry("search", "f", "開啟搜尋列，即時過濾任務名稱或路徑", HelpAction::QuitHint),
                help_entry("detail", "Enter / l / Right", "檢視該任務完整執行細節、路徑與錯誤訊息", HelpAction::QuitHint),
                help_entry("close", "Esc / q / t / h", "關閉任務面板，返回檔案列表", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::TrashPanel => (
            String::from("Cheatsheet: Trash Panel (垃圾桶)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動選取垃圾桶項目", HelpAction::QuitHint),
                help_entry("page", "Ctrl+d / Ctrl+u", "快速半頁向下 / 向上翻頁", HelpAction::QuitHint),
                help_entry("top/bottom", "gg / G", "跳至清單頂部 / 底部", HelpAction::QuitHint),
                help_entry("visual", "v / V", "開啟視覺多選模式（連續標記多個項目）", HelpAction::QuitHint),
                help_entry("mark", "Space", "標記 / 取消標記目前項目", HelpAction::QuitHint),
                help_entry("mark-all", "a", "全選所有項目 / 清除所有標記", HelpAction::QuitHint),
                help_entry("restore", "u", "還原所選或已標記項目回原本目錄（需確認）", HelpAction::QuitHint),
                help_entry("restore-all", "U", "還原垃圾桶內所有項目（需確認）", HelpAction::QuitHint),
                help_entry("delete", "d", "永久刪除所選或已標記項目（需確認）", HelpAction::QuitHint),
                help_entry("empty-trash", "D", "清空整個垃圾桶（永久刪除所有檔案，需確認）", HelpAction::QuitHint),
                help_entry("search", "f", "開啟搜尋列，即時過濾垃圾桶項目名稱", HelpAction::QuitHint),
                help_entry("detail", "Enter / l", "檢視原始路徑與刪除時間", HelpAction::QuitHint),
                help_entry("close", "Esc / q / h", "關閉垃圾桶面板，返回檔案列表", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::DiffMatrix => (
            String::from("Cheatsheet: Diff Matrix (檔案差異比對)"),
            vec![
                help_entry("move", "j / k", "在差異項目清單中上下移動", HelpAction::QuitHint),
                help_entry("switch-col", "h / l", "在左 / 中 / 右各 Panel 欄位間切換焦點", HelpAction::QuitHint),
                help_entry("toggle", "Space", "勾選 / 切換選取要同步的差異項目", HelpAction::QuitHint),
                help_entry("all", "a", "全選 / 取消全選所有差異項目", HelpAction::QuitHint),
                help_entry("diff-detail", "d", "開啟雙欄檔案內容詳細 Diff 比對檢視視窗", HelpAction::QuitHint),
                help_entry("filter-cycle", "f", "循環切換篩選（全部 ➔ 僅差異 ➔ 僅獨有 ➔ 相同）", HelpAction::QuitHint),
                help_entry("gitignore", "i", "切換 .gitignore 規則（包含/排除 build 與忽略檔）", HelpAction::QuitHint),
                help_entry("hidden", ".", "切換顯示 / 隱藏以點開頭之隱藏檔", HelpAction::QuitHint),
                help_entry("search", "/", "檔名關鍵字搜尋比對", HelpAction::QuitHint),
                help_entry("rescan", "r", "重新掃描所有比對 Panel 目錄", HelpAction::QuitHint),
                help_entry("apply", "Enter", "套用同步動作（將選取項目從來源複製至目標）", HelpAction::QuitHint),
                help_entry("exit", "Esc / q", "退出差異比對矩陣，返回多面板模式", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::VisualSelection => (
            String::from("Cheatsheet: Visual Selection (視覺連續選取)"),
            vec![
                help_entry("expand", "j / k / Down / Up", "延伸 / 縮小視覺連續選取範圍", HelpAction::QuitHint),
                help_entry("top/bottom", "gg / G", "連續選取至檔案清單頂部 / 底部", HelpAction::QuitHint),
                help_entry("copy", "y", "複製選取範圍內的所有檔案/資料夾 (Yank)", HelpAction::QuitHint),
                help_entry("cut", "x", "剪下選取範圍內的所有檔案/資料夾 (Cut)", HelpAction::QuitHint),
                help_entry("delete", "d", "批次刪除選取範圍內的所有檔案/資料夾", HelpAction::QuitHint),
                help_entry("rename-regex", "r / R", "對目前選取範圍開啟 Regex 批次改名預覽", HelpAction::QuitHint),
                help_entry("compress", "C", "將選取範圍內所有項目壓縮為 ZIP", HelpAction::QuitHint),
                help_entry("commit", "v", "將選取範圍提交為常規標記並退出視覺模式", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "取消視覺選取模式", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::WindowPicker => (
            String::from("Cheatsheet: Window Layout (視窗分割與管理)"),
            vec![
                help_entry("split-v", "v", "垂直新增分割視窗 (Vertical Split)", HelpAction::QuitHint),
                help_entry("split-s", "s", "水平新增分割視窗 (Horizontal Split)", HelpAction::QuitHint),
                help_entry("focus", "h / j / k / l", "切換焦點至 左 / 下 / 上 / 右 視窗", HelpAction::QuitHint),
                help_entry("close", "c / q", "關閉目前焦點視窗 (Close Pane)", HelpAction::QuitHint),
                help_entry("only", "o", "僅保留目前視窗，關閉其他所有視窗 (Only)", HelpAction::QuitHint),
                help_entry("diff", "d", "開啟多 Panel 目錄 Diff 矩陣比對", HelpAction::QuitHint),
                help_entry("terminal", "t", "在目前目錄開啟外部終端機 (Terminal)", HelpAction::QuitHint),
                help_entry("select-pane", "1..9", "直接切換焦點至指定編號視窗", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / w", "取消退出視窗管理選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::BookmarkPicker => (
            String::from("Cheatsheet: Bookmark (書籤管理)"),
            vec![
                help_entry("add", "a", "自動挑選下一個可用代號，將目前目錄存入書籤", HelpAction::QuitHint),
                help_entry("jump-list", "g", "打開書籤清單進行跳轉", HelpAction::QuitHint),
                help_entry("delete-list", "d", "打開書籤清單進行刪除", HelpAction::QuitHint),
                help_entry("clear-all", "D", "直接清空所有已儲存書籤", HelpAction::QuitHint),
                help_entry("quick-jump", "'{key}", "按單一按鍵直接跳轉至對應代號書籤", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / b", "取消退出書籤選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::BookmarkList => (
            String::from("Cheatsheet: Bookmark List (書籤清單)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動選擇書籤", HelpAction::QuitHint),
                help_entry("page", "Ctrl+d / Ctrl+u", "快速半頁向下 / 向上翻頁", HelpAction::QuitHint),
                help_entry("search", "f", "即時過濾書籤名稱或路徑", HelpAction::QuitHint),
                help_entry("jump", "Enter / l", "跳轉至所選書籤目錄（刪除模式下為刪除）", HelpAction::QuitHint),
                help_entry("delete", "d / {key}", "刪除對應代號書籤（在刪除模式下）", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / h", "取消退出書籤清單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::ZoxideList => (
            String::from("Cheatsheet: Zoxide (歷史目錄跳轉)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動選擇常用目錄", HelpAction::QuitHint),
                help_entry("page", "Ctrl+d / Ctrl+u", "快速半頁向下 / 向上翻頁", HelpAction::QuitHint),
                help_entry("top/bottom", "gg / G", "跳至清單頂部 / 底部", HelpAction::QuitHint),
                help_entry("filter", "f", "即時過濾歷史目錄路徑關鍵字", HelpAction::QuitHint),
                help_entry("jump", "Enter / l", "跳轉進入所選常用目錄", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / h", "取消並返回檔案列表", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::SortPicker => (
            String::from("Cheatsheet: Sort (檔案排序)"),
            vec![
                help_entry("by-name", "n", "依檔案名稱排序 (Name)", HelpAction::QuitHint),
                help_entry("by-size", "s", "依檔案大小排序 (Size)", HelpAction::QuitHint),
                help_entry("by-mtime", "m", "依修改時間排序 (Modified Time)", HelpAction::QuitHint),
                help_entry("by-ext", "e", "依副檔名排序 (Extension)", HelpAction::QuitHint),
                help_entry("reverse", "r", "反轉目前排序順序 (Reverse)", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / ,", "取消退出排序選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::LineModePicker => (
            String::from("Cheatsheet: Linemode (欄位顯示)"),
            vec![
                help_entry("size", "s", "右側欄位顯示檔案容量 (Size)", HelpAction::QuitHint),
                help_entry("perms", "p", "右側欄位顯示檔案權限 (Permissions)", HelpAction::QuitHint),
                help_entry("btime", "b", "右側欄位顯示建立時間 (Birth Time)", HelpAction::QuitHint),
                help_entry("mtime", "t / m", "右側欄位顯示修改時間 (Modified Time)", HelpAction::QuitHint),
                help_entry("none", "n", "簡潔模式，不顯示右側額外欄位 (None)", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / m", "取消退出欄位選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::GoPicker => (
            String::from("Cheatsheet: Quick Jump (快速跳轉)"),
            vec![
                help_entry("documents", "d", "快速跳轉至 ~/Documents 目錄", HelpAction::QuitHint),
                help_entry("desktop", "k", "快速跳轉至 ~/Desktop 目錄", HelpAction::QuitHint),
                help_entry("home", "h", "快速跳轉至使用者家目錄 ~", HelpAction::QuitHint),
                help_entry("goto-path", "t", "開啟路徑跳轉輸入框 (Goto path)", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / g", "取消退出跳轉選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::ThemePicker => (
            String::from("Cheatsheet: Theme (佈景主題)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動即時預覽佈景主題", HelpAction::QuitHint),
                help_entry("page", "Ctrl+d / Ctrl+u", "快速半頁向下 / 向上翻頁", HelpAction::QuitHint),
                help_entry("apply", "Enter / l", "套用所選主題並儲存為預設", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q / h", "取消並恢復原主題", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::CommandMode => (
            String::from("Cheatsheet: Command Mode (命令列模式)"),
            vec![
                help_entry("execute", "Enter", "執行目前輸入之指令", HelpAction::QuitHint),
                help_entry("complete", "Tab", "自動補全指令名稱或檔案路徑", HelpAction::QuitHint),
                help_entry("history", "Up / Down", "瀏覽歷史輸入指令", HelpAction::QuitHint),
                help_entry("start", "Ctrl+a / Home", "移動游標至指令開頭", HelpAction::QuitHint),
                help_entry("end", "Ctrl+e / End", "移動游標至指令結尾", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "取消並退出命令模式", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::Filter => (
            String::from("Cheatsheet: Filter (檔案即時過濾)"),
            vec![
                help_entry("confirm", "Enter", "確認鎖定目前過濾條件", HelpAction::QuitHint),
                help_entry("toggle-fuzzy", "Tab", "切換模糊過濾 (Fuzzy) 與精確過濾 (Exact)", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "清除過濾條件並返回完整清單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::Preview => (
            String::from("Cheatsheet: Preview (檔案預覽)"),
            vec![
                help_entry("scroll", "j / k / Down / Up", "向下 / 向上捲動預覽文字內容", HelpAction::QuitHint),
                help_entry("page", "Ctrl+d / Ctrl+u", "快速半頁向下 / 向上翻滾預覽", HelpAction::QuitHint),
                help_entry("search", "/", "在預覽內容中搜尋關鍵字", HelpAction::QuitHint),
                help_entry("match", "n / N", "跳至下一個 / 上一個搜尋匹配項目", HelpAction::QuitHint),
                help_entry("exit", "Tab / Esc / q / h", "退出預覽捲動，返回檔案列表", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::Rename => (
            String::from("Cheatsheet: Rename (重新命名)"),
            vec![
                help_entry("edit", "Characters", "編輯新檔案或資料夾名稱", HelpAction::QuitHint),
                help_entry("apply", "Enter", "確認套用重新命名", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "取消重新命名並返回檔案列表 (Normal 模式)", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::CreateEntry => (
            String::from("Cheatsheet: Create Entry (新增檔案/目錄)"),
            vec![
                help_entry("name", "Characters", "輸入名稱（以 / 結尾為資料夾，否則為檔案）", HelpAction::QuitHint),
                help_entry("create", "Enter", "確認建立新檔案或目錄", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "取消建立並返回檔案列表 (Normal 模式)", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::ConfirmAction => (
            String::from("Cheatsheet: Confirm Action (確認操作)"),
            vec![
                help_entry("confirm", "y / Enter", "確認執行此操作", HelpAction::QuitHint),
                help_entry("cancel", "n / Esc / q", "取消操作並返回", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::CopyPicker => (
            String::from("Cheatsheet: Copy Path (複製路徑)"),
            vec![
                help_entry("full-path", "1", "複製完整絕對路徑到剪貼簿", HelpAction::QuitHint),
                help_entry("filename", "2", "僅複製檔案名稱到剪貼簿", HelpAction::QuitHint),
                help_entry("parent-dir", "3", "複製所在目錄路徑到剪貼簿", HelpAction::QuitHint),
                help_entry("cancel", "Esc / c", "取消複製選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::OpenPicker => (
            String::from("Cheatsheet: Open With (開啟應用程式)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動選擇應用程式", HelpAction::QuitHint),
                help_entry("open", "Enter", "使用所選應用程式開啟檔案", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "取消並返回檔案列表", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::ToolPanel => (
            String::from("Cheatsheet: Tool Dependencies (相依工具)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動檢視相依工具狀態", HelpAction::QuitHint),
                help_entry("close", "Esc / q", "關閉相依工具面板", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::RegexRename => (
            String::from("Cheatsheet: Regex Batch Rename (批次改名)"),
            vec![
                help_entry("move", "j / k / Down / Up", "上下移動檢視改名預覽項目", HelpAction::QuitHint),
                help_entry("apply", "Enter", "確認套用批次改名", HelpAction::QuitHint),
                help_entry("cancel", "Esc / q", "取消並退出批次改名", HelpAction::QuitHint),
            ],
        ),
    }
}

/// 只取出 help 面板渲染需要的列內容。
pub(crate) fn help_panel_lines(query: &str) -> Vec<HelpPanelLine> {
    help_entries(query)
        .into_iter()
        .map(|entry| entry.line)
        .collect()
}


/// 根據解壓結果數量與略過項目數，整理出適合顯示在狀態列的訊息。
pub(crate) fn extraction_status_label(extracted: &[ExtractedArchive], skipped: usize) -> String {
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
pub(crate) fn command_suggestions_for_buffer(
    base_dir: Option<&Path>,
    query: &str,
) -> Vec<CommandSuggestionLine> {
    if let Some(context) = command_path_completion_context(base_dir, query) {
        return path_completion_suggestions(&context);
    }
    command_suggestions(query)
}

/// 根據目前 command mode 的輸入內容，整理出適合顯示的命令補全候選。
pub(crate) fn command_suggestions(query: &str) -> Vec<CommandSuggestionLine> {
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
pub(crate) fn command_suggestion_sort_key(query: &str, command: &str) -> (usize, String, String) {
    let head = command.split_whitespace().next().unwrap_or(command);
    let remainder = head.chars().count().saturating_sub(query.chars().count());
    (remainder, head.to_string(), command.to_string())
}

/// 找出多個候選字串的最長共同前綴，供路徑補全先延伸到共享部分。
pub(crate) fn longest_common_prefix(values: &[&str]) -> String {
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
pub(crate) struct CommandPathCompletionContext {
    replacement_prefix: String,
    typed_directory: String,
    search_dir: PathBuf,
    partial_name: String,
    preferred_separator: char,
    /// `true` 代表路徑會觸發 UNC/SMB 網路存取，不可在 command render 時同步掃描。
    network_path: bool,
}

/// 若目前 command buffer 正在輸入路徑，整理出路徑補全所需的上下文。
pub(crate) fn command_path_completion_context(
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
pub(crate) fn path_completion_suggestions(
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
pub(crate) fn split_typed_path(input: &str) -> (String, String) {
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
pub(crate) fn help_entry(
    command: &str,
    shortcut: &str,
    description: &str,
    action: HelpAction,
) -> HelpEntry {
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
pub(crate) fn trash_panel_status(
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
pub(crate) fn help_panel_status(query: &str, count: usize, editing: bool) -> String {
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
pub(crate) fn task_panel_status(
    query: &str,
    count: usize,
    selected: usize,
    editing: bool,
    marked_count: usize,
) -> String {
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
    } else if marked_count > 0 {
        format!(
            "tasks: {}/{} ({} marked) (d delete marked, v visual, a all, x/c cancel, f search)",
            selected + 1,
            count,
            marked_count
        )
    } else {
        format!(
            "tasks: {}/{} (d delete, D clear all, v visual, Space mark, x/c cancel, f search)",
            selected + 1,
            count
        )
    }
}

/// 依照本次貼上衝突的名稱與數量，產生覆蓋確認視窗的狀態列文字。
pub(crate) fn paste_overwrite_confirm_status(target_name: &str, entry_count: usize) -> String {
    if entry_count <= 1 {
        format!("confirm overwrite {target_name}: y/n")
    } else {
        format!("confirm overwrite {target_name} ({entry_count} items): y/n")
    }
}

/// 當使用者取消這次覆蓋貼上時，回傳狀態列要顯示的訊息。
pub(crate) fn paste_overwrite_cancelled_status(target_name: &str, entry_count: usize) -> String {
    if entry_count <= 1 {
        format!("paste cancelled: {target_name}")
    } else {
        format!("paste cancelled: {target_name} ({entry_count} items)")
    }
}

/// 依照 trash 確認操作種類，回傳確認視窗與狀態列要顯示的文字。
pub(crate) fn trash_confirm_status(
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
pub(crate) fn trash_confirm_cancelled_status(
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
pub(crate) fn trash_confirm_panel_id(action: &TrashConfirmAction) -> Option<usize> {
    match action {
        TrashConfirmAction::RestoreFromPanel { pane_id, .. }
        | TrashConfirmAction::DeleteFromPanel { pane_id, .. } => Some(*pane_id),
    }
}

/// 從 trash 確認操作還原出原本的 trash 面板狀態，讓取消或重繪時能留在同一個列表。
pub(crate) fn trash_panel_pending_action_from_confirm_action(
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
pub(crate) fn trash_panel_overlay_state_from_pending_action(
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
pub(crate) fn task_panel_lines(tasks: &[TaskRecord], marked_ids: &[usize]) -> Vec<TaskPanelLine> {
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
            marked: marked_ids.contains(&task.id),
        })
        .collect()
}

/// 產生 task 面板的 byte 進度欄，讓使用者直接判斷傳輸是否仍在前進。
///
/// 參數：`task: &TaskRecord`，要顯示的 task。
/// 回傳：`String`；支援進度的工作顯示 `24.4G / 77.2G`，一般外部工作顯示 `-`。
pub(crate) fn task_progress_label(task: &TaskRecord) -> String {
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
pub(crate) fn format_task_bytes(bytes: u64) -> String {
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
pub(crate) fn task_state_label(state: TaskState) -> &'static str {
    match state {
        TaskState::Running => "RUNNING",
        TaskState::Done => "DONE",
        TaskState::Failed => "FAILED",
        TaskState::Cancelled => "CANCELLED",
        TaskState::Interrupted => "INTERRUPTED",
    }
}

/// 將 unix 毫秒時間轉成 task 面板使用的簡短時間。
pub(crate) fn format_task_time(unix_ms: u64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms))
        .format("%H:%M:%S")
        .to_string()
}

/// 取得目前系統時間的 unix 毫秒。
pub(crate) fn unix_time_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 將 unix 毫秒時間轉成較容易閱讀的本地時間字串。
pub(crate) fn format_deleted_at(unix_ms: u64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms))
        .format("%m/%d %H:%M")
        .to_string()
}

/// 回傳建立流程的狀態列內容，讓使用者知道目前正處於哪一種編輯模式。
pub(crate) fn create_status_label(mode: &str) -> String {
    format!("create entry: {mode}")
}

/// 依照目前 preview search 文字與命中數量產生狀態列訊息。
pub(crate) fn preview_search_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("preview search: all")
    } else {
        format!("preview search: {buffer} ({matches})")
    }
}

/// 依照目前列表內 find-next 文字與命中數量產生狀態列訊息。
pub(crate) fn list_find_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("find next: type query")
    } else {
        format!("find next: {buffer} ({matches})")
    }
}

/// 依照目前列表內 find-next 文字與命中數量產生鎖定後的狀態列訊息。
pub(crate) fn list_find_locked_status(buffer: &str, matches: usize) -> String {
    if buffer.is_empty() {
        String::from("find next: empty")
    } else {
        format!("find next locked: {buffer} ({matches})")
    }
}

/// 依照目前 global search 文字、結果數與模式，產生狀態列訊息。
pub(crate) fn global_search_status(
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
pub(crate) fn global_search_visible_len(search: &GlobalSearchState) -> usize {
    filtered_global_search_entries(&search.results, &search.filter.buffer).len()
}

/// 建立 filter 狀態列文字。
pub(crate) fn format_filter_status(filter: &FilterState) -> String {
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
pub(crate) fn global_search_filter_status(filter: &PanelSearchState, matches: usize) -> String {
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
pub(crate) fn missing_search_tool_status(mode: SearchMode, tool: &str) -> String {
    format!("{} requires {tool}; run :status", mode.status_label())
}

/// 把 `fzf` 回傳的相對路徑文字轉回實際檔案系統路徑。
pub(crate) fn jump_selection_to_path(root_dir: &PathBuf, selection: &str) -> PathBuf {
    let mut target = root_dir.clone();
    let trimmed = selection.trim_end_matches('/');
    for segment in trimmed.split('/').filter(|segment| !segment.is_empty()) {
        target.push(segment);
    }
    target
}

/// 依照編輯模式決定建立輸入框的標題文字。
pub(crate) fn create_editor_title(mode: RenameMode) -> &'static str {
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
pub(crate) fn insert_char(buffer: &mut String, cursor: &mut usize, ch: char) {
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
pub(crate) fn backspace_char(buffer: &mut String, cursor: &mut usize) {
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
pub(crate) fn delete_char_at(buffer: &mut String, cursor: usize) {
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
pub(crate) fn char_to_byte_index(text: &str, char_index: usize) -> usize {
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
pub(crate) fn move_cursor_right(buffer: &str, cursor: usize) -> usize {
    let end = buffer.chars().count();
    (cursor + 1).min(end)
}

/// 將 Insert 模式的插入點轉成 Normal 模式可指向的字元位置。
pub(crate) fn normal_cursor(buffer: &str, cursor: usize) -> usize {
    cursor.min(buffer.chars().count().saturating_sub(1))
}

/// 在 Normal 模式向右移動一格，但不會移到最後一個字元之外。
pub(crate) fn normal_move_right(buffer: &str, cursor: usize) -> usize {
    (cursor + 1).min(buffer.chars().count().saturating_sub(1))
}

/// 回傳 normal 模式中 `$` 應該停留的位置，也就是最後一個可見字元。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
///
/// 回傳：`usize`，normal 模式游標應停留的字元索引。
pub(crate) fn rename_line_end_cursor(buffer: &str) -> usize {
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
pub(crate) fn is_rename_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
}

/// 找出 normal 模式下 `w` 應跳到的位置，也就是下一段名稱的開頭。
///
/// 參數：
/// - `buffer: &str`，目前正在編輯的檔名字串。
/// - `cursor: usize`，目前的字元游標位置。
///
/// 回傳：`usize`，下一個單字的起始字元索引。
pub(crate) fn rename_next_word_start(buffer: &str, cursor: usize) -> usize {
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
pub(crate) fn rename_previous_word_start(buffer: &str, cursor: usize) -> usize {
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
pub(crate) fn rename_word_end(buffer: &str, cursor: usize) -> usize {
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
pub(crate) fn rename_basename_cursor(name: &str) -> usize {
    let dot_index = name.rfind('.').filter(|index| *index > 0);
    match dot_index {
        Some(index) => name[..index].chars().count(),
        None => name.chars().count(),
    }
}
