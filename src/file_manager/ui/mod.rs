//! ratatui 畫面組裝與純顯示格式化函數。
//!
//! 本模組只根據 `App`/`PaneState` 的快照繪圖，不執行檔案操作或改變業務狀態。
//! panel 內 UI 應限制在傳入的 `Rect`，顏色一律取自 `Theme`，狹窄視窗則交由本層
//! 的截斷與動態對齊 helper 處理，避免各功能自行計算造成版面不一致。

pub(crate) mod dialogs;
pub(crate) mod diff;
pub(crate) mod entry;
pub(crate) mod input;
pub(crate) mod pane;
pub(crate) mod pickers;
pub(crate) mod text;
pub(crate) mod types;

#[allow(unused_imports)]
pub(crate) use crate::config::IconStyle;

#[allow(unused_imports)]
pub(crate) use dialogs::{
    centered_rect, render_confirm_dialog, render_paste_overwrite_dialog,
    render_trash_confirm_dialog,
};

#[allow(unused_imports)]
pub(crate) use diff::render_diff_matrix;

#[allow(unused_imports)]
pub(crate) use entry::{
    ascii_entry_icon, entry_icon, entry_style, file_category, highlight_match_style,
    highlight_name_spans, render_entry_line,
};

#[allow(unused_imports)]
pub(crate) use input::{
    render_command_palette, render_filter_input, render_global_search_panel, render_inline_editor,
    render_inline_picker, render_preview_search_input, render_top_right_input,
    top_right_input_rect,
};

#[allow(unused_imports)]
pub(crate) use pane::{
    format_pane_title, format_pane_title_parts, pad_preview_lines_for_render,
    regex_rename_status_style, render_pane, render_pane_title_line, render_update_badge,
    search_empty_message, search_list_selected_index, visible_list_window_range,
};

#[allow(unused_imports)]
pub(crate) use pickers::{
    render_bookmark_action_picker, render_bookmark_picker, render_go_picker,
    render_linemode_picker, render_shortcut_grid_panel, render_sort_picker,
    render_theme_command_picker, render_theme_picker, render_window_picker,
    render_window_resize_picker, render_yank_picker, render_zoxide_picker,
    shortcut_panel_item_width, shortcut_panel_lines,
};

#[allow(unused_imports)]
pub(crate) use text::{
    compact_last_segment_only, compact_path_for_title, compute_scrolled_input,
    cursor_display_width, format_compact_size, format_diff_path_column, format_permissions_detail,
    format_size_short, format_sort_detail, format_system_time, format_unix_permissions,
    join_title_parts, normalize_title_status_segments, prefix_len_clamp, task_panel_display_lines,
    title_separator_width, truncate_text, truncate_text_end_preserving_tail,
    truncate_text_to_display_width, wrap_text_for_width,
};

#[allow(unused_imports)]
pub(crate) use types::{
    BookmarkPanelLine, CommandPaletteState, CommandSuggestionLine, FileCategory, HelpPanelLine,
    InlineEditorState, InlinePickerState, PaneListState, RegexRenamePanelLine, ScrolledInputView,
    SearchListState, ShortcutPanelItem, TaskPanelLine, TrashPanelLine, ZoxidePanelLine,
};

#[cfg(test)]
#[path = "../tests/ui_test.rs"]
mod tests;
