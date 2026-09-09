use tempfile::tempdir;

use super::*;
use crate::file_manager::app::TaskState;

/// 驗證 task 歷史可完整保存並重新載入狀態、時間、byte 進度、來源與目的地。
///
/// 保護目的：task 歷史是長時間 SMB copy 的診斷依據；若序列化漏掉任一欄位，
/// 使用者重開 PaneFM 後就無法判斷工作何時開始、資料從哪裡來、寫到哪裡或停在
/// 哪個進度。
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
        source_locations: vec![String::from("/source/archive.zip")],
        destination_location: Some(String::from("/destination")),
        state: TaskState::Done,
        progress_percent: Some(100),
        completed_bytes: Some(1_024),
        total_bytes: Some(1_024),
        started_at_unix_ms: 1_700_000_000_000,
        finished_at_unix_ms: Some(1_700_000_001_000),
    }];

    save_task_history(&path, &tasks).expect("save history");
    let loaded = load_task_history(&path).expect("load history");

    assert_eq!(loaded, tasks);
}

/// 驗證舊版只有百分比的 task 歷史仍可載入。
///
/// 保護目的：新增 byte 欄位不能讓使用者既有 `task-history.json` 變成啟動錯誤；
/// 舊紀錄可顯示無 byte 資料，新工作則會保存完整 byte。
#[test]
fn legacy_task_history_without_byte_fields_remains_readable() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("task-history.json");
    fs::write(
            &path,
            r#"{"version":1,"tasks":[{"id":1,"pane_id":1,"kind":"paste","title":"copy","detail":"destination","state":"running","progress_percent":42,"started_at_unix_ms":1,"finished_at_unix_ms":null}]}"#,
        )
        .expect("legacy history");

    let tasks = load_task_history(&path).expect("load legacy history");
    assert_eq!(tasks[0].progress_percent, Some(42));
    assert_eq!(tasks[0].completed_bytes, None);
    assert_eq!(tasks[0].total_bytes, None);
    assert!(tasks[0].source_locations.is_empty());
    assert_eq!(tasks[0].destination_location, None);
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
