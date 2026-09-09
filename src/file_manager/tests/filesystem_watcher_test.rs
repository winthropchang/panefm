use std::time::{Duration, Instant};

use notify::{EventKind, event::AccessKind};
use tempfile::tempdir;

use super::{FilesystemWatcher, event_kind_requires_reload, is_likely_network_path};

#[test]
/// 驗證單純讀取檔案不會被當成列表變更，但新增、修改與刪除都會要求刷新。
/// 保護目的：避免 preview 讀取檔案觸發 watcher 後形成「reload -> read -> reload」迴圈。
fn access_events_do_not_request_directory_reload() {
    assert!(!event_kind_requires_reload(&EventKind::Access(
        AccessKind::Read
    )));
    assert!(event_kind_requires_reload(&EventKind::Create(
        notify::event::CreateKind::File
    )));
    assert!(event_kind_requires_reload(&EventKind::Modify(
        notify::event::ModifyKind::Any
    )));
    assert!(event_kind_requires_reload(&EventKind::Remove(
        notify::event::RemoveKind::File
    )));
}

#[test]
/// 驗證外部程式在監看目錄建立檔案後，跨平台 watcher 會回報該目錄。
/// 保護目的：這正是 Finder／Explorer 修改完成後 PaneFM 自動更新所依賴的完整事件鏈。
fn external_file_creation_reports_watched_directory() {
    let directory = tempdir().expect("tempdir");
    let watched_path = directory.path().canonicalize().expect("canonical tempdir");
    let mut watcher = FilesystemWatcher::new(Duration::from_millis(50)).expect("watcher");
    assert!(watcher.sync_directories([watched_path.clone()]).is_empty());
    // macOS FSEvents 在剛註冊 watcher 的極短窗口內可能尚未開始送事件；先讓 backend
    // 完成啟動，測試才能驗證真正的外部建立，而不是依賴 PollWatcher 補救競態。
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(watched_path.join("created-outside.txt"), "content")
        .expect("create external file");

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if watcher.changed_directories().contains(&watched_path) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("watcher did not report external file creation");
}

#[test]
/// 驗證本機大型目錄不會被誤判為網路位置，避免註冊輪詢 watcher 時同步掃描全部項目。
///
/// 保護目的：目錄非阻塞載入完成後，若 watcher 又在主執行緒建立數萬筆 snapshot，
/// 使用者仍會感覺 TUI 卡住；本測試固定一般本機路徑只能使用原生 watcher。
fn local_directory_does_not_require_polling_fallback() {
    let directory = tempdir().expect("tempdir");
    assert!(!is_likely_network_path(directory.path()));
}
