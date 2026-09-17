//! Help 系統的核心資料型別、情境種類定義與建構輔助工具。

use super::super::*;

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

/// 只取出 help 面板渲染需要的列內容。
pub(crate) fn help_panel_lines(query: &str) -> Vec<HelpPanelLine> {
    super::entries::help_entries(query)
        .into_iter()
        .map(|entry| entry.line)
        .collect()
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
