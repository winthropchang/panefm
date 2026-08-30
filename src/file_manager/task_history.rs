//! 背景 task 歷史的跨 session 持久化。
//!
//! `App` 只負責建立與更新 task；本模組負責決定檔案位置、JSON 格式與安全寫入。
//! 寫檔採用同目錄暫存檔再更名，避免程式在序列化途中結束時破壞上一份完整紀錄。

use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[cfg(not(test))]
use std::env;

use serde::{Deserialize, Serialize};

use super::app::TaskRecord;

/// task 歷史檔目前使用的格式版本，未來欄位演進時可據此做相容轉換。
const TASK_HISTORY_VERSION: u32 = 1;

/// `task-history.json` 最外層格式。
///
/// 使用物件而不是直接序列化陣列，是為了未來加入 session 或恢復資訊時，仍可辨識
/// 舊格式並提供清楚的升級路徑。
#[derive(Debug, Serialize, Deserialize)]
struct TaskHistoryFile {
    version: u32,
    tasks: Vec<TaskRecord>,
}

/// 決定 task 歷史檔案位置。
///
/// 參數：
/// - `cwd: &Path`，PaneFM 啟動時的工作目錄。
/// - `config_source: Option<&Path>`，本次實際讀取的 `config.toml`。
///
/// 回傳：`PathBuf`。有設定檔時放在設定檔旁，否則放在啟動目錄。
pub(crate) fn task_history_file_path(cwd: &Path, config_source: Option<&Path>) -> PathBuf {
    if let Some(parent) = config_source.and_then(Path::parent) {
        return parent.join("task-history.json");
    }

    // 單元測試不可共用開發者家目錄中的真實歷史，否則平行測試會互相覆寫；隱藏的
    // `.tfm` 也不會污染測試中的一般檔名搜尋結果。
    #[cfg(test)]
    return cwd.join(".tfm").join("task-history.json");

    #[cfg(not(test))]
    {
        if let Some(state_dir) = env::var_os("PANEFM_STATE_DIR") {
            return PathBuf::from(state_dir).join("task-history.json");
        }
        if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(config_home)
                .join("panefm")
                .join("task-history.json");
        }
        if let Some(app_data) = env::var_os("APPDATA") {
            return PathBuf::from(app_data)
                .join("panefm")
                .join("task-history.json");
        }
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".config")
                .join("panefm")
                .join("task-history.json");
        }
        cwd.join(".tfm").join("task-history.json")
    }
}

/// 從磁碟載入 task 歷史。
///
/// 參數：`path: &Path`，`task-history.json` 的完整路徑。
/// 回傳：`io::Result<Vec<TaskRecord>>`；檔案不存在時視為第一次啟動並回傳空清單，
/// JSON 損壞或版本不支援時回傳可顯示給使用者的錯誤。
pub(crate) fn load_task_history(path: &Path) -> io::Result<Vec<TaskRecord>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let history: TaskHistoryFile = serde_json::from_str(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if history.version != TASK_HISTORY_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported task history version: {}", history.version),
        ));
    }
    Ok(history.tasks)
}

/// 將目前 task 歷史完整寫回磁碟。
///
/// 參數：
/// - `path: &Path`，目標 `task-history.json`。
/// - `tasks: &[TaskRecord]`，最多由 `App` 保留的歷史項目。
///
/// 回傳：`io::Result<()>`。成功代表 JSON 已完整取代舊檔；失敗時舊檔盡可能保留。
pub(crate) fn save_task_history(path: &Path, tasks: &[TaskRecord]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_vec_pretty(&TaskHistoryFile {
        version: TASK_HISTORY_VERSION,
        tasks: tasks.to_vec(),
    })
    .map_err(io::Error::other)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, content)?;

    // Unix 可直接以 rename 原子取代；Windows 不允許覆蓋既有檔案，因此先刪除舊檔。
    // 暫存檔仍確保序列化失敗時不會先破壞上一版紀錄。
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::file_manager::app::TaskState;

    /// 驗證 task 歷史可完整保存並重新載入所有狀態、時間與百分比。
    ///
    /// 保護目的：task 歷史是長時間 SMB copy 的診斷依據；若序列化漏掉任一欄位，
    /// 使用者重開 PaneFM 後就無法判斷工作何時開始、何時結束或停在哪個進度。
    #[test]
    fn task_history_round_trip_preserves_diagnostic_fields() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("task-history.json");
        let tasks = vec![TaskRecord {
            id: 7,
            pane_id: 2,
            kind: String::from("paste"),
            title: String::from("copy archive.zip"),
            detail: String::from("destination: share"),
            state: TaskState::Done,
            progress_percent: Some(100),
            started_at_unix_ms: 1_700_000_000_000,
            finished_at_unix_ms: Some(1_700_000_001_000),
        }];

        save_task_history(&path, &tasks).expect("save history");
        let loaded = load_task_history(&path).expect("load history");

        assert_eq!(loaded, tasks);
    }

    /// 驗證沒有歷史檔時會回傳空清單，而不是阻止 PaneFM 第一次啟動。
    ///
    /// 保護目的：`task-history.json` 是執行期資料，不應要求使用者預先建立檔案。
    #[test]
    fn missing_task_history_is_treated_as_empty() {
        let dir = tempdir().expect("tempdir");
        let loaded = load_task_history(&dir.path().join("missing.json")).expect("load history");

        assert!(loaded.is_empty());
    }

    /// 驗證歷史檔預設與實際設定檔放在同一目錄。
    ///
    /// 保護目的：macOS 與 Windows 的設定路徑不同，固定由設定檔推導才能讓使用者在
    /// 兩個平台都能找到 task 歷史，而不會誤寫到當前瀏覽的目錄。
    #[test]
    fn task_history_path_follows_loaded_config() {
        let cwd = Path::new("/workspace/project");
        let config = Path::new("/settings/panefm/config.toml");

        assert_eq!(
            task_history_file_path(cwd, Some(config)),
            PathBuf::from("/settings/panefm/task-history.json")
        );
        assert_eq!(
            task_history_file_path(cwd, None),
            PathBuf::from("/workspace/project/.tfm/task-history.json")
        );
    }
}
