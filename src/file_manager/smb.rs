//! SMB URL 解析、掛載位置判斷與跨平台路徑轉換。
//!
//! `smb://host/share/path` 是書籤與 command 使用的穩定表示；macOS 會解析已掛載
//! volume，Windows 則轉成 UNC path。此層只解析或產生掛載請求，不應執行檔案複製。

use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(any(test, target_os = "macos"))]
use std::fs;

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
    NeedsMount {
        local_path: PathBuf,
    },
}

/// 去除字串前後的外層引號（若有的話）。
pub(crate) fn strip_quotes(input: &str) -> &str {
    let trimmed = input.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].trim()
    } else {
        trimmed
    }
}

/// 去除主機字串中的帳號資訊（如 `user@` 或 `domain;user@`）、IPv6 中括號與 port（如 `:445`）。
pub(crate) fn clean_host(raw: &str) -> &str {
    let without_user = raw.rsplit('@').next().unwrap_or(raw);
    if let Some(stripped) = without_user.strip_prefix('[')
        && let Some(end) = stripped.find(']')
    {
        return &stripped[..end];
    }
    without_user.split(':').next().unwrap_or(without_user)
}

/// 解析 `smb://host/share/path`、`//host/share/path` 或 `\\host\share\path` 這類字串，
/// 整理出 host、share 與子路徑，並將 UNC 格式正規化為標準 `smb://` 格式。
/// 支援首尾引號清除與大小寫不敏感的 scheme 前綴。
pub(crate) fn parse_smb_location(input: &str) -> io::Result<SmbLocation> {
    let trimmed = strip_quotes(input);

    let (rest, was_unc) = if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("smb://") {
        (&trimmed[6..], false)
    } else if let Some(rest) = trimmed.strip_prefix("//") {
        (rest, true)
    } else if let Some(rest) = trimmed.strip_prefix(r"\\") {
        (rest, true)
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SMB location must start with smb://, //, or \\\\",
        ));
    };

    let normalized = if was_unc {
        rest.replace('\\', "/")
    } else {
        rest.to_string()
    };

    let mut segments = normalized.split('/').filter(|s| !s.is_empty());
    let raw_host = segments.next().unwrap_or_default().trim();
    let raw_share = segments.next().unwrap_or_default().trim();
    if raw_host.is_empty() || raw_share.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SMB 位址格式錯誤：請使用 goto smb://host/share[/path]，不能只有 IP 或主機名稱",
        ));
    }

    let decoded_share = percent_decode(raw_share)?;
    let mut subpath = PathBuf::new();
    let mut subpath_segments = Vec::new();
    for segment in segments {
        let trimmed_seg = segment.trim();
        if trimmed_seg.is_empty() {
            continue;
        }
        subpath.push(percent_decode(trimmed_seg)?);
        subpath_segments.push(trimmed_seg);
    }

    let host = percent_decode(clean_host(raw_host))?;

    let url = if was_unc {
        let mut canonical = format!("smb://{host}/{raw_share}");
        for seg in subpath_segments {
            canonical.push('/');
            canonical.push_str(seg);
        }
        canonical
    } else {
        trimmed.to_string()
    };

    Ok(SmbLocation {
        url,
        host,
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

/// 從 macOS 的 mount table 找出 host 與 share 都相符的 SMB 掛載點，
/// 若主機名稱不完全一致則 fallback 至 share 名稱相符的 smbfs 掛載點，
/// 若 mount 未列出則嘗試 `smbutil statshares -a`，
/// 若依然沒有則動態掃描 `/Volumes` 是否有相符或帶有 `-1` 等後綴的現存目錄。
///
/// 參數：`location: &SmbLocation`，使用者輸入的 SMB 位址。
/// 回傳：`ResolvedSmbLocation`；找到正確掛載點時會再接上 SMB 子路徑。
#[cfg(all(target_os = "macos", not(test)))]
fn resolve_macos_smb_location(location: &SmbLocation) -> ResolvedSmbLocation {
    let mount_output = get_macos_mount_output();

    let mounted_root = mount_output
        .as_deref()
        .and_then(|output| find_macos_smb_mount(output, &location.host, &location.share))
        .or_else(|| {
            mount_output
                .as_deref()
                .and_then(|output| find_macos_smb_mount_by_share(output, &location.share))
        })
        .or_else(|| {
            let smbutil_output = get_macos_smbutil_output();
            smbutil_output.as_deref().and_then(|output| {
                find_smbutil_mount(
                    output,
                    &location.host,
                    &location.share,
                    Path::new("/Volumes"),
                )
            })
        })
        .or_else(|| find_matching_volume(Path::new("/Volumes"), &location.share));

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

#[cfg(all(target_os = "macos", not(test)))]
fn get_macos_mount_output() -> Option<String> {
    let try_cmd = |prog: &str, args: &[&str]| {
        Command::new(prog)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };

    try_cmd("/sbin/mount", &[])
        .or_else(|| try_cmd("mount", &[]))
        .or_else(|| try_cmd("/sbin/mount", &["-t", "smbfs"]))
}

#[cfg(all(target_os = "macos", not(test)))]
fn get_macos_smbutil_output() -> Option<String> {
    let output = Command::new("/usr/bin/smbutil")
        .args(["statshares", "-a"])
        .output()
        .or_else(|_| Command::new("smbutil").args(["statshares", "-a"]).output())
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

/// 解析 macOS `mount` 輸出，找出指定 SMB host/share 對應的本機掛載目錄。
///
/// 例如 `//user@server/share on /Volumes/share-1 (smbfs, ...)` 會回傳
/// `/Volumes/share-1`，而不是只依 share 名稱猜測 `/Volumes/share`。
/// 支援 host 欄位包含 port (例如 `server:445`) 或帳號時的比對。
///
/// 參數：
/// - `mount_output: &str`，`mount` 命令的完整標準輸出。
/// - `expected_host: &str`，SMB 主機名稱或 IP。
/// - `expected_share: &str`，SMB share 名稱。
///
/// 回傳：`Option<PathBuf>`；找不到相符的 SMB 掛載時回傳 `None`。
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn find_macos_smb_mount(
    mount_output: &str,
    expected_host: &str,
    expected_share: &str,
) -> Option<PathBuf> {
    let clean_expected_host = clean_host(expected_host.trim());
    let clean_expected_share = decode_share_name(expected_share.trim().trim_matches('/'));

    mount_output.lines().find_map(|line| {
        let (source, mounted) = line.split_once(" on ")?;
        let (mounted_path, _fs_info) = mounted.rsplit_once(" (")?;
        let remote = source.strip_prefix("//")?;
        let (authority, share_part) = remote.split_once('/')?;
        let host = clean_host(authority);
        let raw_share = share_part.split('/').next().unwrap_or(share_part);
        let decoded_share = decode_share_name(raw_share);

        let host_matches = host.eq_ignore_ascii_case(clean_expected_host);
        let share_matches = decoded_share.eq_ignore_ascii_case(&clean_expected_share);

        (host_matches && share_matches).then(|| PathBuf::from(decode_mount_field(mounted_path)))
    })
}

/// 從 macOS 的 mount 輸出中，尋找檔案系統為 smbfs 且 share 名稱相符的掛載點。
/// 當使用者以 IP 輸入但 mount 紀錄中是 hostname 或帶有 port (如 :445) 時，
/// 此函式可作為 fallback 正確找到掛載目錄（例如 `/Volumes/mingfong` 或 `/Volumes/mingfong-1`）。
///
/// 參數：
/// - `mount_output: &str`，`mount` 命令的完整標準輸出。
/// - `expected_share: &str`，SMB share 名稱。
///
/// 回傳：`Option<PathBuf>`；找到相符的 smbfs 掛載時回傳本機掛載目錄。
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn find_macos_smb_mount_by_share(
    mount_output: &str,
    expected_share: &str,
) -> Option<PathBuf> {
    let clean_expected_share = decode_share_name(expected_share.trim().trim_matches('/'));

    mount_output.lines().find_map(|line| {
        let (source, mounted) = line.split_once(" on ")?;
        let (mounted_path, fs_info) = mounted.rsplit_once(" (")?;
        if !fs_info.starts_with("smbfs") {
            return None;
        }
        let remote = source.strip_prefix("//")?;
        let (_authority, share_part) = remote.split_once('/')?;
        let raw_share = share_part.split('/').next().unwrap_or(share_part);
        let decoded_share = decode_share_name(raw_share);
        if decoded_share.eq_ignore_ascii_case(&clean_expected_share) {
            Some(PathBuf::from(decode_mount_field(mounted_path)))
        } else {
            None
        }
    })
}

/// 同時支援八進位跳脫（如 `\040`）與百分比編碼（如 `%20`）之 share 名稱解碼。
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn decode_share_name(input: &str) -> String {
    let unescaped = decode_mount_field(input);
    percent_decode(&unescaped).unwrap_or(unescaped)
}

/// 解析 macOS `smbutil statshares -a` 輸出，取得活躍 SMB share 與 server 資訊。
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn parse_smbutil_statshares(output: &str) -> Vec<(String, String)> {
    let mut shares = Vec::new();
    for line in output.lines() {
        if let Some((before, after)) = line.split_once("SERVER_NAME") {
            let share = before.trim();
            let server = after.split_whitespace().next().unwrap_or("").trim();
            if !share.is_empty()
                && !server.is_empty()
                && !share.eq_ignore_ascii_case("SHARE")
                && !share.eq_ignore_ascii_case("ATTRIBUTE TYPE")
            {
                shares.push((share.to_string(), server.to_string()));
            }
        }
    }
    shares
}

/// 結合 `smbutil statshares` 輸出與 volumes 目錄掃描，確認掛載路徑。
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn find_smbutil_mount(
    smbutil_output: &str,
    expected_host: &str,
    expected_share: &str,
    volumes_dir: &Path,
) -> Option<PathBuf> {
    let shares = parse_smbutil_statshares(smbutil_output);
    let clean_expected_host = clean_host(expected_host.trim());
    let clean_expected_share = decode_share_name(expected_share.trim().trim_matches('/'));

    // 1. 同時比對 host 與 share
    for (share, server) in &shares {
        let clean_server = clean_host(server);
        let decoded_share = decode_share_name(share);
        if clean_server.eq_ignore_ascii_case(clean_expected_host)
            && decoded_share.eq_ignore_ascii_case(&clean_expected_share)
            && let Some(vol) = find_matching_volume(volumes_dir, &decoded_share)
        {
            return Some(vol);
        }
    }

    // 2. 備援：依 share 名稱比對
    for (share, _) in &shares {
        let decoded_share = decode_share_name(share);
        if decoded_share.eq_ignore_ascii_case(&clean_expected_share)
            && let Some(vol) = find_matching_volume(volumes_dir, &decoded_share)
        {
            return Some(vol);
        }
    }

    None
}

/// 檢查目錄是否存在且包含至少一筆項目。
#[cfg(any(test, target_os = "macos"))]
fn is_dir_non_empty(path: &Path) -> bool {
    if let Ok(mut rd) = fs::read_dir(path) {
        rd.next().is_some()
    } else {
        false
    }
}

/// 依 share 名稱在指定 volumes 目錄（通常為 `/Volumes`）中比對相符的掛載目錄。
/// 支援精確名稱、大小寫不敏感、以及 macOS 常見的重複掛載後綴（如 `-1`, `-2`, ` 1`）。
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn find_matching_volume(volumes_dir: &Path, expected_share: &str) -> Option<PathBuf> {
    let clean_expected = decode_share_name(expected_share.trim().trim_matches('/'));
    if clean_expected.is_empty() {
        return None;
    }

    let exact_path = volumes_dir.join(&clean_expected);
    let exact_exists = exact_path.is_dir();

    let read_dir = match fs::read_dir(volumes_dir) {
        Ok(rd) => rd,
        Err(_) => {
            return if exact_exists { Some(exact_path) } else { None };
        }
    };

    let mut exact_match: Option<PathBuf> = None;
    let mut suffixed_matches: Vec<PathBuf> = Vec::new();

    for entry in read_dir.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !file_type.is_dir() && !file_type.is_symlink() {
            continue;
        }

        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        if name_str.eq_ignore_ascii_case(&clean_expected) {
            exact_match = Some(entry.path());
        } else if name_str.len() > clean_expected.len()
            && name_str[..clean_expected.len()].eq_ignore_ascii_case(&clean_expected)
        {
            let suffix = &name_str[clean_expected.len()..];
            if (suffix.starts_with('-') || suffix.starts_with(' '))
                && suffix[1..].chars().all(|c| c.is_ascii_digit())
            {
                suffixed_matches.push(entry.path());
            }
        }
    }

    if !suffixed_matches.is_empty() {
        suffixed_matches.sort_by(|a, b| b.cmp(a));
        if let Some(exact) = exact_match.as_ref()
            && is_dir_non_empty(exact)
        {
            return exact_match;
        }
        return Some(suffixed_matches.remove(0));
    }

    if exact_match.is_some() {
        exact_match
    } else if exact_exists {
        Some(exact_path)
    } else {
        None
    }
}

/// 解開 mount 輸出欄位中的八進位跳脫，例如 `\040` 代表空白。
///
/// 參數：`input: &str`，mount table 中的單一路徑欄位。
/// 回傳：`String`，可交給 `PathBuf` 使用的本機路徑。
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn decode_mount_field(input: &str) -> String {
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
    let share_root = find_matching_volume(mount_root, &location.share)
        .unwrap_or_else(|| mount_root.join(&location.share));
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
#[path = "tests/smb_test.rs"]
mod tests;
