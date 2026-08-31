//! 跨平台 Copy-on-Write (CoW) 檔案與目錄快速克隆模組。
//!
//! 在支援的檔案系統（如 macOS APFS）上，呼叫系統原生 `clonefile` 可在微秒內完成
//! 數十 GB 檔案或整個大型專案目錄的原子克隆，且在修改前不額外佔用磁碟空間。
//! 當來源與目的跨越不同磁區、檔案系統（如 FAT32/exFAT/NTFS）或網路芳鄰（SMB）時，
//! 本模組精確識別錯誤並提供平滑降級（Fallback）判定，由上層無縫切換為串流或平行複製。

#[cfg(any(target_os = "macos", test))]
use std::fs;
use std::io;
use std::path::Path;

#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn clonefile(
        src: *const std::ffi::c_char,
        dst: *const std::ffi::c_char,
        flags: u32,
    ) -> std::ffi::c_int;
}

/// 在支援的平台上嘗試使用 Copy-on-Write (CoW) 秒級克隆單一檔案。
///
/// 參數：
/// - `source: &Path`，來源檔案路徑。
/// - `target: &Path`，尚未存在的目標檔案路徑。
///
/// 回傳：
/// - `Ok(())`：成功建立 CoW 克隆檔案，且大小已核對一致。
/// - `Err(error)`：若回傳 `is_cow_unsupported_error(&error) == true`，代表跨磁區、跨網路芳鄰
///   （SMB）或檔案系統不支援，呼叫端應平滑降級為標準串流複製。
pub(crate) fn clone_file_cow(source: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("clone target already exists: {}", target.display()),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let expected_size = fs::metadata(source)?.len();
        let src_c = CString::new(source.as_os_str().as_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let dst_c = CString::new(target.as_os_str().as_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        // CLONE_NOFOLLOW = 0x0001: 不追隨符號連結，直接克隆連結本身
        let ret = unsafe { clonefile(src_c.as_ptr(), dst_c.as_ptr(), 1) };
        if ret == 0 {
            // 驗證克隆後的目標檔案大小
            let target_size = fs::metadata(target)?.len();
            if target_size == expected_size {
                return Ok(());
            } else {
                let _ = fs::remove_file(target);
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "incomplete CoW clone: expected {expected_size} bytes, got {target_size}"
                    ),
                ));
            }
        }
        Err(io::Error::last_os_error())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = source;
        let _ = target;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "CoW clonefile is not supported natively on this platform",
        ))
    }
}

/// 在支援的平台上嘗試使用 Copy-on-Write (CoW) 秒級克隆整個資料夾目錄樹。
///
/// 參數：
/// - `source: &Path`，來源資料夾路徑。
/// - `target: &Path`，尚未存在的目標資料夾路徑。
///
/// 回傳：
/// - `Ok(())`：成功建立 CoW 目錄克隆。
/// - `Err(error)`：若回傳 `is_cow_unsupported_error(&error) == true`，呼叫端應降級為平行多 Worker 複製。
#[allow(dead_code)]
pub(crate) fn clone_dir_cow(source: &Path, target: &Path) -> io::Result<()> {
    if target.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("clone target directory already exists: {}", target.display()),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let src_c = CString::new(source.as_os_str().as_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let dst_c = CString::new(target.as_os_str().as_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        let ret = unsafe { clonefile(src_c.as_ptr(), dst_c.as_ptr(), 1) };
        if ret == 0 {
            if target.is_dir() {
                return Ok(());
            } else {
                let _ = fs::remove_dir_all(target);
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "CoW clone directory did not create a directory",
                ));
            }
        }
        Err(io::Error::last_os_error())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = source;
        let _ = target;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "CoW directory clone is not supported natively on this platform",
        ))
    }
}

/// 判斷 CoW 克隆回傳的錯誤是否屬於「非致命／不支援／跨裝置」類型，應平滑降級至一般複製路徑。
///
/// 涵蓋情境：
/// - `EXDEV` (18)：跨磁區或跨檔案系統（如 APFS 複製到 SMB、或 Mac 內部 SSD 複製到外接隨身碟）。
/// - `ENOTSUP` (45) / `EOPNOTSUPP` (102)：檔案系統或遠端 SMB 伺服器不支援 CoW 指令。
/// - `EINVAL` (22) / `ENOSYS` (78/38)：特定虛擬檔案系統或不支援 syscall。
/// - `EPERM` (1)：特定掛載點不支援 clonefile。
/// - Windows / Linux 的對應錯誤碼。
pub(crate) fn is_cow_unsupported_error(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::Unsupported {
        return true;
    }

    #[cfg(unix)]
    {
        match error.raw_os_error() {
            // EXDEV = 18 (Cross-device link)
            Some(18) => true,
            // ENOTSUP = 45 on macOS, 95 on Linux; EOPNOTSUPP = 102 on macOS, 95 on Linux
            Some(45) | Some(95) | Some(102) => true,
            // EINVAL = 22, ENOSYS = 78 on macOS, 38 on Linux
            Some(22) | Some(38) | Some(78) => true,
            // EPERM = 1 (某些網路磁碟掛載時回傳)
            Some(1) => true,
            _ => false,
        }
    }

    #[cfg(windows)]
    {
        match error.raw_os_error() {
            // ERROR_INVALID_FUNCTION = 1, ERROR_NOT_SUPPORTED = 50, ERROR_NOT_SAME_DEVICE = 17
            Some(1) | Some(17) | Some(50) => true,
            _ => false,
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn cow_error_detector_identifies_unsupported_and_cross_device() {
        let unsupported_err = io::Error::new(io::ErrorKind::Unsupported, "not supported");
        assert!(is_cow_unsupported_error(&unsupported_err));

        #[cfg(unix)]
        {
            let exdev_err = io::Error::from_raw_os_error(18);
            assert!(is_cow_unsupported_error(&exdev_err));

            let enotsup_err = io::Error::from_raw_os_error(45);
            assert!(is_cow_unsupported_error(&enotsup_err));
        }

        let not_found = io::Error::new(io::ErrorKind::NotFound, "file not found");
        assert!(!is_cow_unsupported_error(&not_found));
    }

    #[test]
    fn cow_file_and_dir_clone_work_or_fallback_gracefully() {
        let dir = tempdir().expect("tempdir");
        let src_file = dir.path().join("source.txt");
        let dst_file = dir.path().join("target.txt");
        fs::write(&src_file, b"Hello Copy-on-Write!").expect("write src");

        let clone_result = clone_file_cow(&src_file, &dst_file);
        match clone_result {
            Ok(()) => {
                assert!(dst_file.exists());
                assert_eq!(
                    fs::read_to_string(&dst_file).expect("read dst"),
                    "Hello Copy-on-Write!"
                );
            }
            Err(error) => {
                assert!(
                    is_cow_unsupported_error(&error),
                    "若 CoW 失敗必須是可降級錯誤: {error:?}"
                );
            }
        }

        let src_dir = dir.path().join("src_folder");
        let dst_dir = dir.path().join("dst_folder");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(src_dir.join("subfile.txt"), b"subcontent").expect("write sub");

        let clone_dir_result = clone_dir_cow(&src_dir, &dst_dir);
        match clone_dir_result {
            Ok(()) => {
                assert!(dst_dir.is_dir());
                assert!(dst_dir.join("subfile.txt").exists());
            }
            Err(error) => {
                assert!(
                    is_cow_unsupported_error(&error),
                    "若 CoW 目錄克隆失敗必須是可降級錯誤: {error:?}"
                );
            }
        }
    }
}
