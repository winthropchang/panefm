//! 目前開啟目錄的跨平台檔案系統監看服務。
//!
//! 一般本機磁碟優先使用作業系統原生事件，讓 Finder 或 Explorer 的新增、刪除與
//! 改名可以快速反映到列表。部分 SMB 掛載點不會可靠送出原生事件，因此同時保留
//! 低頻 [`PollWatcher`] 作為 fallback；兩個來源最後都進入同一條 channel，再由
//! `App` 去重與延遲合併，避免一次檔案操作造成大量重複 reload。

use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use notify::{Config, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};

/// 封裝原生 watcher、輪詢 fallback 與事件接收端。
///
/// `watched_dirs` 只保存至少被一種 backend 成功監看的目錄；`native_dirs` 與
/// `poll_dirs` 分開記錄，讓其中一種 backend 不支援 SMB 時仍能由另一種接手。
pub(crate) struct FilesystemWatcher {
    native: RecommendedWatcher,
    poll: PollWatcher,
    receiver: Receiver<notify::Result<Event>>,
    watched_dirs: BTreeSet<PathBuf>,
    native_dirs: BTreeSet<PathBuf>,
    poll_dirs: BTreeSet<PathBuf>,
}

impl fmt::Debug for FilesystemWatcher {
    /// 只輸出目前監看的路徑，不洩漏 notify backend 的內部平台資源。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FilesystemWatcher")
            .field("watched_dirs", &self.watched_dirs)
            .finish_non_exhaustive()
    }
}

impl FilesystemWatcher {
    /// 建立跨平台 watcher，但尚不監看任何目錄。
    ///
    /// 參數：
    /// - `fallback_interval: Duration`，原生事件失效時，輪詢 backend 重新掃描目錄的間隔。
    ///
    /// 回傳：`notify::Result<FilesystemWatcher>`；無法建立平台 watcher 時回傳 notify
    /// 原始錯誤，呼叫端可選擇停用自動刷新而不讓整個 TUI 結束。
    pub(crate) fn new(fallback_interval: Duration) -> notify::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let native_sender = sender.clone();
        let native = notify::recommended_watcher(move |event| {
            let _ = native_sender.send(event);
        })?;
        let poll = PollWatcher::new(
            move |event| {
                let _ = sender.send(event);
            },
            Config::default()
                .with_poll_interval(fallback_interval)
                .with_compare_contents(false),
        )?;

        Ok(Self {
            native,
            poll,
            receiver,
            watched_dirs: BTreeSet::new(),
            native_dirs: BTreeSet::new(),
            poll_dirs: BTreeSet::new(),
        })
    }

    /// 讓 watcher 的監看集合與目前所有 panel 目錄一致。
    ///
    /// 參數：
    /// - `directories: I`，每個元素為一個 panel 的目前目錄；相同目錄會自動去重。
    ///
    /// 回傳：`Vec<String>`，列出完全無法由任何 backend 監看的目錄錯誤。單一 backend
    /// 失敗不算整體失敗，因為另一個 backend 仍可能正常運作。
    pub(crate) fn sync_directories<I>(&mut self, directories: I) -> Vec<String>
    where
        I: IntoIterator<Item = PathBuf>,
    {
        let desired = directories.into_iter().collect::<BTreeSet<_>>();

        for directory in self
            .native_dirs
            .difference(&desired)
            .cloned()
            .collect::<Vec<_>>()
        {
            let _ = self.native.unwatch(&directory);
            self.native_dirs.remove(&directory);
        }
        for directory in self
            .poll_dirs
            .difference(&desired)
            .cloned()
            .collect::<Vec<_>>()
        {
            let _ = self.poll.unwatch(&directory);
            self.poll_dirs.remove(&directory);
        }

        let mut errors = Vec::new();
        for directory in desired.difference(&self.watched_dirs) {
            let native_result = self.native.watch(directory, RecursiveMode::NonRecursive);
            if native_result.is_ok() {
                self.native_dirs.insert(directory.clone());
            }
            // 本機大型目錄若同時註冊 PollWatcher，`watch` 可能同步建立數萬筆快照，讓
            // 使用者剛切換目錄就卡住。網路位置仍主動保留輪詢；本機只有原生監看失敗
            // 時才 fallback，兼顧 Finder／Explorer 即時更新與 SMB 可靠性。
            let needs_poll = native_result.is_err() || is_likely_network_path(directory);
            let poll_result =
                needs_poll.then(|| self.poll.watch(directory, RecursiveMode::NonRecursive));
            if poll_result.as_ref().is_some_and(Result::is_ok) {
                self.poll_dirs.insert(directory.clone());
            }
            if let Err(native_error) = native_result {
                match poll_result {
                    Some(Err(poll_error)) => errors.push(format!(
                        "{}: native watcher: {native_error}; polling watcher: {poll_error}",
                        directory.display()
                    )),
                    None => errors.push(format!(
                        "{}: native watcher: {native_error}; polling watcher was not started",
                        directory.display()
                    )),
                    Some(Ok(())) => {}
                }
            }
        }

        // 即使兩種 backend 都暫時失敗，也記住本輪已嘗試的目錄，避免主迴圈每
        // 150ms 重試並用相同錯誤洗掉狀態列。panel 切換路徑後集合改變時會再嘗試。
        self.watched_dirs = desired;
        errors
    }

    /// 非阻塞取出目前累積的變更，並轉成需要重新載入的監看目錄集合。
    ///
    /// 參數：無。
    ///
    /// 回傳：`BTreeSet<PathBuf>`；只包含 create、modify、remove 或無法分類的變更。
    /// 單純讀取檔案產生的 access event 會被排除，避免 PaneFM 自己 preview 時反覆刷新。
    pub(crate) fn changed_directories(&self) -> BTreeSet<PathBuf> {
        let mut changed = BTreeSet::new();
        for event in self.receiver.try_iter().take(256).flatten() {
            if !event_kind_requires_reload(&event.kind) {
                continue;
            }
            for path in event.paths {
                for directory in &self.watched_dirs {
                    if path == *directory || path.parent() == Some(directory.as_path()) {
                        changed.insert(directory.clone());
                    }
                }
            }
        }
        changed
    }
}

/// 判斷路徑是否很可能位於網路 share，這類位置需要輪詢補足不可靠的原生事件。
///
/// 參數：`path: &Path`，目前 panel 的實際本機或掛載路徑。
/// 回傳：`bool`；Windows UNC 與 macOS `/Volumes` 掛載位置回傳 `true`。其他平台暫時
/// 回傳 `false`，未來支援 Linux 時可只在此擴充 mount 判斷，不必修改 watcher 主流程。
fn is_likely_network_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        text.starts_with(r"\\") || text.starts_with("//")
    }
    #[cfg(target_os = "macos")]
    {
        return path.starts_with("/Volumes");
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = path;
        false
    }
}

/// 判斷 notify 事件是否會改變檔案列表可見內容或 metadata。
///
/// 參數：`kind: &EventKind`，notify 回傳的跨平台事件種類。
/// 回傳：`bool`；create、modify、remove 與 other 為 `true`，access 為 `false`。
fn event_kind_requires_reload(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) | EventKind::Other
    )
}

#[cfg(test)]
mod tests {
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
}
