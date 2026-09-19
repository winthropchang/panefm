use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc::Receiver},
    time::Instant,
};

use anyhow::Result;
use ratatui::layout::Rect;

use crate::{
    config::AppConfig,
    theme::{Theme, ThemePreset},
};

use crate::file_manager::{
    bookmark::BookmarkStore,
    diff::{DiffJobEvent, DiffMatrixState},
    filesystem_watcher::FilesystemWatcher,
    layout::LayoutNode,
    open::{LaunchSpec, OpenPickerOption, OpenTarget},
    operation_history::OperationHistory,
    pane::{FilterMode, PaneState},
    search::{GlobalSearchEntry, GlobalSearchEvent},
    trash::TrashStore,
    vcs::VcsManager,
    zoxide::ZoxideTracker,
};

use super::{
    completion::CommandCompletionCycle,
    fs_jobs::{DirectoryLoadJob, DirectorySizeJob, FileJobEvent},
    help::{HelpEntry, HelpReturnState},
    pickers::{BookmarkListMode, BookmarkPrompt},
    rename::RegexRenamePreview,
    tasks::TaskRecord,
    trash::TrashConfirmAction,
};

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
    pub(crate) fn status_label(self) -> &'static str {
        match self {
            SearchMode::Path => "global search",
            SearchMode::Content => "content search",
        }
    }

    /// 回傳搜尋輸入框標題。
    pub(crate) fn panel_title(self, _editing: bool) -> &'static str {
        match self {
            SearchMode::Path => " Global search file by fd ",
            SearchMode::Content => " Global search content by rg ",
        }
    }
}

/// 描述排隊中的外部命令與它對應的 task id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueuedLaunch {
    pub(crate) task_id: usize,
    pub(crate) launch: LaunchSpec,
}

/// 描述目前已知可升級的新版本資訊（用於渲染頂部黃底紅字徽章）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateBadgeInfo {
    pub(crate) latest_version: String,
    pub(crate) download_url: String,
    pub(crate) asset_name: String,
}

/// 描述一次 UNC 網路路徑背景跳轉的完成訊息。
///
/// worker 會在主執行緒之外複製並載入 [`PaneState`]；主迴圈收到結果後，只有在
/// task 尚未被取消且目標 panel 仍存在時才套用，避免失聯 SMB 主機凍結整個 TUI。
#[derive(Debug)]
pub(crate) struct NetworkGotoEvent {
    /// 對應 task manager 中的任務編號。
    pub(crate) task_id: usize,
    /// 啟動跳轉時的 active panel 編號。
    pub(crate) pane_id: usize,
    /// 使用者輸入的 UNC 目標，供狀態列與錯誤訊息顯示。
    pub(crate) target: PathBuf,
    /// 背景載入完成的 panel 狀態，或作業系統回傳的 I/O 錯誤。
    pub(crate) result: io::Result<PaneState>,
}

/// 記錄目前是否處於範圍標記模式，以及起點和目前游標位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VisualSelectionState {
    pub(crate) pane_id: usize,
    pub(crate) anchor: usize,
    pub(crate) current: usize,
}

/// 描述暫時面板中的搜尋輸入狀態。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PanelSearchState {
    pub(crate) buffer: String,
    pub(crate) editing: bool,
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
    WindowResize {
        pane_id: usize,
    },
    LineModePicker {
        pane_id: usize,
    },
    YankPicker {
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
    EasyMotion {
        pane_id: usize,
        target_char: Option<char>,
        labels: Vec<(char, usize)>,
    },
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
    pub(crate) network_goto_rx: Option<Receiver<NetworkGotoEvent>>,
    /// 目前 UNC `goto` 對應的 task id，供 Esc 與 task panel 取消後捨棄晚到結果。
    pub(crate) active_network_goto_task_id: Option<usize>,
    /// 所有大型 paste/compress/extract 工作接收端，以 task id 區分並允許並行完成。
    pub(crate) file_job_receivers: BTreeMap<usize, Receiver<FileJobEvent>>,
    /// 記錄目前正在由背景工作處理（寫入、壓縮、解壓、刪除）的路徑集合，用來防止使用者在傳輸中途進入未完成的目錄。
    pub(crate) active_file_job_busy_paths: BTreeMap<usize, Vec<PathBuf>>,
    /// 每個 panel 各自擁有的 linemode size 背景掃描，不會互相覆蓋或阻塞 TUI。
    pub(crate) directory_size_jobs: BTreeMap<usize, DirectorySizeJob>,
    /// 每個 panel 最新一次非阻塞目錄讀取；新導航會取代舊 worker 並即時取消舊掃描。
    pub(crate) directory_load_jobs: BTreeMap<usize, DirectoryLoadJob>,
    /// 已成功讀取的目錄清單快取；重複進出大型目錄時先立即顯示，再由背景結果校正。
    pub(crate) directory_entry_cache: BTreeMap<PathBuf, Vec<crate::file_manager::entry::FileEntry>>,
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
    pub(crate) filesystem_watcher: Option<FilesystemWatcher>,
    /// watcher 短時間內回報的目錄先集中在這裡，等 debounce 到期再一起刷新。
    pub(crate) pending_watched_directories: BTreeSet<PathBuf>,
    /// 下一次允許套用 watcher 刷新的時間；`None` 代表目前沒有待處理事件。
    pub(crate) filesystem_refresh_deadline: Option<Instant>,
    /// 要求主事件迴圈在下一幀前清除實體 terminal 與 ratatui buffer。
    pub(crate) full_redraw_requested: bool,
    /// 記錄最近一次 render 時 panels 所分配到的區域（不含頂部 tabs 與底部 status/hint）。
    pub(crate) latest_pane_area: Option<Rect>,
    /// 描述目前已知可升級的新版本資訊（用於渲染頂部黃底紅字徽章）。
    pub(crate) update_badge_info: Option<UpdateBadgeInfo>,
    /// 背景版本檢查接收端。
    pub(crate) update_check_rx: Option<Receiver<crate::updater::UpdateCheckResult>>,
    /// 內部就地升級工作接收端。
    pub(crate) in_app_update_rx: Option<Receiver<InAppUpdateMsg>>,
    /// 目前是否正在下載與安裝更新。
    pub(crate) in_app_updating: bool,
    /// 版本控制（Git 與 SVN）背景管理與查詢 worker。
    pub(crate) vcs_manager: VcsManager,
}

/// 應用程式內部就地升級通訊訊息。
#[derive(Debug)]
pub(crate) enum InAppUpdateMsg {
    /// 下載進度事件（已下載位元組, 總位元組）。
    Progress {
        downloaded: usize,
        total: Option<u64>,
    },
    /// 升級結束事件（成功回傳新版本字串，失敗回傳錯誤訊息）。
    Completed(Result<String, String>),
}
