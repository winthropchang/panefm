use std::{
    io::{self, Write},
    process::{Command, Stdio},
};

use super::{PlatformKind, current_platform};

/// 將指定文字寫入系統剪貼簿。
///
/// 目前正式支援：
/// - macOS：`pbcopy`
/// - Windows：`clip.exe`
pub(crate) fn write_text_to_system_clipboard_for_platform(
    text: &str,
    platform: PlatformKind,
) -> io::Result<()> {
    match platform {
        PlatformKind::Windows => {
            #[cfg(target_os = "windows")]
            {
                win_clipboard::write_clipboard_text(text)
                    .or_else(|_| win_clipboard::write_clip_exe(text))
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = (text, platform);
                Ok(())
            }
        }
        PlatformKind::MacOs => {
            let mut child = Command::new("pbcopy")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes())?;
            }
            let status = child.wait()?;
            if status.success() {
                Ok(())
            } else {
                Err(io::Error::other("clipboard command failed"))
            }
        }
        PlatformKind::LinuxLike => {
            let mut child = Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(text.as_bytes())?;
            }
            let status = child.wait()?;
            if status.success() {
                Ok(())
            } else {
                Err(io::Error::other("clipboard command failed"))
            }
        }
    }
}

/// 用目前實際執行的平台把文字寫入系統剪貼簿。
pub(crate) fn write_text_to_system_clipboard(text: &str) -> io::Result<()> {
    write_text_to_system_clipboard_for_platform(text, current_platform())
}

#[cfg(test)]
pub(crate) static TEST_CLIPBOARD_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(target_os = "windows")]
mod win_clipboard {
    use std::{io, ptr::null_mut};

    const CF_TEXT: u32 = 1;
    const CF_UNICODETEXT: u32 = 13;
    const CF_HDROP: u32 = 15;
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hWndNewOwner: *mut std::ffi::c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(uFormat: u32) -> *mut std::ffi::c_void;
        fn SetClipboardData(uFormat: u32, hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(uFlags: u32, dwBytes: usize) -> *mut std::ffi::c_void;
        fn GlobalLock(hMem: *mut std::ffi::c_void) -> *mut u8;
        fn GlobalUnlock(hMem: *mut std::ffi::c_void) -> i32;
        fn GlobalFree(hMem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn DragQueryFileW(
            hDrop: *mut std::ffi::c_void,
            iFile: u32,
            lpszFile: *mut u16,
            cch: u32,
        ) -> u32;
    }

    pub fn read_clipboard_text() -> Option<String> {
        unsafe {
            let mut opened = false;
            for _ in 0..30 {
                if OpenClipboard(null_mut()) != 0 {
                    opened = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if !opened {
                return None;
            }

            // 優先嘗試讀取 CF_UNICODETEXT (UTF-16)
            let handle = GetClipboardData(CF_UNICODETEXT);
            if !handle.is_null() {
                let ptr = GlobalLock(handle) as *mut u16;
                if !ptr.is_null() {
                    let mut len = 0;
                    while *ptr.add(len) != 0 {
                        len += 1;
                    }
                    let slice = std::slice::from_raw_parts(ptr, len);
                    let text = String::from_utf16_lossy(slice);
                    GlobalUnlock(handle);
                    CloseClipboard();
                    return Some(text);
                }
            }

            // 支援 CF_HDROP (檔案總管複製檔案或目錄)
            let handle_hdrop = GetClipboardData(CF_HDROP);
            if !handle_hdrop.is_null() {
                let file_count = DragQueryFileW(handle_hdrop, 0xFFFF_FFFF, null_mut(), 0);
                if file_count > 0 {
                    let mut paths = Vec::with_capacity(file_count as usize);
                    for i in 0..file_count {
                        let len = DragQueryFileW(handle_hdrop, i, null_mut(), 0);
                        if len > 0 {
                            let mut buf = vec![0u16; (len + 1) as usize];
                            DragQueryFileW(handle_hdrop, i, buf.as_mut_ptr(), len + 1);
                            buf.truncate(len as usize);
                            paths.push(String::from_utf16_lossy(&buf));
                        }
                    }
                    if !paths.is_empty() {
                        CloseClipboard();
                        return Some(paths.join(" "));
                    }
                }
            }

            // 次選 fallback: CF_TEXT (ANSI)
            let handle_ansi = GetClipboardData(CF_TEXT);
            if !handle_ansi.is_null() {
                let ptr = GlobalLock(handle_ansi);
                if !ptr.is_null() {
                    let c_str = std::ffi::CStr::from_ptr(ptr as *const std::ffi::c_char);
                    let text = c_str.to_string_lossy().into_owned();
                    GlobalUnlock(handle_ansi);
                    CloseClipboard();
                    return Some(text);
                }
            }

            CloseClipboard();
            None
        }
    }

    pub fn write_clipboard_text(text: &str) -> io::Result<()> {
        let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = utf16.len() * std::mem::size_of::<u16>();
        unsafe {
            let mut opened = false;
            for _ in 0..30 {
                if OpenClipboard(null_mut()) != 0 {
                    opened = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if !opened {
                return Err(io::Error::other("failed to open Windows clipboard"));
            }

            EmptyClipboard();

            let h_mem = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if h_mem.is_null() {
                CloseClipboard();
                return Err(io::Error::other("GlobalAlloc failed"));
            }

            let ptr = GlobalLock(h_mem) as *mut u16;
            if ptr.is_null() {
                GlobalFree(h_mem);
                CloseClipboard();
                return Err(io::Error::other("GlobalLock failed"));
            }

            std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
            GlobalUnlock(h_mem);

            if SetClipboardData(CF_UNICODETEXT, h_mem).is_null() {
                GlobalFree(h_mem);
                CloseClipboard();
                return Err(io::Error::other("SetClipboardData failed"));
            }

            CloseClipboard();
            Ok(())
        }
    }

    pub fn write_clip_exe(text: &str) -> io::Result<()> {
        let mut child = std::process::Command::new("clip.exe")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("clip.exe failed"))
        }
    }
}

/// 根據目標平台從作業系統剪貼簿讀取純文字字串。
pub(crate) fn read_text_from_system_clipboard_for_platform(
    platform: PlatformKind,
) -> Option<String> {
    match platform {
        PlatformKind::MacOs => {
            let output = Command::new("pbpaste")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .ok()?;
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        }
        PlatformKind::Windows => {
            #[cfg(target_os = "windows")]
            {
                win_clipboard::read_clipboard_text()
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = platform;
                None
            }
        }
        PlatformKind::LinuxLike => {
            if let Ok(output) = Command::new("wl-paste")
                .args(["--no-newline"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                && output.status.success()
                && let Ok(text) = String::from_utf8(output.stdout)
            {
                return Some(text);
            }
            let output = Command::new("xclip")
                .args(["-selection", "clipboard", "-o"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .ok()?;
            if output.status.success() {
                String::from_utf8(output.stdout).ok()
            } else {
                None
            }
        }
    }
}

/// 從目前實際執行的平台剪貼簿讀取純文字內容。
pub(crate) fn read_text_from_system_clipboard() -> Option<String> {
    read_text_from_system_clipboard_for_platform(current_platform())
}
