use super::*;

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
    pub(crate) line: HelpPanelLine,
    pub(crate) action: HelpAction,
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
            "讓目前 panel 直接跳到指定路徑，支援相對路徑、絕對路徑、Windows 磁碟機路徑、UNC (// 或 \\\\) 與 smb:// share",
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
            "快速跳到 Desktop 目錄 (desKtop)",
            HelpAction::Command("goto ~/Desktop"),
        ),
        help_entry(
            ":goto downloads",
            "gl",
            "快速跳到 Downloads 目錄 (downLoad)",
            HelpAction::Command("goto ~/Downloads"),
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
            ":vdiff",
            "Ctrl+d",
            "切換目前選取檔案的版本控制差異預覽 (Git / SVN Unified Diff)",
            HelpAction::Command("vdiff"),
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
            "A",
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
            "U",
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
            ":copy-panel",
            "yp / y1..9",
            "把目前選取或標記的項目複製到指定 panel 編號目前所在的目錄",
            HelpAction::Command("copy-panel "),
        ),
        help_entry(
            ":move <path>",
            "mm",
            "直接把目前選取或標記的項目移動到指定目錄",
            HelpAction::Command("move "),
        ),
        help_entry(
            ":move-panel",
            "mp / m1..9",
            "把目前選取或標記的項目移動到指定 panel 編號目前所在的目錄",
            HelpAction::Command("move-panel "),
        ),
        help_entry(
            ":linemode",
            "m",
            "打開 Move / LineMode 功能面板，支援搬移與檔案欄位顯示設定",
            HelpAction::Command("linemode"),
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
            ":update",
            "",
            "檢查 GitHub 最新版本並原地自動下載替換執行檔",
            HelpAction::Command("update"),
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
            ":resize-mode",
            "wr",
            "進入視窗連續尺寸調整模式 (Sticky Resize Mode，hjkl/方向鍵微調)",
            HelpAction::Command("resize-mode"),
        ),
        help_entry(
            ":equal",
            "w=",
            "均等重設所有分割視窗大小 (Equalize)",
            HelpAction::Command("equal"),
        ),
        help_entry(
            ":width <+/-cols>",
            "wW",
            "調整目前視窗寬度（例如 :width +15 或 :width -10）",
            HelpAction::Command("width "),
        ),
        help_entry(
            ":height <+/-rows>",
            "wH",
            "調整目前視窗高度（例如 :height +5 或 :height -5）",
            HelpAction::Command("height "),
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
    WindowResize,
    SortPicker,
    GoPicker,
    LineModePicker,
    YankPicker,
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
                PendingAction::WindowResize { .. } => ContextHelpKind::WindowResize,
                PendingAction::SortPicker { .. } => ContextHelpKind::SortPicker,
                PendingAction::GoPicker { .. } => ContextHelpKind::GoPicker,
                PendingAction::LineModePicker { .. } => ContextHelpKind::LineModePicker,
                PendingAction::YankPicker { .. } => ContextHelpKind::YankPicker,
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
                help_entry(
                    "move",
                    "j / k",
                    "向下 / 向上移動檔案游標",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "navigate",
                    "h / l",
                    "返回上一層目錄 / 進入資料夾或開啟檔案",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "open",
                    "Enter / o",
                    "開啟所選檔案或進入資料夾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy",
                    "y / Y",
                    "複製目前檔案或所有標記項目 / 清除剪貼簿",
                    HelpAction::Command("copy"),
                ),
                help_entry(
                    "cut",
                    "x / X",
                    "剪下目前檔案或所有標記項目 / 清除剪貼簿",
                    HelpAction::Command("cut"),
                ),
                help_entry(
                    "paste",
                    "p / P",
                    "貼上檔案 / 強制覆蓋貼上",
                    HelpAction::Command("paste"),
                ),
                help_entry(
                    "rename",
                    "r",
                    "重新命名目前選取的檔案或資料夾",
                    HelpAction::Command("rename"),
                ),
                help_entry(
                    "regex-rename",
                    "R / :reg",
                    "開啟 Regex 批次改名預覽面板",
                    HelpAction::Command("rename-regex"),
                ),
                help_entry(
                    "create",
                    "a",
                    "建立新檔案或目錄（以 / 結尾為資料夾）",
                    HelpAction::Command("create"),
                ),
                help_entry(
                    "trash",
                    "d",
                    "將檔案移至垃圾桶 (需確認)",
                    HelpAction::Command("trash"),
                ),
                help_entry(
                    "delete!",
                    "D",
                    "永久直接刪除檔案或資料夾 (不進垃圾桶)",
                    HelpAction::Command("delete!"),
                ),
                help_entry(
                    "undo",
                    "u",
                    "復原上一步貼上或搬移操作 (Undo)",
                    HelpAction::Command("undo"),
                ),
                help_entry(
                    "visual",
                    "v / V",
                    "開啟視覺連續多選模式 (Visual Selection)",
                    HelpAction::Visual,
                ),
                help_entry(
                    "mark",
                    "Space",
                    "單檔切換標記 / 取消標記",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark-all",
                    "A",
                    "全選目前目錄所有檔案與資料夾",
                    HelpAction::Command("mark-all"),
                ),
                help_entry(
                    "unmark-all",
                    "U",
                    "清除目前目錄所有標記",
                    HelpAction::Command("unmark-all"),
                ),
                help_entry(
                    "invert-marks",
                    "Ctrl+r",
                    "反向切換目前目錄標記狀態",
                    HelpAction::Command("invert-marks"),
                ),
                help_entry(
                    "preview",
                    "Tab",
                    "切換右側檔案預覽 / 進入預覽模式",
                    HelpAction::Command("preview"),
                ),
                help_entry(
                    "hidden",
                    ".",
                    "切換顯示 / 隱藏以點開頭之隱藏檔",
                    HelpAction::Hidden,
                ),
                help_entry(
                    "compress",
                    "C",
                    "將所選項目壓縮為 ZIP 壓縮檔",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "extract",
                    "E",
                    "解壓縮所選壓縮檔（支援 zip, tar.gz, tar 等）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "sort",
                    ",",
                    "打開排序選單 (Name, Size, MTime, Ext, Reverse)",
                    HelpAction::Sort,
                ),
                help_entry(
                    "search-name",
                    "s",
                    "啟動 fd 檔名全域即時搜尋",
                    HelpAction::Command("search"),
                ),
                help_entry(
                    "search-content",
                    "S",
                    "啟動 rg 檔案內容全文即時搜尋",
                    HelpAction::Command("search"),
                ),
                help_entry(
                    "jump",
                    "z",
                    "啟動 fzf 目錄樹模糊搜尋跳轉",
                    HelpAction::Command("jump"),
                ),
                help_entry(
                    "zoxide",
                    "Z",
                    "打開 zoxide 常用歷史目錄跳轉清單",
                    HelpAction::Command("zoxide"),
                ),
                help_entry(
                    "list-find",
                    "/",
                    "快速檔名跳轉搜尋 (List Find)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filter",
                    "f / F",
                    "即時過濾目前目錄檔案 (Normal / Fuzzy Filter)",
                    HelpAction::Filter,
                ),
                help_entry(
                    "linemode",
                    "m",
                    "打開 Linemode 選單 (Size, Perms, BTime, MTime, None)",
                    HelpAction::Command("linemode "),
                ),
                help_entry(
                    "bookmark",
                    "b",
                    "打開書籤快捷選單 (ba 新增, bg 跳轉, bd 刪除, bD 清空)",
                    HelpAction::Command("bookmark"),
                ),
                help_entry(
                    "window",
                    "w",
                    "打開視窗分割與焦點選單 (wv 垂直, ws 水平, wh/j/k/l 切換, wc 關閉)",
                    HelpAction::Command("window"),
                ),
                help_entry(
                    "theme",
                    "t",
                    "打開佈景主題快速切換選單",
                    HelpAction::Command("theme list"),
                ),
                help_entry(
                    "tasks",
                    "T",
                    "打開任務管理面板 (檢視背景傳輸與執行進度)",
                    HelpAction::Command("tasks"),
                ),
                help_entry(
                    "trash-panel",
                    "gt",
                    "打開垃圾桶面板 (檢視與還原已刪除項目)",
                    HelpAction::Command("trash"),
                ),
                help_entry(
                    "diff",
                    "Alt+d / :diff",
                    "開啟全螢幕多 Panel 目錄矩陣與檔案內容比對",
                    HelpAction::Command("diff"),
                ),
                help_entry(
                    "command",
                    ":",
                    "開啟底端命令列模式 (Command Mode)",
                    HelpAction::Command(""),
                ),
                help_entry(
                    "help",
                    "~/F1",
                    "打開全局完整說明手冊 (Help Dictionary)",
                    HelpAction::Command("help"),
                ),
                help_entry(
                    "cheatsheet",
                    "?",
                    "開啟當前面板快捷鍵指南 (Cheatsheet)",
                    HelpAction::Command("cheatsheet"),
                ),
            ],
        ),
        ContextHelpKind::GlobalSearch => (
            String::from("Cheatsheet: Global Search (全域搜尋)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "在搜尋結果清單中上下移動",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump-large",
                    "J / K",
                    "大步快速移動游標",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "跳至搜尋結果頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "open",
                    "Enter / l / Right",
                    "跳轉並定位至所選檔案位置",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filter",
                    "f",
                    "在目前搜尋結果中進行即時模糊過濾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "re-edit",
                    "i / s",
                    "重新回到搜尋關鍵字輸入框重新搜尋",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "preview",
                    "Tab",
                    "（內容搜尋模式）切換進入 / 離開右側檔案內容預覽",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "preview-match",
                    "n / p / N",
                    "在預覽中跳至下一個 / 上一個內容比對匹配行",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "preview-scroll",
                    "j / k",
                    "在預覽中上下捲動檔案內容",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "exit",
                    "Esc / q / h",
                    "退出搜尋結果面板，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ListFind => (
            String::from("Cheatsheet: List Find (檔名尋找)"),
            vec![
                help_entry(
                    "type",
                    "Characters",
                    "輸入要尋找的檔名關鍵字",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "confirm",
                    "Enter",
                    "確認尋找並鎖定目標項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "next/prev",
                    "n / N",
                    "在檔案列表中跳至下一個 / 上一個匹配項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消檔名尋找並退出",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::TaskPanel => (
            String::from("Cheatsheet: Task Panel (任務管理)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選取任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "跳至任務清單頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "visual",
                    "v / V",
                    "開啟視覺連續多選模式（連續標記多個任務）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark",
                    "Space",
                    "標記 / 取消標記目前任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark-all",
                    "a",
                    "全選所有任務 / 清除所有標記",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d",
                    "直接刪除所選或所有已標記的任務記錄（不彈窗）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "clear-all",
                    "D",
                    "直接清空面板中所有任務記錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "x / c",
                    "取消目前正在執行的背景任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel-all",
                    "X / C",
                    "取消所有正在執行的背景任務",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "f",
                    "開啟搜尋列，即時過濾任務名稱或路徑",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "detail",
                    "Enter / l / Right",
                    "檢視該任務完整執行細節、路徑與錯誤訊息",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "close",
                    "Esc / q / t / h",
                    "關閉任務面板，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::TrashPanel => (
            String::from("Cheatsheet: Trash Panel (垃圾桶)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選取垃圾桶項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "跳至清單頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "visual",
                    "v / V",
                    "開啟視覺多選模式（連續標記多個項目）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark",
                    "Space",
                    "標記 / 取消標記目前項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mark-all",
                    "a",
                    "全選所有項目 / 清除所有標記",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "restore",
                    "u",
                    "還原所選或已標記項目回原本目錄（需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "restore-all",
                    "U",
                    "還原垃圾桶內所有項目（需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d",
                    "永久刪除所選或已標記項目（需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "empty-trash",
                    "D",
                    "清空整個垃圾桶（永久刪除所有檔案，需確認）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "f",
                    "開啟搜尋列，即時過濾垃圾桶項目名稱",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "detail",
                    "Enter / l",
                    "檢視原始路徑與刪除時間",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "close",
                    "Esc / q / h",
                    "關閉垃圾桶面板，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::DiffMatrix => (
            String::from("Cheatsheet: Diff Matrix (檔案差異比對)"),
            vec![
                help_entry(
                    "move",
                    "j / k",
                    "在差異項目清單中上下移動",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "switch-col",
                    "h / l",
                    "在左 / 中 / 右各 Panel 欄位間切換焦點",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "toggle",
                    "Space",
                    "勾選 / 切換選取要同步的差異項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "all",
                    "a",
                    "全選 / 取消全選所有差異項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "diff-detail",
                    "d",
                    "開啟雙欄檔案內容詳細 Diff 比對檢視視窗",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filter-cycle",
                    "f",
                    "循環切換篩選（全部 ➔ 僅差異 ➔ 僅獨有 ➔ 相同）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "gitignore",
                    "i",
                    "切換 .gitignore 規則（包含/排除 build 與忽略檔）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "hidden",
                    ".",
                    "切換顯示 / 隱藏以點開頭之隱藏檔",
                    HelpAction::QuitHint,
                ),
                help_entry("search", "/", "檔名關鍵字搜尋比對", HelpAction::QuitHint),
                help_entry(
                    "rescan",
                    "r",
                    "重新掃描所有比對 Panel 目錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "apply",
                    "Enter",
                    "套用同步動作（將選取項目從來源複製至目標）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "exit",
                    "Esc / q",
                    "退出差異比對矩陣，返回多面板模式",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::VisualSelection => (
            String::from("Cheatsheet: Visual Selection (視覺連續選取)"),
            vec![
                help_entry(
                    "expand",
                    "j / k / Down / Up",
                    "延伸 / 縮小視覺連續選取範圍",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "連續選取至檔案清單頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy",
                    "y",
                    "複製選取範圍內的所有檔案/資料夾 (Yank)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cut",
                    "x",
                    "剪下選取範圍內的所有檔案/資料夾 (Cut)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d",
                    "批次刪除選取範圍內的所有檔案/資料夾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "rename-regex",
                    "r / R",
                    "對目前選取範圍開啟 Regex 批次改名預覽",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "compress",
                    "C",
                    "將選取範圍內所有項目壓縮為 ZIP",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "commit",
                    "v",
                    "將選取範圍提交為常規標記並退出視覺模式",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消視覺選取模式",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::WindowPicker => (
            String::from("Cheatsheet: Window Layout (視窗分割與管理)"),
            vec![
                help_entry(
                    "split-v",
                    "v",
                    "垂直新增分割視窗 (Vertical Split)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "split-s",
                    "s",
                    "水平新增分割視窗 (Horizontal Split)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "focus",
                    "h / j / k / l",
                    "切換焦點至 左 / 下 / 上 / 右 視窗",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "close",
                    "c / q",
                    "關閉目前焦點視窗 (Close Pane)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "only",
                    "o",
                    "僅保留目前視窗，關閉其他所有視窗 (Only)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "diff",
                    "d",
                    "開啟多 Panel 目錄 Diff 矩陣比對",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "terminal",
                    "t",
                    "在目前目錄開啟外部終端機 (Terminal)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "resize-mode",
                    "r",
                    "進入視窗連續尺寸調整模式 (Sticky Resize Mode)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "equalize",
                    "=",
                    "均等重設所有分割視窗大小 (Equalize)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "width",
                    "W",
                    "調整目前視窗寬度 (預填 :width 指令)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "height",
                    "H",
                    "調整目前視窗高度 (預填 :height 指令)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "select-pane",
                    "1..9",
                    "直接切換焦點至指定編號視窗",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / w",
                    "取消退出視窗管理選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::WindowResize => (
            String::from("Cheatsheet: Window Resize Mode (視窗尺寸調整)"),
            vec![
                help_entry(
                    "shrink-width",
                    "h / Left",
                    "減少目前視窗寬度 (4 欄)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "expand-width",
                    "l / Right",
                    "增加目前視窗寬度 (4 欄)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "expand-height",
                    "k / Up",
                    "增加目前視窗高度 (2 列)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "shrink-height",
                    "j / Down",
                    "減少目前視窗高度 (2 列)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "equalize",
                    "=",
                    "均等平衡所有視窗尺寸 (Reset Equal)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "done",
                    "Esc / Enter / q",
                    "結束調整模式並返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::BookmarkPicker => (
            String::from("Cheatsheet: Bookmark (書籤管理)"),
            vec![
                help_entry(
                    "add",
                    "a",
                    "自動挑選下一個可用代號，將目前目錄存入書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump-list",
                    "g",
                    "打開書籤清單進行跳轉",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete-list",
                    "d",
                    "打開書籤清單進行刪除",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "clear-all",
                    "D",
                    "直接清空所有已儲存書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "quick-jump",
                    "'{key}",
                    "按單一按鍵直接跳轉至對應代號書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / b",
                    "取消退出書籤選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::BookmarkList => (
            String::from("Cheatsheet: Bookmark List (書籤清單)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選擇書籤",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "f",
                    "即時過濾書籤名稱或路徑",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump",
                    "Enter / l",
                    "跳轉至所選書籤目錄（刪除模式下為刪除）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "delete",
                    "d / {key}",
                    "刪除對應代號書籤（在刪除模式下）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消退出書籤清單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ZoxideList => (
            String::from("Cheatsheet: Zoxide (歷史目錄跳轉)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選擇常用目錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "top/bottom",
                    "gg / G",
                    "跳至清單頂部 / 底部",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filter",
                    "f",
                    "即時過濾歷史目錄路徑關鍵字",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "jump",
                    "Enter / l",
                    "跳轉進入所選常用目錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消並返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::SortPicker => (
            String::from("Cheatsheet: Sort (檔案排序)"),
            vec![
                help_entry(
                    "by-name",
                    "n",
                    "依檔案名稱排序 (Name)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "by-size",
                    "s",
                    "依檔案大小排序 (Size)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "by-mtime",
                    "m",
                    "依修改時間排序 (Modified Time)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "by-ext",
                    "e",
                    "依副檔名排序 (Extension)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "reverse",
                    "r",
                    "反轉目前排序順序 (Reverse)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / ,",
                    "取消退出排序選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::LineModePicker => (
            String::from("Cheatsheet: Move / Linemode (搬移與欄位顯示)"),
            vec![
                help_entry(
                    "move to path",
                    "m",
                    "開啟 :move 目錄搬移命令列",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "move to panel",
                    "p",
                    "開啟 :move-panel 編號搬移命令列",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "move to pane",
                    "1..9",
                    "直接將選取項目搬移到指定 Panel",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "size",
                    "s",
                    "右側欄位顯示檔案容量 (Size)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "perms",
                    "r",
                    "右側欄位顯示檔案權限 (Permissions)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "btime",
                    "b",
                    "右側欄位顯示建立時間 (Birth Time)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "mtime",
                    "t",
                    "右側欄位顯示修改時間 (Modified Time)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "none",
                    "n",
                    "簡潔模式，不顯示右側額外欄位 (None)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消退出選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::YankPicker => (
            String::from("Cheatsheet: Yank / Copy (複製到剪貼簿或視窗)"),
            vec![
                help_entry(
                    "copy to clipboard",
                    "y",
                    "複製選取或標記檔案到內部剪貼簿 (yy)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy to panel",
                    "p",
                    "開啟 :copy-panel 編號複製命令列",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "copy to pane",
                    "1..9",
                    "直接將選取項目複製到指定 Panel",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消退出複製選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::GoPicker => (
            String::from("Cheatsheet: Quick Jump (快速跳轉)"),
            vec![
                help_entry(
                    "documents",
                    "d",
                    "快速跳轉至 ~/Documents 目錄",
                    HelpAction::Command("goto ~/Documents"),
                ),
                help_entry(
                    "desktop",
                    "k",
                    "快速跳轉至 ~/Desktop 目錄 (desKtop)",
                    HelpAction::Command("goto ~/Desktop"),
                ),
                help_entry(
                    "downloads",
                    "l",
                    "快速跳轉至 ~/Downloads 目錄 (downLoad)",
                    HelpAction::Command("goto ~/Downloads"),
                ),
                help_entry(
                    "home",
                    "h",
                    "快速跳轉至使用者家目錄 ~",
                    HelpAction::Command("goto ~"),
                ),
                help_entry(
                    "goto-path",
                    "t",
                    "開啟路徑跳轉輸入框 (Goto path)",
                    HelpAction::Command("goto "),
                ),
                help_entry(
                    "cancel",
                    "Esc / q / g",
                    "取消退出跳轉選單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ThemePicker => (
            String::from("Cheatsheet: Theme (佈景主題)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動即時預覽佈景主題",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻頁",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "apply",
                    "Enter / l",
                    "套用所選主題並儲存為預設",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q / h",
                    "取消並恢復原主題",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::CommandMode => (
            String::from("Cheatsheet: Command Mode (命令列模式)"),
            vec![
                help_entry(
                    "execute",
                    "Enter",
                    "執行目前輸入之指令",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "complete",
                    "Tab",
                    "自動補全指令名稱或檔案路徑",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "history",
                    "Up / Down",
                    "瀏覽歷史輸入指令",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "start",
                    "Ctrl+a / Home",
                    "移動游標至指令開頭",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "end",
                    "Ctrl+e / End",
                    "移動游標至指令結尾",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消並退出命令模式",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::Filter => (
            String::from("Cheatsheet: Filter (檔案即時過濾)"),
            vec![
                help_entry(
                    "confirm",
                    "Enter",
                    "確認鎖定目前過濾條件",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "toggle-fuzzy",
                    "Tab",
                    "切換模糊過濾 (Fuzzy) 與精確過濾 (Exact)",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "清除過濾條件並返回完整清單",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::Preview => (
            String::from("Cheatsheet: Preview (檔案預覽)"),
            vec![
                help_entry(
                    "scroll",
                    "j / k / Down / Up",
                    "向下 / 向上捲動預覽文字內容",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "page",
                    "Ctrl+d / Ctrl+u",
                    "快速半頁向下 / 向上翻滾預覽",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "search",
                    "/",
                    "在預覽內容中搜尋關鍵字",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "match",
                    "n / N",
                    "跳至下一個 / 上一個搜尋匹配項目",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "exit",
                    "Tab / Esc / q / h",
                    "退出預覽捲動，返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::Rename => (
            String::from("Cheatsheet: Rename (重新命名)"),
            vec![
                help_entry(
                    "edit",
                    "Characters",
                    "編輯新檔案或資料夾名稱",
                    HelpAction::QuitHint,
                ),
                help_entry("apply", "Enter", "確認套用重新命名", HelpAction::QuitHint),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消重新命名並返回檔案列表 (Normal 模式)",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::CreateEntry => (
            String::from("Cheatsheet: Create Entry (新增檔案/目錄)"),
            vec![
                help_entry(
                    "name",
                    "Characters",
                    "輸入名稱（以 / 結尾為資料夾，否則為檔案）",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "create",
                    "Enter",
                    "確認建立新檔案或目錄",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消建立並返回檔案列表 (Normal 模式)",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ConfirmAction => (
            String::from("Cheatsheet: Confirm Action (確認操作)"),
            vec![
                help_entry(
                    "confirm",
                    "y / Enter",
                    "確認執行此操作",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "n / Esc / q",
                    "取消操作並返回",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::CopyPicker => (
            String::from("Cheatsheet: Copy Path (複製路徑)"),
            vec![
                help_entry(
                    "full-path",
                    "1",
                    "複製完整絕對路徑到剪貼簿",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "filename",
                    "2",
                    "僅複製檔案名稱到剪貼簿",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "parent-dir",
                    "3",
                    "複製所在目錄路徑到剪貼簿",
                    HelpAction::QuitHint,
                ),
                help_entry("cancel", "Esc / c", "取消複製選單", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::OpenPicker => (
            String::from("Cheatsheet: Open With (開啟應用程式)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動選擇應用程式",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "open",
                    "Enter",
                    "使用所選應用程式開啟檔案",
                    HelpAction::QuitHint,
                ),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消並返回檔案列表",
                    HelpAction::QuitHint,
                ),
            ],
        ),
        ContextHelpKind::ToolPanel => (
            String::from("Cheatsheet: Tool Dependencies (相依工具)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動檢視相依工具狀態",
                    HelpAction::QuitHint,
                ),
                help_entry("close", "Esc / q", "關閉相依工具面板", HelpAction::QuitHint),
            ],
        ),
        ContextHelpKind::RegexRename => (
            String::from("Cheatsheet: Regex Batch Rename (批次改名)"),
            vec![
                help_entry(
                    "move",
                    "j / k / Down / Up",
                    "上下移動檢視改名預覽項目",
                    HelpAction::QuitHint,
                ),
                help_entry("apply", "Enter", "確認套用批次改名", HelpAction::QuitHint),
                help_entry(
                    "cancel",
                    "Esc / q",
                    "取消並退出批次改名",
                    HelpAction::QuitHint,
                ),
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
