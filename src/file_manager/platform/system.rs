use std::{
    io,
    path::{Path, PathBuf},
};

use super::PlatformKind;
use crate::file_manager::open::{LaunchMode, LaunchSpec};

/// 取得目前執行檔所在的目錄路徑。
///
/// 若無法取得執行檔路徑或其父目錄，回傳 `None`。
pub(crate) fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

/// 將可能帶有 Windows 擴展長度前綴（`\\?\` 或 `\\?\UNC\`）的正規化路徑轉回乾淨的通用路徑。
///
/// Windows 的 `canonicalize()` 會自動加入 `\\?\` 前綴，但 PowerShell（如 `Set-Location`、`Test-Path`）
/// 以及許多終端工具無法直接識別 `\\?\`，甚至會拋出「路徑不存在」錯誤。
pub(crate) fn simplify_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{stripped}"))
        } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            path.to_path_buf()
        }
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// 判斷指定路徑是否位於網路共享（如 SMB / UNC、Windows 網路磁碟機或 macOS `/Volumes`）。
///
/// 網路共享上的檔案操作極易受 SMB 快取欺騙、Server-Side Copy Offload 失敗或 0-byte 假死影響，
/// 傳輸引擎應以本函式分流，改用可靠的分塊串流讀寫與伺服器落盤確認。
pub(crate) fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if text.starts_with(r"\\")
        || text.starts_with("//")
        || text == "/Volumes"
        || text.starts_with("/Volumes/")
        || text.starts_with(r"/Volumes\")
    {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        if let Some(std::path::Component::Prefix(prefix)) = path.components().next() {
            let prefix_str = prefix.as_os_str().to_string_lossy();
            if prefix_str.len() == 2 && prefix_str.ends_with(':') {
                let root_str = format!("{}\\", prefix_str);
                let wide: Vec<u16> = std::ffi::OsStr::new(&root_str)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
                }
                const DRIVE_REMOTE: u32 = 4;
                unsafe {
                    if GetDriveTypeW(wide.as_ptr()) == DRIVE_REMOTE {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[cfg(not(windows))]
    {
        false
    }
}

/// 依指定平台建立「用系統預設程式打開目標」的命令。
pub(crate) fn system_open_spec_for_platform(
    path: &Path,
    platform: PlatformKind,
) -> io::Result<LaunchSpec> {
    let launch = match platform {
        PlatformKind::Windows => LaunchSpec {
            program: "cmd.exe".to_string(),
            args: vec![
                "/C".to_string(),
                "start".to_string(),
                "".to_string(),
                path.display().to_string(),
            ],
            mode: LaunchMode::Detached,
        },
        PlatformKind::MacOs => LaunchSpec {
            program: "open".to_string(),
            args: vec![path.display().to_string()],
            mode: LaunchMode::Detached,
        },
        PlatformKind::LinuxLike => LaunchSpec {
            program: "xdg-open".to_string(),
            args: vec![path.display().to_string()],
            mode: LaunchMode::Detached,
        },
    };
    Ok(launch)
}

/// 依指定平台建立「在系統檔案管理器中顯示目標」的命令。
pub(crate) fn reveal_in_system_spec_for_platform(
    path: &Path,
    platform: PlatformKind,
) -> io::Result<LaunchSpec> {
    let launch = match platform {
        PlatformKind::Windows => LaunchSpec {
            program: "explorer.exe".to_string(),
            args: vec![format!("/select,{}", path.display())],
            mode: LaunchMode::Detached,
        },
        PlatformKind::MacOs => LaunchSpec {
            program: "open".to_string(),
            args: vec!["-R".to_string(), path.display().to_string()],
            mode: LaunchMode::Detached,
        },
        PlatformKind::LinuxLike => {
            let parent = path.parent().unwrap_or(path);
            return system_open_spec_for_platform(parent, platform);
        }
    };
    Ok(launch)
}

/// 查詢指定路徑所在檔案系統的可用空間（位元組）。
///
/// 若指定路徑尚未建立，會遞迴往上尋找最近存在的父目錄進行查詢。
#[cfg(unix)]
pub(crate) fn available_disk_space(path: &Path) -> io::Result<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let mut check_path = path;
    while !check_path.exists() {
        if let Some(parent) = check_path.parent() {
            check_path = parent;
        } else {
            break;
        }
    }

    let c_path = CString::new(check_path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    let frsize = if stat.f_frsize > 0 {
        stat.f_frsize as u64
    } else {
        stat.f_bsize as u64
    };
    Ok((stat.f_bavail as u64).saturating_mul(frsize))
}

/// 查詢指定路徑所在檔案系統的可用空間（位元組）。
#[cfg(windows)]
pub(crate) fn available_disk_space(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;

    let mut check_path = path;
    while !check_path.exists() {
        if let Some(parent) = check_path.parent() {
            check_path = parent;
        } else {
            break;
        }
    }

    let wide: Vec<u16> = check_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let mut free_bytes_to_caller = 0u64;
    let success = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_to_caller,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(free_bytes_to_caller)
    }
}

/// 查詢指定路徑所在檔案系統的可用空間（位元組）。
#[cfg(not(any(unix, windows)))]
pub(crate) fn available_disk_space(_path: &Path) -> io::Result<u64> {
    Ok(u64::MAX)
}
