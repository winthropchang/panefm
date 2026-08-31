//! SMB URL 解析、掛載位置判斷與跨平台路徑轉換。
//!
//! `smb://host/share/path` 是書籤與 command 使用的穩定表示；macOS 會解析已掛載
//! volume，Windows 則轉成 UNC path。此層只解析或產生掛載請求，不應執行檔案複製。

use std::{
    io,
    path::PathBuf,
};

#[cfg(any(test, target_os = "macos"))]
use std::path::Path;

#[cfg(all(target_os = "macos", not(test)))]
use std::process::Command;

use super::open::{LaunchMode, LaunchSpec};

/// 描述已解析的 SMB 位置資訊。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SmbLocation {
    pub(crate) url: String,
    pub(crate) host: String,
    pub(crate) share: String,
    pub(crate) subpath: PathBuf,
}

/// 描述目前 SMB 位置是否已能直接映射成可進入的本機路徑。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedSmbLocation {
    Ready(PathBuf),
    #[allow(dead_code)]
    NeedsMount { local_path: PathBuf },
}

/// 解析 `smb://host/share/path` 這類字串，整理出 host、share 與子路徑。
pub(crate) fn parse_smb_location(input: &str) -> io::Result<SmbLocation> {
    let trimmed = input.trim();
    let Some(rest) = trimmed.strip_prefix("smb://") else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SMB location must start with smb://",
        ));
    };

    let mut segments = rest.split('/');
    let host = segments.next().unwrap_or_default().trim();
    let share = segments.next().unwrap_or_default().trim();
    if host.is_empty() || share.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SMB 位址格式錯誤：請使用 goto smb://host/share[/path]，不能只有 IP 或主機名稱",
        ));
    }

    let decoded_share = percent_decode(share)?;
    let mut subpath = PathBuf::new();
    for segment in segments {
        if segment.is_empty() {
            continue;
        }
        subpath.push(percent_decode(segment)?);
    }

    Ok(SmbLocation {
        url: trimmed.to_string(),
        host: percent_decode(host)?,
        share: decoded_share,
        subpath,
    })
}

/// 將 SMB 位置依目前平台規則解析成實際可存取路徑。
///
/// Windows 直接使用 UNC；macOS 會讀取 mount table 並同時核對 host 與 share，
/// 避免同名 share 被系統掛載成 `/Volumes/name-1` 時誤用另一個掛載點。
///
/// 參數：`location: &SmbLocation`，已解析的 SMB 位址。
/// 回傳：`ResolvedSmbLocation`，包含可直接進入的路徑或需要掛載的預期位置。
#[cfg(all(any(target_os = "windows", target_os = "macos"), not(test)))]
pub(crate) fn resolve_smb_location(location: &SmbLocation) -> ResolvedSmbLocation {
    #[cfg(target_os = "windows")]
    {
        ResolvedSmbLocation::Ready(windows_unc_path(location))
    }

    #[cfg(target_os = "macos")]
    {
        resolve_macos_smb_location(location)
    }
}

/// 從 macOS 的 mount table 找出 host 與 share 都相符的 SMB 掛載點。
///
/// 參數：`location: &SmbLocation`，使用者輸入的 SMB 位址。
/// 回傳：`ResolvedSmbLocation`；找到正確掛載點時會再接上 SMB 子路徑。
#[cfg(all(target_os = "macos", not(test)))]
fn resolve_macos_smb_location(location: &SmbLocation) -> ResolvedSmbLocation {
    let mounted_root = Command::new("mount")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| find_macos_smb_mount(&output, &location.host, &location.share));

    let Some(share_root) = mounted_root else {
        return ResolvedSmbLocation::NeedsMount {
            local_path: Path::new("/Volumes")
                .join(&location.share)
                .join(&location.subpath),
        };
    };
    let local_path = if location.subpath.as_os_str().is_empty() {
        share_root
    } else {
        share_root.join(&location.subpath)
    };
    ResolvedSmbLocation::Ready(local_path)
}

