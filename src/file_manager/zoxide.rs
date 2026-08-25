//! zoxide 常用目錄學習、查詢與非阻塞更新佇列。
//!
//! 使用者每次正常進入目錄時，`App` 只嘗試把路徑送進 bounded channel；背景 worker
//! 才執行 `zoxide add`。佇列滿時寧可丟棄單次紀錄，也不能讓目錄切換等待外部程序。

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{SyncSender, TrySendError},
};

#[cfg(not(test))]
use std::{sync::mpsc, thread};

use anyhow::{Context, Result};

use super::tools::{find_system_command, missing_tool_message};

/// 在背景依序把瀏覽過的目錄送進 zoxide，避免目錄切換被外部程式啟動時間阻塞。
///
/// `SyncSender<PathBuf>` 使用有上限的佇列；若使用者移動速度超過背景處理速度，
/// 寧可略過一次重複學習，也不能讓 TUI 主執行緒停下來等待。
#[derive(Debug, Clone)]
pub(crate) struct ZoxideTracker {
    sender: Option<SyncSender<PathBuf>>,
}

impl ZoxideTracker {
    /// 建立專用的 zoxide 背景 worker。
    ///
    /// 參數：無。
    /// 回傳：`ZoxideTracker`，可透過 `track()` 非阻塞地提交目錄。
    pub(crate) fn new() -> Self {
        #[cfg(test)]
        {
            // App 測試需要立即查詢剛加入的資料；正式版本仍使用下方背景 worker。
            return Self { sender: None };
        }

        #[cfg(not(test))]
        let (sender, receiver) = mpsc::sync_channel::<PathBuf>(64);
        #[cfg(not(test))]
        thread::spawn(move || {
            while let Ok(path) = receiver.recv() {
                let _ = add_directory_to_zoxide(&path);
            }
        });
        #[cfg(not(test))]
        return Self {
            sender: Some(sender),
        };
    }

    /// 非阻塞地排入一個瀏覽過的目錄。
    ///
    /// 參數：
    /// - `path: &Path`，剛完成切換的目錄。
    ///
    /// 回傳：`() `；佇列暫滿或 worker 已結束時會略過，不影響檔案操作。
    pub(crate) fn track(&self, path: &Path) {
        if let Some(sender) = &self.sender {
            match sender.try_send(path.to_path_buf()) {
                Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
            }
        } else {
            #[cfg(test)]
            let _ = add_directory_to_zoxide(path);
        }
    }
}

/// 取得系統安裝的 `zoxide` 命令；本程式不再攜帶第三方 binary。
pub(crate) fn zoxide_command() -> Result<OsString> {
    find_system_command("zoxide")
        .ok_or_else(|| anyhow::anyhow!("{}", missing_tool_message("zoxide")))
}

/// 回傳 PaneFM 專屬的 zoxide 資料目錄。
///
/// 這裡會交給 `_ZO_DATA_DIR` 使用，讓 zoxide 的學習資料與使用者系統 shell 分開，
/// 避免互相污染，也讓 app 打包後可以獨立搬移與測試。
pub(crate) fn zoxide_data_dir() -> Result<PathBuf> {
    #[cfg(test)]
    {
        let thread_id = format!("{:?}", std::thread::current().id())
            .replace("ThreadId(", "")
            .replace(')', "");
        return Ok(std::env::temp_dir()
            .join("panefm-tests")
            .join("zoxide")
            .join(thread_id));
    }

    #[cfg(not(test))]
    let data_root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library").join("Application Support"))
        })
        .unwrap_or_else(std::env::temp_dir);

    #[cfg(not(test))]
    Ok(preferred_zoxide_data_dir(&data_root))
}

/// 選擇 PaneFM 的 zoxide 資料目錄，並相容改名前已存在的學習資料。
///
/// 參數：
/// - `data_root: &Path`，平台提供的應用程式資料根目錄。
///
/// 回傳：`PathBuf`。舊目錄存在且新目錄尚未建立時回傳舊目錄，其他情況回傳新版目錄。
fn preferred_zoxide_data_dir(data_root: &Path) -> PathBuf {
    let current = data_root.join("panefm").join("zoxide");
    let legacy = data_root.join("terminal-file-manager").join("zoxide");

    if !current.exists() && legacy.exists() {
        legacy
    } else {
        current
    }
}

/// 把指定目錄寫進 zoxide 資料庫，讓之後 `Z` / `:zoxide` 能依 frecency 排序跳轉。
///
/// 參數：
/// - `path: &Path`，要寫入資料庫的目錄路徑。
///
/// 回傳：`Result<()>`。
/// - 成功時代表 zoxide 已接受這次目錄記錄。
/// - 失敗時代表 zoxide binary 啟動失敗、資料目錄建立失敗，或 zoxide 回傳錯誤。
pub(crate) fn add_directory_to_zoxide(path: &Path) -> Result<()> {
    let data_dir = zoxide_data_dir()?;
    add_directory_to_zoxide_with_data_dir(path, &data_dir)
}

/// 用指定資料目錄把目錄寫進 zoxide 資料庫，供正式流程與測試共用。
fn add_directory_to_zoxide_with_data_dir(path: &Path, data_dir: &Path) -> Result<()> {
    if !path.is_dir() {
        return Ok(());
    }

    let command = zoxide_command()?;
    fs::create_dir_all(&data_dir).context("create zoxide data directory")?;

    let status = Command::new(command)
        .env("_ZO_DATA_DIR", &data_dir)
        .arg("add")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("run zoxide add")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("zoxide add exited with status {status}"))
    }
}

/// 從 zoxide 資料庫讀出目前可跳轉的目錄清單，依 frecency 由高到低排序。
///
/// 參數：無。
///
/// 回傳：`Result<Vec<PathBuf>>`。
/// - 成功時回傳 zoxide 建議的目錄清單。
/// - 失敗時代表 zoxide binary 啟動失敗、資料目錄建立失敗，或查詢指令回傳錯誤。
pub(crate) fn query_zoxide_directories() -> Result<Vec<PathBuf>> {
    let data_dir = zoxide_data_dir()?;
    query_zoxide_directories_with_data_dir(&data_dir)
}

/// 用指定資料目錄查詢 zoxide 資料庫，供正式流程與測試共用。
fn query_zoxide_directories_with_data_dir(data_dir: &Path) -> Result<Vec<PathBuf>> {
    let command = zoxide_command()?;
    fs::create_dir_all(&data_dir).context("create zoxide data directory")?;

    let output = Command::new(command)
        .env("_ZO_DATA_DIR", &data_dir)
        .arg("query")
        .arg("--list")
        .output()
        .context("run zoxide query --list")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "zoxide query exited with status {}",
            output.status
        ));
    }

    let stdout = String::from_utf8(output.stdout).context("decode zoxide query output")?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
}

#[cfg(test)]
mod tests {
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
}
