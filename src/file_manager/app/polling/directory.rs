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
        let (sort_mode, random_seed) = self
            .panes
            .get(&pane_id)
            .map(|pane| (pane.sort_mode, pane.random_seed))
            .unwrap_or((SortMode::Natural { reverse: false }, 0));
        if let Some(cached) = self.directory_entry_cache.get(&cwd).cloned()
            && let Some(pane) = self.panes.get_mut(&pane_id)
        {
            pane.replace_entries_presorted(cached, selected_path.as_deref());
        }

        let (sender, receiver) = mpsc::channel();
        let worker_cwd = cwd.clone();
        let worker_selection = selected_path.clone();
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
    /// 參數：無。回傳：`() `；第一批（< 1ms）讓 UI 立即畫出列表與響應游標，後續增量批次平滑呈現。
    pub(crate) fn poll_directory_load_jobs(&mut self) {
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
                        self.directory_entry_cache
                            .insert(event.cwd.clone(), entries.clone());
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
    }

    /// 非阻塞套用各 panel 的目錄大小快照。
    ///
    /// 參數：無，資料來自 `directory_size_jobs`。
    /// 回傳：`() `；每個 job 每幀最多處理 64 筆，避免大量小目錄拖慢鍵盤事件。
    pub(crate) fn poll_directory_size_jobs(&mut self) {
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
                for (path, bytes, complete) in updates {
                    pane.update_directory_size(&path, bytes, complete);
                }
            }
            if finished || !cwd_matches {
                self.cancel_directory_size_scan(pane_id);
            }
        }
    }
}