/// 解析 macOS `mount` 輸出，找出指定 SMB host/share 對應的本機掛載目錄。
///
/// 例如 `//user@server/share on /Volumes/share-1 (smbfs, ...)` 會回傳
/// `/Volumes/share-1`，而不是只依 share 名稱猜測 `/Volumes/share`。
///
/// 參數：
/// - `mount_output: &str`，`mount` 命令的完整標準輸出。
/// - `expected_host: &str`，SMB 主機名稱或 IP。
/// - `expected_share: &str`，SMB share 名稱。
///
/// 回傳：`Option<PathBuf>`；找不到完全相符的 SMB 掛載時回傳 `None`。
#[cfg(any(test, target_os = "macos"))]
fn find_macos_smb_mount(
    mount_output: &str,
    expected_host: &str,
    expected_share: &str,
) -> Option<PathBuf> {
    mount_output.lines().find_map(|line| {
        let (source, mounted) = line.split_once(" on ")?;
        let mounted_path = mounted.split_once(" (")?.0;
        let remote = source.strip_prefix("//")?;
        let (authority, share) = remote.split_once('/')?;
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let decoded_share = percent_decode(share).unwrap_or_else(|_| share.to_string());

        (host.eq_ignore_ascii_case(expected_host)
            && decoded_share.eq_ignore_ascii_case(expected_share))
        .then(|| PathBuf::from(decode_mount_field(mounted_path)))
    })
}

