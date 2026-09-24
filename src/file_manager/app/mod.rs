//! PaneFM 的應用狀態機、命令分派與使用者操作流程。
//!
//! [`App`] 是第一層協調者：每個 panel 的瀏覽資料由 `PaneState` 保存，全域只保留
//! layout、焦點、剪貼簿與背景 task。`handle_key` 會按照「暫時 UI -> 文字輸入 ->
//! panel 模式 -> 一般列表」的優先順序分派事件，避免同一按鍵同時觸發兩種行為。
//!
//! 新功能應盡量把檔案系統或平台細節放進對應模組，這裡只保留狀態轉換；所有會
//! 阻塞的搜尋、外部程式或網路操作都必須排入背景流程，不能卡住 TUI 主執行緒。

#[allow(unused_imports)]
use std::{
    collections::{BTreeMap, BTreeSet},
    env, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    time::Instant,
};

#[allow(unused_imports)]
use anyhow::Result;
#[allow(unused_imports)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

#[allow(unused_imports)]
use crate::{
    config::{AppConfig, LoadedConfig, StartupLinemode, StartupSort},
    theme::{Theme, ThemePreset},
};

#[allow(unused_imports)]
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
    platform::{read_text_from_system_clipboard, write_text_to_system_clipboard},
    search::{
        GlobalSearchEntry, GlobalSearchEvent, stream_content_search_entries, stream_search_entries,
    },
    smb::{ResolvedSmbLocation, build_smb_mount_launch, parse_smb_location},
    task_history::{load_task_history, save_task_history, task_history_file_path},
    tools::external_tool_statuses,
    trash::{TrashListEntry, TrashStore},
    ui::{
        CommandPaletteState, CommandSuggestionLine, HelpPanelLine, InlineEditorState,
        InlinePickerState, PaneListState, SearchListState, render_bookmark_action_picker,
        render_bookmark_picker, render_command_palette, render_confirm_dialog, render_diff_matrix,
        render_filter_input, render_global_search_panel, render_go_picker, render_linemode_picker,
        render_pane, render_paste_overwrite_dialog, render_preview_search_input,
        render_theme_command_picker, render_theme_picker, render_trash_confirm_dialog,
        render_window_picker, render_window_resize_picker, render_yank_picker,
        render_zoxide_picker, visible_list_window_range,
    },
    zoxide::{ZoxideTracker, query_zoxide_directories},
};

#[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
#[allow(unused_imports)]
use super::smb::resolve_smb_location;

#[cfg(any(all(not(target_os = "windows"), not(target_os = "macos")), test))]
#[allow(unused_imports)]
use super::smb::resolve_smb_location_with_mount_root;

mod commands;
mod completion;
pub(crate) mod file_ops;
mod fs_jobs;
mod help;
mod keys;
mod keys_util;
mod lifecycle;
mod line_editor;
mod navigation;
mod pickers;
mod polling;
mod rename;
mod render;
mod render_dialogs;
mod render_pane;
mod selection;
mod state;
mod status;
mod tasks;
#[cfg(test)]
mod tests;
mod trash;

pub(crate) use completion::*;
pub(crate) use fs_jobs::*;
pub(crate) use help::*;
pub(crate) use keys_util::*;
pub(crate) use lifecycle::*;
pub(crate) use line_editor::*;
pub(crate) use pickers::*;
pub(crate) use rename::*;
pub(crate) use render::*;
#[allow(unused_imports)]
pub(crate) use render_pane::visible_job_badge_paths;
pub(crate) use state::*;
pub(crate) use status::*;
pub(crate) use tasks::*;
pub(crate) use trash::*;

impl App {
    /// 回傳目前 active panel 的目錄；供 OSC 7 終端同步或外部查詢使用。
    pub(crate) fn active_pane_cwd(&self) -> Option<&Path> {
        self.panes
            .get(&self.focused_pane)
            .map(|pane| pane.cwd.as_path())
    }

    /// 取得目前 panels 分配到的區域，若尚未 render 則回退至終端實際大小。
    pub(crate) fn current_pane_area(&self) -> Rect {
        self.latest_pane_area.unwrap_or_else(|| {
            crossterm::terminal::size()
                .map(|(w, h)| Rect::new(0, 0, w, h.saturating_sub(3)))
                .unwrap_or_else(|_| Rect::new(0, 0, 120, 40))
        })
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

    /// 檢查是否有即將接管 terminal 的外部程式（如外部編輯器或 fzf）。
    pub(crate) fn has_pending_external_takeover(&self) -> bool {
        self.pending_launch.is_some() || self.pending_fzf_jump.is_some()
    }
}
