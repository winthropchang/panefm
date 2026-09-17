use super::super::*;

impl App {
    pub(crate) fn poll_update_check(&mut self) {
        let Some(rx) = &self.update_check_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => {
                if let crate::updater::UpdateCheckResult::UpdateAvailable {
                    latest_version,
                    download_url,
                    asset_name,
                    ..
                } = result
                {
                    self.update_badge_info = Some(UpdateBadgeInfo {
                        latest_version,
                        download_url,
                        asset_name,
                    });
                }
                self.update_check_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_check_rx = None;
            }
        }
    }

    /// 輪詢就地升級工作進度與結果。
    pub(crate) fn poll_in_app_update(&mut self) {
        let Some(rx) = &self.in_app_update_rx else {
            return;
        };
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(InAppUpdateMsg::Progress { downloaded, total }) => {
                    let dl_mb = downloaded as f64 / (1024.0 * 1024.0);
                    if let Some(total_bytes) = total {
                        let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
                        let percent =
                            ((downloaded as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0);
                        self.status = format!(
                            "⏳ 正在下載更新: {percent:.1}% ({dl_mb:.1} MB / {total_mb:.1} MB)..."
                        );
                    } else {
                        self.status = format!("⏳ 正在下載更新: {dl_mb:.1} MB...");
                    }
                }
                Ok(InAppUpdateMsg::Completed(Ok(latest_version))) => {
                    self.in_app_updating = false;
                    self.in_app_update_rx = None;
                    self.update_badge_info = None;
                    self.status = format!("✅ 成功升級至 v{latest_version}！請重啟 panefm 生效。");
                    return;
                }
                Ok(InAppUpdateMsg::Completed(Err(err))) => {
                    self.in_app_updating = false;
                    self.in_app_update_rx = None;
                    if err.contains("無須更新") || err.contains("無更新") {
                        self.update_badge_info = None;
                        self.status = format!("ℹ️ {err}");
                    } else {
                        self.status = format!("❌ 升級失敗: {err}");
                    }
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            self.in_app_updating = false;
            self.in_app_update_rx = None;
        }
    }

    /// 在 TUI 內部觸發就地升級程序。
    pub(crate) fn trigger_in_app_update(&mut self) {
        if self.in_app_updating {
            self.status = String::from("⏳ 正在下載並安裝更新，請稍候...");
            return;
        }

        let download_info = self.update_badge_info.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.in_app_update_rx = Some(rx);
        self.in_app_updating = true;
        self.status = String::from("🚀 正在連線檢查並下載最新版本...");

        std::thread::spawn(move || {
            let do_download = |url: &str| {
                let tx_p = tx.clone();
                crate::updater::download_and_install_with_progress(
                    url,
                    crate::updater::DEFAULT_DOWNLOAD_TIMEOUT_SECS,
                    move |downloaded, total| {
                        let _ = tx_p.send(InAppUpdateMsg::Progress { downloaded, total });
                    },
                )
            };

            let valid_info = download_info.filter(|info| {
                let url = info.download_url.trim();
                url.starts_with("http://") || url.starts_with("https://")
            });

            let result = if let Some(info) = valid_info {
                match do_download(&info.download_url) {
                    Ok(()) => {
                        let now_secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let cache = crate::updater::UpdateStateCache {
                            last_check_timestamp: now_secs,
                            latest_version: info.latest_version.clone(),
                            asset_name: info.asset_name,
                            download_url: info.download_url,
                        };
                        let _ = crate::updater::save_update_cache(&cache, None);
                        Ok(info.latest_version)
                    }
                    Err(err) => {
                        // 若原下載網址失效，嘗試重新連線 GitHub 取得最新版本下載資訊
                        match crate::updater::check_for_update(8) {
                            Ok(crate::updater::UpdateCheckResult::UpdateAvailable {
                                latest_version,
                                download_url,
                                asset_name,
                                ..
                            }) => match do_download(&download_url) {
                                Ok(()) => {
                                    let now_secs = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    let cache = crate::updater::UpdateStateCache {
                                        last_check_timestamp: now_secs,
                                        latest_version: latest_version.clone(),
                                        asset_name,
                                        download_url,
                                    };
                                    let _ = crate::updater::save_update_cache(&cache, None);
                                    Ok(latest_version)
                                }
                                Err(second_err) => Err(second_err.to_string()),
                            },
                            _ => Err(err.to_string()),
                        }
                    }
                }
            } else {
                // 快取中沒有有效的下載網址（例如手動模擬測試或快取缺乏 url），向 GitHub 查詢真實發布資訊
                match crate::updater::check_for_update(8) {
                    Ok(crate::updater::UpdateCheckResult::UpdateAvailable {
                        latest_version,
                        download_url,
                        asset_name,
                        ..
                    }) => match do_download(&download_url) {
                        Ok(()) => {
                            let now_secs = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            let cache = crate::updater::UpdateStateCache {
                                last_check_timestamp: now_secs,
                                latest_version: latest_version.clone(),
                                asset_name,
                                download_url,
                            };
                            let _ = crate::updater::save_update_cache(&cache, None);
                            Ok(latest_version)
                        }
                        Err(err) => Err(err.to_string()),
                    },
                    Ok(crate::updater::UpdateCheckResult::UpToDate { current_version }) => {
                        let now_secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let cache = crate::updater::UpdateStateCache {
                            last_check_timestamp: now_secs,
                            latest_version: current_version.clone(),
                            asset_name: String::new(),
                            download_url: String::new(),
                        };
                        let _ = crate::updater::save_update_cache(&cache, None);
                        Err(format!(
                            "遠端 GitHub 目前版本為 v{current_version}，無更新版本可用（已同步校正本機快取）。"
                        ))
                    }
                    Err(err) => Err(err.to_string()),
                }
            };
            let _ = tx.send(InAppUpdateMsg::Completed(result));
        });
    }
}
