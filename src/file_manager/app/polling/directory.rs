use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

use super::super::*;

impl App {
    /// 為指定 panel 啟動非同步遞迴目錄大小掃描。
    ///
    /// 參數：`pane_id: usize`，要顯示 `ms` 大小的 panel 編號。
    /// 回傳：`() `；panel 不存在或已在計算相同目錄時不重複重啟，避免複製或頻繁變更時數字反覆歸零跳動。
    pub(crate) fn start_directory_size_scan(&mut self, pane_id: usize) {
        let (cwd, directories) = {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                return;
            };
            if self
                .directory_size_jobs
                .get(&pane_id)
                .is_some_and(|job| job.cwd == pane.cwd)
            {
                return;
            }
            pane.init_directory_sizes_if_missing();
            let cwd = pane.cwd.clone();
            let directories = pane
                .entries
                .iter()
                .filter(|entry| entry.is_dir)
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>();
            (cwd, directories)
        };
        self.cancel_directory_size_scan(pane_id);
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            scan_directory_sizes(directories, &worker_cancelled, &sender);
            let _ = sender.send(DirectorySizeEvent::Done);
        });
        self.directory_size_jobs.insert(
            pane_id,
            DirectorySizeJob {
                cwd,
                receiver,
                cancelled,
            },
        );
    }

    /// 在 panel 成功切換目錄後，同步目前目錄容量工作的生命週期。
    ///
    /// 參數：
    /// - `pane_id: usize`，剛完成目錄切換的 panel 編號。
    /// - `previous_cwd: &Path`，切換前的工作目錄，用來避免選到一般檔案時無謂重啟。
    ///
    /// 回傳：`() `。只有工作目錄真的改變才處理；`linemode size` 或 size 排序啟用時，
    /// 會取消舊目錄工作並立即替新列表填入 `~0B`、啟動新掃描。其他顯示模式則只取消
    /// 可能殘留的舊工作，避免背景執行緒繼續走訪已離開的目錄。
    pub(crate) fn restart_directory_size_scan_after_navigation(
        &mut self,
        pane_id: usize,
        previous_cwd: &Path,
    ) {
        let is_size_detail = {
            let Some(pane) = self.panes.get_mut(&pane_id) else {
                self.cancel_directory_size_scan(pane_id);
                return;
            };
            if pane.cwd == previous_cwd {
                return;
            }
            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                pane.clear_directory_sizes();
                true
            } else {
                false
            }
        };
        self.cancel_directory_size_scan(pane_id);
        if is_size_detail {
            self.start_directory_size_scan(pane_id);
        }
    }

    /// 取消指定 panel 尚未完成的大小掃描並丟棄其接收端。
    ///
    /// 參數：`pane_id: usize`，要停止掃描的 panel 編號。
    /// 回傳：`() `；沒有工作時安全地不做任何事。
    pub(crate) fn cancel_directory_size_scan(&mut self, pane_id: usize) {
        if let Some(job) = self.directory_size_jobs.remove(&pane_id) {
            job.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// 啟動單一 panel 的非阻塞目錄讀取，並先套用已有快取。
    ///
    /// 參數：`pane_id` 是導航來源 panel；`cwd` 是新目錄；`selected_path` 是回到父目錄
    /// 時應重新選取的子目錄。回傳：`() `；I/O 結果由 `poll_directory_load_jobs` 套用。
    /// 檢查目錄快取是否仍然新鮮（在指定的 max_age 時間內）。
    pub(crate) fn is_directory_cache_fresh(
        &self,
        path: &Path,
        max_age: std::time::Duration,
    ) -> bool {
        if !self.directory_entry_cache.contains_key(path) {
            return false;
        }
        self.directory_cache_timestamps
            .get(path)
            .is_some_and(|timestamp| timestamp.elapsed() < max_age)
    }

    /// 將成功讀取的目錄清單存入快取並記錄時間戳與 LRU 限制。
    pub(crate) fn store_directory_cache(
        &mut self,
        path: PathBuf,
        entries: Vec<crate::file_manager::entry::FileEntry>,
    ) {
        if entries.is_empty() {
            return;
        }
        self.directory_entry_cache.insert(path.clone(), entries);
        self.directory_cache_timestamps
            .insert(path, std::time::Instant::now());
        self.prune_directory_cache_if_needed();
    }

    /// 主動讓指定目錄的快取失效（由 Watcher 或檔案變更調用）。
    pub(crate) fn invalidate_directory_cache(&mut self, path: &Path) {
        self.directory_entry_cache.remove(path);
        self.directory_cache_timestamps.remove(path);
        self.directory_cache_cursors.remove(path);
    }

    /// 依據條件保留快取項目，同步清理過期路徑。
    pub(crate) fn retain_directory_cache<F>(&mut self, mut predicate: F)
    where
        F: FnMut(&PathBuf) -> bool,
    {
        self.directory_entry_cache.retain(|path, _| predicate(path));
        self.directory_cache_timestamps
            .retain(|path, _| predicate(path));
        self.directory_cache_cursors
            .retain(|path, _| predicate(path));
    }

    /// 限制目錄快取最大容量（預設 64 個目錄），淘汰最舊的項目以防記憶體無界膨脹。
    pub(crate) fn prune_directory_cache_if_needed(&mut self) {
        const MAX_CACHED_DIRECTORIES: usize = 64;
        if self.directory_cache_timestamps.len() > MAX_CACHED_DIRECTORIES
            && let Some((oldest_path, _)) = self
                .directory_cache_timestamps
                .iter()
                .min_by_key(|(_, ts)| **ts)
                .map(|(p, ts)| (p.clone(), *ts))
        {
            self.directory_entry_cache.remove(&oldest_path);
            self.directory_cache_timestamps.remove(&oldest_path);
            self.directory_cache_cursors.remove(&oldest_path);
        }
    }

    /// 為指定 panel 啟動非同步目錄讀取工作，若目標處於新鮮快取中則直接 0ms 瞬間就緒。
    pub(crate) fn start_directory_load(
        &mut self,
        pane_id: usize,
        cwd: PathBuf,
        selected_path: Option<PathBuf>,
    ) {
        self.cancel_directory_size_scan(pane_id);
        self.cancel_directory_load(pane_id);
        if self.config.ui.vcs.enabled {
            self.vcs_manager.request_query(pane_id, cwd.clone());
        }

        let target_selected =
            selected_path.or_else(|| self.directory_cache_cursors.get(&cwd).cloned().flatten());

        // 目錄穿梭 0ms 瞬間響應：在 5 秒 TTL 內重訪目錄直接命中快取，完全跳過背景 Worker 與硬碟 I/O
        if self.is_directory_cache_fresh(&cwd, std::time::Duration::from_secs(5))
            && let Some(cached) = self.directory_entry_cache.get(&cwd).cloned()
            && let Some(pane) = self.panes.get_mut(&pane_id)
        {
            pane.replace_entries_presorted(cached, target_selected.as_deref());
            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                pane.init_directory_sizes_if_missing();
                self.start_directory_size_scan(pane_id);
            }
            self.status = format!("opened directory: {}", cwd.display());
            return;
        }

        let (sort_mode, random_seed) = self
            .panes
            .get(&pane_id)
            .map(|pane| (pane.sort_mode, pane.random_seed))
            .unwrap_or((SortMode::Natural { reverse: false }, 0));
        if let Some(cached) = self.directory_entry_cache.get(&cwd).cloned()
            && let Some(pane) = self.panes.get_mut(&pane_id)
        {
            pane.replace_entries_presorted(cached, target_selected.as_deref());
        }

        let (sender, receiver) = mpsc::channel();
        let worker_cwd = cwd.clone();
        let worker_selection = target_selected;
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        thread::spawn(move || {
            let thread_pane_id = pane_id;
            let thread_cwd = worker_cwd.clone();
            let thread_selection = worker_selection.clone();
            let thread_sender = sender.clone();
            let stream_result = crate::file_manager::pane::stream_dir_entries_with_cancellation(
                &worker_cwd,
                sort_mode,
                random_seed,
                &worker_cancelled,
                move |progress| {
                    thread_sender
                        .send(DirectoryLoadEvent {
                            pane_id: thread_pane_id,
                            cwd: thread_cwd.clone(),
                            selected_path: thread_selection.clone(),
                            result: Ok(progress),
                        })
                        .is_ok()
                },
            );
            if let Err(error) = stream_result {
                let _ = sender.send(DirectoryLoadEvent {
                    pane_id,
                    cwd: worker_cwd,
                    selected_path: worker_selection,
                    result: Err(error),
                });
            }
        });
        self.directory_load_jobs.insert(
            pane_id,
            DirectoryLoadJob {
                cwd: cwd.clone(),
                receiver,
                cancelled,
            },
        );
        self.status = format!("loading directory: {}", cwd.display());
    }

    /// 取消指定 panel 尚未完成的目錄載入工作。
    ///
    /// 參數：`pane_id: usize`，要停止載入的 panel 編號。
    /// 回傳：`() `；沒有工作時安全地不做任何事。
    pub(crate) fn cancel_directory_load(&mut self, pane_id: usize) {
        if let Some(job) = self.directory_load_jobs.remove(&pane_id) {
            job.cancelled.store(true, Ordering::Relaxed);
        }
    }

    /// 非阻塞接收大型目錄清單；分批套用快速發現項目，並在完成時更新快取與 size linemode。
    ///
    /// 參數：無。回傳：`bool`；若有任何目錄資料載入並更新畫面則回傳 `true`。
    pub(crate) fn poll_directory_load_jobs(&mut self) -> bool {
        let mut changed = false;
        let pane_ids = self.directory_load_jobs.keys().copied().collect::<Vec<_>>();
        for pane_id in pane_ids {
            let mut job_done = false;
            let Some((job_cwd, events)) = self
                .directory_load_jobs
                .get(&pane_id)
                .map(|job| (job.cwd.clone(), job.receiver.try_iter().collect::<Vec<_>>()))
            else {
                continue;
            };
            if !events.is_empty() {
                changed = true;
            }
            for event in events {
                if event.cwd != job_cwd {
                    continue;
                }
                match event.result {
                    Ok(DirectoryLoadProgress::Batch {
                        entries,
                        is_first_chunk,
                    }) => {
                        let target_pane_id = if self
                            .panes
                            .get(&event.pane_id)
                            .is_some_and(|p| p.cwd == event.cwd)
                        {
                            Some(event.pane_id)
                        } else {
                            self.panes
                                .iter()
                                .find(|(_, p)| p.cwd == event.cwd)
                                .map(|(&id, _)| id)
                        };
                        if let Some(target_id) = target_pane_id
                            && let Some(pane) = self.panes.get_mut(&target_id)
                        {
                            if is_first_chunk {
                                pane.replace_entries_presorted(
                                    entries,
                                    event.selected_path.as_deref(),
                                );
                            } else {
                                pane.extend_entries(entries);
                            }
                            // `ms` 可能在目錄清單尚未載入完成時就已啟用。每批新加入的
                            // 目錄都要立刻取得部分容量狀態，否則右側欄位會一直空白，直到
                            // 完整清單載入並重新啟動容量掃描後才第一次出現內容。
                            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                                pane.init_directory_sizes_if_missing();
                            }
                            self.status = format!(
                                "loading directory: {} ({} items)",
                                event.cwd.display(),
                                pane.entries.len()
                            );
                        }
                    }
                    Ok(DirectoryLoadProgress::Complete(entries)) => {
                        job_done = true;
                        self.store_directory_cache(event.cwd.clone(), entries.clone());
                        let mut restart_size_scan = false;
                        if let Some(pane) = self.panes.get_mut(&event.pane_id)
                            && pane.cwd == event.cwd
                        {
                            pane.replace_entries_presorted(entries, event.selected_path.as_deref());
                            if matches!(pane.active_detail_kind(), SortDetailKind::Size) {
                                pane.init_directory_sizes_if_missing();
                                restart_size_scan = true;
                            }
                            self.status = format!("opened directory: {}", event.cwd.display());
                        }
                        if restart_size_scan {
                            // 載入期間可能已有只包含首批目錄的同 cwd 掃描。先取消舊工作
                            // 才能確保完整清單中的每個直接子目錄都會被納入新一輪計算。
                            self.cancel_directory_size_scan(event.pane_id);
                            self.start_directory_size_scan(event.pane_id);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                        job_done = true;
                    }
                    Err(error) => {
                        job_done = true;
                        if self
                            .panes
                            .get(&event.pane_id)
                            .is_some_and(|pane| pane.cwd == event.cwd)
                        {
                            self.status = format!("open directory failed: {error}");
                        }
                    }
                }
            }
            if job_done {
                self.directory_load_jobs.remove(&pane_id);
            }
        }
        changed
    }

    /// 非阻塞套用各 panel 的目錄大小快照。
    ///
    /// 參數：無，資料來自 `directory_size_jobs`。
    /// 回傳：`bool`；若有任何目錄大小被更新則回傳 `true`。
    pub(crate) fn poll_directory_size_jobs(&mut self) -> bool {
        let mut changed = false;
        let pane_ids = self.directory_size_jobs.keys().copied().collect::<Vec<_>>();
        for pane_id in pane_ids {
            let mut finished = false;
            let mut updates = Vec::new();
            let Some(job) = self.directory_size_jobs.get(&pane_id) else {
                continue;
            };
            let job_cwd = job.cwd.clone();
            for event in job.receiver.try_iter().take(64) {
                match event {
                    DirectorySizeEvent::Update {
                        path,
                        bytes,
                        complete,
                    } => updates.push((path, bytes, complete)),
                    DirectorySizeEvent::Done => finished = true,
                }
            }
            let cwd_matches = self
                .panes
                .get(&pane_id)
                .is_some_and(|pane| pane.cwd == job_cwd);
            if cwd_matches && let Some(pane) = self.panes.get_mut(&pane_id) {
                if !updates.is_empty() {
                    changed = true;
                }
                for (path, bytes, complete) in updates {
                    pane.update_directory_size(&path, bytes, complete);
                }
            }
            if finished || !cwd_matches {
                self.cancel_directory_size_scan(pane_id);
            }
        }
        changed
    }
}
