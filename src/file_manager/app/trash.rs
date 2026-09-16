use std::io;

use chrono::{DateTime, Local};

use super::{PanelSearchState, PendingAction, fuzzy_matched_indices_by_fields};
use crate::file_manager::trash::{TrashListEntry, TrashStore};
use crate::file_manager::ui::TrashPanelLine;

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

/// 將 unix 毫秒時間轉成較容易閱讀的本地時間字串。
pub(crate) fn format_deleted_at(unix_ms: u64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + std::time::Duration::from_millis(unix_ms))
        .format("%m/%d %H:%M")
        .to_string()
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
