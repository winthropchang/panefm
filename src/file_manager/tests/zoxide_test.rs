use std::{fs, path::PathBuf, sync::mpsc};

use tempfile::tempdir;

use super::{
    ZoxideTracker, add_directory_to_zoxide_with_data_dir, preferred_zoxide_data_dir,
    query_zoxide_directories_with_data_dir, zoxide_data_dir,
};

#[test]
/// 驗證每個平台都能解析出非空的 zoxide 資料目錄。
/// 保護目的：避免 zoxide 資料路徑或背景佇列調整後，阻塞目錄切換或遺失既有學習資料。
fn zoxide_data_dir_is_not_empty() {
    let data_dir = zoxide_data_dir().expect("data dir");
    assert!(!data_dir.as_os_str().is_empty());
}

#[test]
/// 驗證改名後仍會沿用既有 zoxide 資料，新資料存在時則優先使用 PaneFM 目錄。
/// 保護目的：避免 zoxide 資料路徑或背景佇列調整後，阻塞目錄切換或遺失既有學習資料。
fn preferred_zoxide_data_dir_preserves_legacy_learning_data() {
    let dir = tempdir().expect("tempdir");
    let legacy = dir.path().join("terminal-file-manager").join("zoxide");
    fs::create_dir_all(&legacy).expect("legacy zoxide data");

    assert_eq!(preferred_zoxide_data_dir(dir.path()), legacy);

    let current = dir.path().join("panefm").join("zoxide");
    fs::create_dir_all(&current).expect("current zoxide data");
    assert_eq!(preferred_zoxide_data_dir(dir.path()), current);
}

#[test]
/// 驗證加入測試目錄後，後續查詢可以依 frecency 回傳同一路徑。
/// 保護目的：避免 zoxide 資料路徑或背景佇列調整後，阻塞目錄切換或遺失既有學習資料。
fn add_then_query_returns_tracked_directory() {
    let dir = tempdir().expect("tempdir");
    let data_dir = dir.path().join("zoxide-data");
    add_directory_to_zoxide_with_data_dir(dir.path(), &data_dir).expect("add directory");
    let results = query_zoxide_directories_with_data_dir(&data_dir).expect("query directories");
    assert!(
        results.iter().any(|path| path == dir.path()),
        "expected zoxide results to contain {}",
        dir.path().display()
    );
}

#[test]
/// 驗證背景佇列已滿時，提交瀏覽目錄會直接略過，而不是等待 worker 造成介面卡頓。
///
/// 參數：無。
/// 回傳：無；若 `track()` 因滿佇列阻塞，測試會無法完成。
/// 保護目的：避免 zoxide 資料路徑或背景佇列調整後，阻塞目錄切換或遺失既有學習資料。
fn tracker_drops_updates_when_queue_is_full() {
    let (sender, _receiver) = mpsc::sync_channel(1);
    sender
        .send(PathBuf::from("already-queued"))
        .expect("fill queue");
    let tracker = ZoxideTracker {
        sender: Some(sender),
    };

    tracker.track(PathBuf::from("must-not-block").as_path());
}
