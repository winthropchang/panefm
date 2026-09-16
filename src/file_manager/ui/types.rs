use ratatui::text::Span;

use crate::file_manager::{app::RenameMode, search::GlobalSearchEntry, tools::ToolStatus};

/// 描述底部快捷鍵面板中的單一項目。
#[derive(Clone, Copy)]
pub(crate) struct ShortcutPanelItem<'a> {
    pub(crate) shortcut: &'a str,
    pub(crate) label: &'a str,
}

/// 描述 command palette 繪製所需的狀態。
pub(crate) struct CommandPaletteState<'a> {
    pub(crate) buffer: &'a str,
    pub(crate) suggestions: &'a [CommandSuggestionLine],
    pub(crate) selected: usize,
    pub(crate) cursor: usize,
    pub(crate) mode: RenameMode,
}

/// 描述 inline 編輯器目前需要顯示的內容、標題與游標位置。
///
/// 這個結構只負責把 `App` 的輸入狀態轉交給 UI，
/// 讓繪圖函數可以知道目前文字內容、游標在哪裡、處於哪一種模式，
/// 還有應該顯示哪一種標題。
#[derive(Clone, Copy)]
pub(crate) struct InlineEditorState<'a> {
    pub(crate) buffer: &'a str,
    pub(crate) cursor: usize,
    pub(crate) title: &'a str,
}

/// 描述 inline 選單目前需要顯示的標題、選項與游標位置。
#[derive(Clone, Copy)]
pub(crate) struct InlinePickerState<'a> {
    pub(crate) title: &'a str,
    pub(crate) options: &'a [String],
    pub(crate) selected: usize,
}

/// 描述目前 pane 是否要把主列表暫時切換成 global search 的結果畫面。
#[derive(Clone, Copy)]
pub(crate) struct SearchListState<'a> {
    pub(crate) results: &'a [GlobalSearchEntry],
    pub(crate) selected: usize,
    pub(crate) loading: bool,
    pub(crate) preview_query: Option<&'a str>,
    pub(crate) preview_scroll: Option<usize>,
    pub(crate) preview_current_match: Option<usize>,
}

/// 描述目前 pane 的列表區是否被某種特殊模式接管。
#[derive(Clone, Copy)]
pub(crate) enum PaneListState<'a> {
    Search(SearchListState<'a>),
    Tasks {
        lines: &'a [TaskPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
        cursor: usize,
    },
    Trash {
        lines: &'a [TrashPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
        cursor: usize,
    },
    Help {
        lines: &'a [HelpPanelLine],
        selected: usize,
        search: &'a str,
        editing: bool,
        cursor: usize,
        custom_title: Option<&'a str>,
    },
    Tools {
        statuses: &'a [ToolStatus],
        selected: usize,
    },
    RegexRename {
        lines: &'a [RegexRenamePanelLine],
        selected: usize,
    },
}

/// 描述 trash 面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrashPanelLine {
    pub(crate) name: String,
    pub(crate) original_path: String,
    pub(crate) deleted_at: String,
    pub(crate) marked: bool,
}

/// 描述說明面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HelpPanelLine {
    pub(crate) command: String,
    pub(crate) shortcut: String,
    pub(crate) description: String,
}

/// 描述 task 面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskPanelLine {
    pub(crate) state: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: String,
    pub(crate) progress: String,
    pub(crate) title: String,
    /// 任務來源位置；多選操作可包含多筆，渲染時會限制展開數量避免面板過長。
    pub(crate) source_locations: Vec<String>,
    /// 任務目的位置；刪除等沒有目的地的工作使用 `None`。
    pub(crate) destination_location: Option<String>,
    pub(crate) detail: String,
    pub(crate) marked: bool,
}

/// 描述書籤列表彈窗中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BookmarkPanelLine {
    pub(crate) key: String,
    pub(crate) path: String,
}

/// 描述 zoxide 目錄列表彈窗中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ZoxidePanelLine {
    pub(crate) path: String,
}

/// 描述 regex 批次改名預覽面板中單一列要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegexRenamePanelLine {
    pub(crate) original_name: String,
    pub(crate) new_name: String,
    pub(crate) status: String,
}

/// 描述 command palette 中單一條命令補全候選。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandSuggestionLine {
    pub(crate) command: String,
    pub(crate) display_command: String,
    pub(crate) shortcut: String,
    pub(crate) description: String,
}

/// 描述單行文字輸入框經過水平滑動視窗計算後的顯示內容與游標欄位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScrolledInputView {
    pub(crate) spans: Vec<Span<'static>>,
    pub(crate) cursor_col: u16,
}

/// 表示列表需要區分的檔案類別。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileCategory {
    File,
    Executable,
    Image,
    Archive,
    Source,
}