/// 解開 mount 輸出欄位中的八進位跳脫，例如 `\040` 代表空白。
///
/// 參數：`input: &str`，mount table 中的單一路徑欄位。
/// 回傳：`String`，可交給 `PathBuf` 使用的本機路徑。
#[cfg(any(test, target_os = "macos"))]
fn decode_mount_field(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..=index + 3]
                .iter()
                .all(|byte| matches!(byte, b'0'..=b'7'))
        {
            let value = (bytes[index + 1] - b'0') * 64
                + (bytes[index + 2] - b'0') * 8
                + (bytes[index + 3] - b'0');
            output.push(value);
            index += 4;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// 用指定掛載根目錄解析 SMB 位置，主要提供測試與 Unix 類平台使用。
#[cfg(any(test, all(not(target_os = "windows"), not(target_os = "macos"))))]
pub(crate) fn resolve_smb_location_with_mount_root(
    location: &SmbLocation,
    mount_root: &Path,
) -> ResolvedSmbLocation {
    let share_root = mount_root.join(&location.share);
    let local_path = if location.subpath.as_os_str().is_empty() {
        share_root.clone()
    } else {
        share_root.join(&location.subpath)
    };

    if share_root.exists() {
        ResolvedSmbLocation::Ready(local_path)
    } else {
        ResolvedSmbLocation::NeedsMount { local_path }
    }
}

/// 建立目前平台用來請求系統掛載 SMB share 的外部命令。
pub(crate) fn build_smb_mount_launch(location: &SmbLocation) -> LaunchSpec {
    #[cfg(target_os = "windows")]
    {
        LaunchSpec {
            program: "explorer.exe".to_string(),
            args: vec![windows_unc_root(location)],
            mode: LaunchMode::Detached,
        }
    }

    #[cfg(target_os = "macos")]
    {
        LaunchSpec {
            program: "open".to_string(),
            args: vec![location.url.clone()],
            mode: LaunchMode::Detached,
        }
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        LaunchSpec {
            program: "xdg-open".to_string(),
            args: vec![location.url.clone()],
            mode: LaunchMode::Detached,
        }
    }
}

#[cfg(all(target_os = "windows", not(test)))]
/// 把已解析的 SMB share 與其子路徑組成 Windows 可直接存取的 UNC PathBuf。
///
/// 參數：`location: &SmbLocation`，包含 host、share 與可選 subpath。
/// 回傳：`PathBuf`，格式類似 `\\host\share\folder`。
fn windows_unc_path(location: &SmbLocation) -> PathBuf {
    let mut path = PathBuf::from(windows_unc_root(location));
    if !location.subpath.as_os_str().is_empty() {
        path.push(&location.subpath);
    }
    path
}

#[cfg(target_os = "windows")]
/// 只產生 Windows SMB share 根目錄，不附加 share 內部子路徑。
///
/// 參數：`location: &SmbLocation`；回傳 `\\host\share` 格式字串。
fn windows_unc_root(location: &SmbLocation) -> String {
    format!(r"\\{}\{}", location.host, location.share)
}

/// 將 `%20` 或 UTF-8 百分比編碼轉回可讀字串，供 SMB 解析與書籤顯示共用。
///
/// 參數：
/// - `input: &str`，可能含有 percent encoding 的 URI 或單一路徑片段。
///
/// 回傳：`io::Result<String>`。
/// - 成功時回傳解碼後的 UTF-8 文字。
/// - 編碼不完整、含非十六進位數字或結果不是合法 UTF-8 時回傳 `InvalidInput`。
pub(crate) fn percent_decode(input: &str) -> io::Result<String> {
    let bytes = input.as_bytes();
    let mut index = 0usize;
    let mut output = Vec::with_capacity(bytes.len());

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid percent-encoding in smb path",
                ));
            }
            let hex = &input[index + 1..index + 3];
            let value = u8::from_str_radix(hex, 16).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid percent-encoding in smb path",
                )
            })?;
            output.push(value);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(output).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "smb path is not valid utf-8 after decoding",
        )
    })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use tempfile::tempdir;

    use super::{
        ResolvedSmbLocation, decode_mount_field, find_macos_smb_mount, parse_smb_location,
        resolve_smb_location_with_mount_root,
    };

    #[test]
    /// 驗證 SMB URL 會拆成 host、share、subpath 並解碼百分比字元。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn parse_smb_location_extracts_share_and_subpath() {
        let location =
            parse_smb_location("smb://192.0.2.10/shared/docs/report%20v1").expect("parse");

        assert_eq!(location.host, "192.0.2.10");
        assert_eq!(location.share, "shared");
        assert_eq!(location.subpath, Path::new("docs").join("report v1"));
    }

    #[test]
    /// 驗證已存在掛載根目錄時直接回傳可瀏覽路徑，不再要求系統掛載。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn resolve_smb_location_with_mount_root_reports_ready_when_share_exists() {
        let dir = tempdir().expect("tempdir");
        let share_root = dir.path().join("shared");
        fs::create_dir(&share_root).expect("share");
        fs::create_dir(share_root.join("docs")).expect("docs");

        let location = parse_smb_location("smb://server/shared/docs").expect("parse");
        let resolved = resolve_smb_location_with_mount_root(&location, dir.path());

        assert_eq!(
            resolved,
            ResolvedSmbLocation::Ready(share_root.join("docs"))
        );
    }

    #[test]
    /// 驗證 share 尚未掛載時回傳 NeedsMount，讓 App 顯示明確連線流程。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn resolve_smb_location_with_mount_root_reports_needs_mount_when_missing() {
        let dir = tempdir().expect("tempdir");
        let location = parse_smb_location("smb://server/shared/docs").expect("parse");
        let resolved = resolve_smb_location_with_mount_root(&location, dir.path());

        assert_eq!(
            resolved,
            ResolvedSmbLocation::NeedsMount {
                local_path: dir.path().join("shared").join("docs")
            }
        );
    }

    #[test]
    /// 驗證缺少 share 名稱的 SMB URL 會被拒絕，避免跳到不明確的 host 根目錄。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn parse_smb_location_requires_share_name() {
        let error = parse_smb_location("smb://192.0.2.10").expect_err("missing share");

        assert_eq!(
            error.to_string(),
            "SMB 位址格式錯誤：請使用 goto smb://host/share[/path]，不能只有 IP 或主機名稱"
        );
    }

    #[test]
    /// 驗證 macOS 有同名 share 時，會依 host 選擇真正對應的 `-1` 掛載點。
    ///
    /// 參數：無。
    /// 回傳：無；若只依 share 名稱誤選其他伺服器的掛載點則測試失敗。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn macos_mount_parser_matches_host_and_share() {
        let output = "//otto@old-server/shared on /Volumes/shared (smbfs, nodev)\n\
                      //domain;otto@192.0.2.10/shared on /Volumes/shared-1 (smbfs, nodev)\n";

        assert_eq!(
            find_macos_smb_mount(output, "192.0.2.10", "shared"),
            Some(PathBuf::from("/Volumes/shared-1"))
        );
    }

    #[test]
    /// 驗證 mount table 的空白跳脫可以還原，避免含空白的掛載目錄無法進入。
    ///
    /// 參數：無。
    /// 回傳：無；若 `\040` 沒有還原成空白則測試失敗。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn mount_field_decoder_restores_octal_escapes() {
        assert_eq!(
            decode_mount_field("/Volumes/Company\\040Share"),
            "/Volumes/Company Share"
        );
    }
}
