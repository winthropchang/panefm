use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use super::fuzzy_matched_indices_by_fields;
use crate::file_manager::ui::TaskPanelLine;

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
