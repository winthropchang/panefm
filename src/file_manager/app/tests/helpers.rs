use super::*;

pub(crate) static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// 建立測試專用預設設定，避免每個 App 案例重複準備 config 與來源路徑。
pub(crate) fn default_loaded_config() -> LoadedConfig {
    LoadedConfig {
        config: AppConfig::default(),
        source: None,
        base_dir: std::path::PathBuf::new(),
    }
}

/// 輪詢測試中的背景搜尋直到完成，並設定 timeout 防止失敗時無限等待。
pub(crate) fn wait_for_global_search(app: &mut App) {
    for _ in 0..300 {
        app.poll_background_tasks();
        if app
            .global_search
            .as_ref()
            .is_some_and(|search| search.searched && !search.loading)
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("global search did not complete in time");
}

/// 輪詢測試中的大型檔案工作直到全部完成，避免測試直接依賴執行緒排程速度。
///
/// 保護目的：paste/compress/extract 已移出主執行緒；測試必須驗證 task 完成事件
/// 確實回到 App，而不是以固定 sleep 掩蓋偶發競態。
pub(crate) fn wait_for_file_jobs(app: &mut App) {
    for _ in 0..200 {
        app.poll_background_tasks();
        if app.file_job_receivers.is_empty() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("background file job did not complete in time");
}
