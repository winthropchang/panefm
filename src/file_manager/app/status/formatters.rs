//! 狀態列文字排版、折疊、高度計算與各模式狀態字串格式化。

use std::time::SystemTime;

use unicode_width::UnicodeWidthChar;

use crate::file_manager::archive::ExtractedArchive;

use super::super::*;

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

/// 取得目前系統時間的 unix 毫秒。
pub(crate) fn unix_time_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
