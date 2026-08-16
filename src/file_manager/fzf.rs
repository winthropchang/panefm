use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::bundled::{ensure_executable_permissions, tfm_cache_root_dir};

/// 目前內建的 `fzf` 版本字串。
pub(crate) const BUNDLED_FZF_VERSION: &str = "0.74.2";

/// 描述單一平台對應的內建 `fzf` asset。
///
/// 這個結構負責告訴程式：
/// - 目前平台應該解出哪一份 binary
/// - binary 檔名是什麼
/// - binary 實際 bytes 在哪裡
#[derive(Clone, Copy)]
struct BundledFzfAsset {
    platform_dir: &'static str,
    executable_name: &'static str,
    bytes: &'static [u8],
}

/// 取得目前平台應使用的內建 `fzf` 可執行檔路徑。
///
/// 參數：無。
///
/// 回傳：`Result<OsString>`。
/// - 成功時回傳已存在於本機快取目錄中的 `fzf` 路徑。
/// - 失敗時代表目前平台尚未內建對應 binary，或寫入快取時發生錯誤。
pub(crate) fn bundled_fzf_command() -> Result<OsString> {
    let Some(asset) = current_bundled_asset() else {
        return Err(anyhow::anyhow!(
            "bundled fzf is not available for this platform: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    };

    let binary_path = bundled_fzf_cache_path(asset)?;
    ensure_bundled_fzf_ready(asset, &binary_path)?;
    Ok(binary_path.into_os_string())
}

/// 回傳目前執行平台對應的內建 `fzf` asset。
fn current_bundled_asset() -> Option<BundledFzfAsset> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(BUNDLED_DARWIN_ARM64),
        ("macos", "x86_64") => Some(BUNDLED_DARWIN_AMD64),
        ("windows", "aarch64") => Some(BUNDLED_WINDOWS_ARM64),
        ("windows", "x86_64") => Some(BUNDLED_WINDOWS_AMD64),
        _ => None,
    }
}

/// 計算目前平台內建 `fzf` 在本機快取中的最終路徑。
fn bundled_fzf_cache_path(asset: BundledFzfAsset) -> Result<PathBuf> {
    Ok(tfm_cache_root_dir()?
        .join("bin")
        .join("fzf")
        .join(format!("v{BUNDLED_FZF_VERSION}"))
        .join(asset.platform_dir)
        .join(asset.executable_name))
}

/// 確保內建 `fzf` 已解包到本機快取目錄。
///
/// 若檔案不存在，或大小與內建 bytes 不一致，就重新寫出。
fn ensure_bundled_fzf_ready(asset: BundledFzfAsset, target_path: &Path) -> Result<()> {
    let needs_write = match fs::metadata(target_path) {
        Ok(metadata) => metadata.len() != asset.bytes.len() as u64,
        Err(_) => true,
    };

    if !needs_write {
        return Ok(());
    }

    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("bundled fzf target path has no parent"))?;
    fs::create_dir_all(parent).context("create bundled fzf cache directory")?;
    fs::write(target_path, asset.bytes).context("write bundled fzf binary")?;
    ensure_executable_permissions(target_path)?;
    Ok(())
}

const BUNDLED_DARWIN_ARM64: BundledFzfAsset = BundledFzfAsset {
    platform_dir: "darwin_arm64",
    executable_name: "fzf",
    bytes: include_bytes!("../../vendor/fzf/v0.74.2/darwin_arm64/fzf"),
};

const BUNDLED_DARWIN_AMD64: BundledFzfAsset = BundledFzfAsset {
    platform_dir: "darwin_amd64",
    executable_name: "fzf",
    bytes: include_bytes!("../../vendor/fzf/v0.74.2/darwin_amd64/fzf"),
};

const BUNDLED_WINDOWS_ARM64: BundledFzfAsset = BundledFzfAsset {
    platform_dir: "windows_arm64",
    executable_name: "fzf.exe",
    bytes: include_bytes!("../../vendor/fzf/v0.74.2/windows_arm64/fzf.exe"),
};

const BUNDLED_WINDOWS_AMD64: BundledFzfAsset = BundledFzfAsset {
    platform_dir: "windows_amd64",
    executable_name: "fzf.exe",
    bytes: include_bytes!("../../vendor/fzf/v0.74.2/windows_amd64/fzf.exe"),
};

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        BUNDLED_FZF_VERSION, bundled_fzf_cache_path, current_bundled_asset,
        ensure_bundled_fzf_ready, tfm_cache_root_dir,
    };

    #[test]
    fn cache_root_dir_is_not_empty() {
        let cache_dir = tfm_cache_root_dir().expect("cache dir");
        assert!(!cache_dir.as_os_str().is_empty());
    }

    #[test]
    fn supported_platform_builds_versioned_cache_path() {
        if let Some(asset) = current_bundled_asset() {
            let cache_path = bundled_fzf_cache_path(asset).expect("cache path");
            let path_text = cache_path.to_string_lossy();
            assert!(path_text.contains("terminal-file-manager"));
            assert!(path_text.contains(&format!("v{BUNDLED_FZF_VERSION}")));
            assert!(path_text.contains(asset.platform_dir));
            assert!(path_text.ends_with(asset.executable_name));
        }
    }

    #[test]
    fn ensure_bundled_fzf_ready_writes_binary_to_target_path() {
        let Some(asset) = current_bundled_asset() else {
            return;
        };

        let dir = tempdir().expect("tempdir");
        let target_path = dir.path().join(asset.executable_name);
        ensure_bundled_fzf_ready(asset, &target_path).expect("extract bundled fzf");

        let metadata = std::fs::metadata(&target_path).expect("metadata");
        assert_eq!(metadata.len(), asset.bytes.len() as u64);
    }
}
