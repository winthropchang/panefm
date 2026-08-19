use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

/// 回傳 terminal-file-manager 使用的本機快取根目錄。
///
/// 設計原則：
/// - macOS 使用 `~/Library/Caches`
/// - Windows 使用 `%LOCALAPPDATA%`
/// - 其他平台先退回系統 temp dir，方便未來擴充
pub(crate) fn tfm_cache_root_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        return Ok(PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("terminal-file-manager"));
    }

    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA is not set"))?;
        return Ok(PathBuf::from(local_app_data).join("terminal-file-manager"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(std::env::temp_dir().join("terminal-file-manager"))
    }
}

/// 回傳 terminal-file-manager 使用的本機資料根目錄。
///
/// 這個目錄和快取不同，適合保存像 zoxide 資料庫這類希望跨重啟保留的內容。
///
/// 設計原則：
/// - macOS 使用 `~/Library/Application Support`
/// - Windows 使用 `%LOCALAPPDATA%`
/// - 其他平台先退回系統 temp dir，方便未來擴充
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn tfm_data_root_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        return Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("terminal-file-manager"));
    }

    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| anyhow::anyhow!("LOCALAPPDATA is not set"))?;
        return Ok(PathBuf::from(local_app_data).join("terminal-file-manager"));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Ok(std::env::temp_dir().join("terminal-file-manager"))
    }
}

/// 在 Unix-like 平台上補上可執行權限。
pub(crate) fn ensure_executable_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = fs::Permissions::from_mode(0o755);
        fs::set_permissions(path, permissions).context("set bundled executable permissions")?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
