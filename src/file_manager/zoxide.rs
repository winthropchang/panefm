use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};

use super::tools::{find_system_command, missing_tool_message};

/// 取得系統安裝的 `zoxide` 命令；本程式不再攜帶第三方 binary。
pub(crate) fn zoxide_command() -> Result<OsString> {
    find_system_command("zoxide")
        .ok_or_else(|| anyhow::anyhow!("{}", missing_tool_message("zoxide")))
}

/// 回傳 terminal-file-manager 專屬的 zoxide 資料目錄。
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
            .join("terminal-file-manager-tests")
            .join("zoxide")
            .join(thread_id));
    }

    #[cfg(not(test))]
    Ok(std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library").join("Application Support"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("terminal-file-manager")
        .join("zoxide"))
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
    use tempfile::tempdir;

    use super::{
        add_directory_to_zoxide_with_data_dir, query_zoxide_directories_with_data_dir,
        zoxide_data_dir,
    };

    #[test]
    fn zoxide_data_dir_is_not_empty() {
        let data_dir = zoxide_data_dir().expect("data dir");
        assert!(!data_dir.as_os_str().is_empty());
    }

    #[test]
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
}
