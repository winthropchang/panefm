use std::{
    io,
    path::{Path, PathBuf},
};

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

/// 將 SMB 位置依 Windows 規則轉成 UNC 路徑，供 pane 直接嘗試進入。
#[cfg(target_os = "windows")]
pub(crate) fn resolve_smb_location(location: &SmbLocation) -> ResolvedSmbLocation {
    #[cfg(target_os = "windows")]
    {
        ResolvedSmbLocation::Ready(windows_unc_path(location))
    }

    #[cfg(not(target_os = "windows"))]
    {
        resolve_smb_location_with_mount_root(location, Path::new("/Volumes"))
    }
}

/// 用指定掛載根目錄解析 SMB 位置，主要提供測試與 Unix 類平台使用。
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

#[cfg(target_os = "windows")]
fn windows_unc_path(location: &SmbLocation) -> PathBuf {
    let mut path = PathBuf::from(windows_unc_root(location));
    if !location.subpath.as_os_str().is_empty() {
        path.push(&location.subpath);
    }
    path
}

#[cfg(target_os = "windows")]
fn windows_unc_root(location: &SmbLocation) -> String {
    format!(r"\\{}\{}", location.host, location.share)
}

/// 將 `%20` 這類百分比編碼轉回可讀字串。
fn percent_decode(input: &str) -> io::Result<String> {
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
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{ResolvedSmbLocation, parse_smb_location, resolve_smb_location_with_mount_root};

    #[test]
    fn parse_smb_location_extracts_share_and_subpath() {
        let location =
            parse_smb_location("smb://192.0.2.10/shared/docs/report%20v1").expect("parse");

        assert_eq!(location.host, "192.0.2.10");
        assert_eq!(location.share, "shared");
        assert_eq!(location.subpath, Path::new("docs").join("report v1"));
    }

    #[test]
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
    fn parse_smb_location_requires_share_name() {
        let error = parse_smb_location("smb://192.0.2.10").expect_err("missing share");

        assert_eq!(
            error.to_string(),
            "SMB 位址格式錯誤：請使用 goto smb://host/share[/path]，不能只有 IP 或主機名稱"
        );
    }
}
