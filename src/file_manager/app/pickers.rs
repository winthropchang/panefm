use std::path::{Path, PathBuf};

use super::{command_home_dir, fuzzy_matched_indices, fuzzy_matched_indices_by_fields};
use crate::file_manager::bookmark::BookmarkEntry;
use crate::file_manager::ui::{BookmarkPanelLine, ZoxidePanelLine};

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

/// 描述 `g` 面板可快速跳轉的系統常用目錄。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GoSpecialDirectory {
    Documents,
    Desktop,
    Downloads,
}

impl GoSpecialDirectory {
    /// 回傳狀態列與提示訊息會使用的目錄名稱。
    ///
    /// 參數：無。
    /// 回傳：`&'static str`。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Documents => "Documents",
            Self::Desktop => "Desktop",
            Self::Downloads => "Downloads",
        }
    }

    /// 回傳相對於使用者家目錄的預設子目錄名稱。
    ///
    /// 參數：無。
    /// 回傳：`&'static str`。
    pub(crate) fn relative_name(self) -> &'static str {
        match self {
            Self::Documents => "Documents",
            Self::Desktop => "Desktop",
            Self::Downloads => "Downloads",
        }
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

/// 把 `fzf` 回傳的相對路徑文字轉回實際檔案系統路徑。
pub(crate) fn jump_selection_to_path(root_dir: &Path, selection: &str) -> PathBuf {
    let mut target = root_dir.to_path_buf();
    let trimmed = selection.trim_end_matches('/');
    for segment in trimmed.split('/').filter(|segment| !segment.is_empty()) {
        target.push(segment);
    }
    target
}
