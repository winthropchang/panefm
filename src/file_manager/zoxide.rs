use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result};

#[cfg(not(test))]
use super::bundled::tfm_data_root_dir;
use super::bundled::{ensure_executable_permissions, tfm_cache_root_dir};

/// 目前內建的 `zoxide` 版本字串。
pub(crate) const BUNDLED_ZOXIDE_VERSION: &str = "0.10.0";

/// 產生原子寫入暫存檔名稱時使用的遞增序號。
static BUNDLED_ZOXIDE_WRITE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// 描述單一平台對應的內建 `zoxide` asset。
#[derive(Clone, Copy)]
struct BundledZoxideAsset {
    platform_dir: &'static str,
    executable_name: &'static str,
    bytes: &'static [u8],
}

/// 取得目前平台應使用的內建 `zoxide` 可執行檔路徑。
///
/// 參數：無。
///
/// 回傳：`Result<OsString>`。
/// - 成功時回傳已存在於本機快取目錄中的 `zoxide` 路徑。
/// - 失敗時代表目前平台尚未內建對應 binary，或寫入快取時發生錯誤。
pub(crate) fn bundled_zoxide_command() -> Result<OsString> {
    let Some(asset) = current_bundled_asset() else {
        return Err(anyhow::anyhow!(
            "bundled zoxide is not available for this platform: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    };

    let binary_path = bundled_zoxide_cache_path(asset)?;
    ensure_bundled_zoxide_ready(asset, &binary_path)?;
    Ok(binary_path.into_os_string())
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
    {
        Ok(tfm_data_root_dir()?.join("zoxide"))
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

    let command = bundled_zoxide_command()?;
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
    let command = bundled_zoxide_command()?;
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

/// 回傳目前執行平台對應的內建 `zoxide` asset。
fn current_bundled_asset() -> Option<BundledZoxideAsset> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(BUNDLED_DARWIN_ARM64),
        ("macos", "x86_64") => Some(BUNDLED_DARWIN_AMD64),
        ("windows", "aarch64") => Some(BUNDLED_WINDOWS_ARM64),
        ("windows", "x86_64") => Some(BUNDLED_WINDOWS_AMD64),
        _ => None,
    }
}

/// 計算目前平台內建 `zoxide` 在本機快取中的最終路徑。
fn bundled_zoxide_cache_path(asset: BundledZoxideAsset) -> Result<PathBuf> {
    Ok(tfm_cache_root_dir()?
        .join("bin")
        .join("zoxide")
        .join(format!("v{BUNDLED_ZOXIDE_VERSION}"))
        .join(asset.platform_dir)
        .join(asset.executable_name))
}

/// 確保內建 `zoxide` 已解包到本機快取目錄。
fn ensure_bundled_zoxide_ready(asset: BundledZoxideAsset, target_path: &Path) -> Result<()> {
    let needs_write = match fs::metadata(target_path) {
        Ok(metadata) => metadata.len() != asset.bytes.len() as u64,
        Err(_) => true,
    };

    if !needs_write {
        return Ok(());
    }

    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("bundled zoxide target path has no parent"))?;
    fs::create_dir_all(parent).context("create bundled zoxide cache directory")?;
    write_bundled_binary_atomically(target_path, asset.bytes)?;
    Ok(())
}

/// 以原子替換方式寫入內建 `zoxide` binary，避免平行測試讀到半寫入檔案。
///
/// 流程：
/// - 先在同目錄寫入唯一名稱的暫存檔
/// - 補上可執行權限
/// - 最後用 `rename` 一次替換正式檔案
fn write_bundled_binary_atomically(target_path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("bundled zoxide target path has no parent"))?;
    let sequence = BUNDLED_ZOXIDE_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp_name = format!(
        ".{}.tmp-{}-{}",
        target_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("zoxide"),
        std::process::id(),
        sequence
    );
    let temp_path = parent.join(temp_name);

    fs::write(&temp_path, bytes).context("write temporary bundled zoxide binary")?;
    ensure_executable_permissions(&temp_path)?;

    if let Err(error) = fs::rename(&temp_path, target_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error).context("replace bundled zoxide binary atomically");
    }

    Ok(())
}

const BUNDLED_DARWIN_ARM64: BundledZoxideAsset = BundledZoxideAsset {
    platform_dir: "darwin_arm64",
    executable_name: "zoxide",
    bytes: include_bytes!("../../vendor/zoxide/v0.10.0/darwin_arm64/zoxide"),
};

const BUNDLED_DARWIN_AMD64: BundledZoxideAsset = BundledZoxideAsset {
    platform_dir: "darwin_amd64",
    executable_name: "zoxide",
    bytes: include_bytes!("../../vendor/zoxide/v0.10.0/darwin_amd64/zoxide"),
};

const BUNDLED_WINDOWS_ARM64: BundledZoxideAsset = BundledZoxideAsset {
    platform_dir: "windows_arm64",
    executable_name: "zoxide.exe",
    bytes: include_bytes!("../../vendor/zoxide/v0.10.0/windows_arm64/zoxide.exe"),
};

const BUNDLED_WINDOWS_AMD64: BundledZoxideAsset = BundledZoxideAsset {
    platform_dir: "windows_amd64",
    executable_name: "zoxide.exe",
    bytes: include_bytes!("../../vendor/zoxide/v0.10.0/windows_amd64/zoxide.exe"),
};

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        BUNDLED_ZOXIDE_VERSION, add_directory_to_zoxide_with_data_dir, bundled_zoxide_cache_path,
        current_bundled_asset, ensure_bundled_zoxide_ready, query_zoxide_directories_with_data_dir,
        write_bundled_binary_atomically, zoxide_data_dir,
    };

    #[test]
    fn supported_platform_builds_versioned_cache_path() {
        if let Some(asset) = current_bundled_asset() {
            let cache_path = bundled_zoxide_cache_path(asset).expect("cache path");
            let path_text = cache_path.to_string_lossy();
            assert!(path_text.contains("terminal-file-manager"));
            assert!(path_text.contains(&format!("v{BUNDLED_ZOXIDE_VERSION}")));
            assert!(path_text.contains(asset.platform_dir));
            assert!(path_text.ends_with(asset.executable_name));
        }
    }

    #[test]
    fn ensure_bundled_zoxide_ready_writes_binary_to_target_path() {
        let Some(asset) = current_bundled_asset() else {
            return;
        };

        let dir = tempdir().expect("tempdir");
        let target_path = dir.path().join(asset.executable_name);
        ensure_bundled_zoxide_ready(asset, &target_path).expect("extract bundled zoxide");

        let metadata = std::fs::metadata(&target_path).expect("metadata");
        assert_eq!(metadata.len(), asset.bytes.len() as u64);
    }

    #[test]
    fn write_bundled_binary_atomically_replaces_target_without_temp_leftovers() {
        let Some(asset) = current_bundled_asset() else {
            return;
        };

        let dir = tempdir().expect("tempdir");
        let target_path = dir.path().join(asset.executable_name);

        std::fs::write(&target_path, b"old").expect("seed old file");
        write_bundled_binary_atomically(&target_path, asset.bytes).expect("atomic write");

        let metadata = std::fs::metadata(&target_path).expect("metadata");
        assert_eq!(metadata.len(), asset.bytes.len() as u64);

        let temp_files = std::fs::read_dir(dir.path())
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp-")
            })
            .count();
        assert_eq!(temp_files, 0);
    }

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
