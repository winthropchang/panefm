#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AncestorTerminalInfo {
    pub(crate) kind: AncestorTerminalKind,
    pub(crate) exe_path: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AncestorTerminalKind {
    Alacritty,
    WezTerm,
    WindowsTerminal,
}

/// 透過祖先程序樹追蹤 Windows 當前正在運行的宿主終端名稱與執行檔絕對路徑。
#[cfg(target_os = "windows")]
pub(crate) fn detect_ancestor_terminal_info() -> Option<AncestorTerminalInfo> {
    use std::collections::HashMap;
    use std::mem::size_of;
    use std::os::windows::raw::HANDLE;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESSENTRY32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }

    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> HANDLE;
        fn Process32FirstW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> i32;
        fn CloseHandle(hObject: HANDLE) -> i32;
        fn GetCurrentProcessId() -> u32;
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> HANDLE;
        fn QueryFullProcessImageNameW(
            hProcess: HANDLE,
            dwFlags: u32,
            lpExeName: *mut u16,
            lpdwSize: *mut u32,
        ) -> i32;
    }

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut process_map = HashMap::new();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
                process_map.insert(entry.th32ProcessID, (entry.th32ParentProcessID, exe_name));

                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);

        let mut current_pid = GetCurrentProcessId();
        for _ in 0..10 {
            if let Some(&(parent_pid, ref exe_name)) = process_map.get(&current_pid) {
                let kind = if exe_name.contains("alacritty") {
                    Some(AncestorTerminalKind::Alacritty)
                } else if exe_name.contains("wezterm") {
                    Some(AncestorTerminalKind::WezTerm)
                } else if exe_name.contains("windowsterminal") || exe_name == "wt.exe" {
                    Some(AncestorTerminalKind::WindowsTerminal)
                } else {
                    None
                };

                if let Some(kind) = kind {
                    let mut exe_path = None;
                    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, current_pid);
                    if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                        let mut buffer = [0u16; 1024];
                        let mut size = buffer.len() as u32;
                        if QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size)
                            != 0
                            && size > 0
                        {
                            exe_path = Some(String::from_utf16_lossy(&buffer[..size as usize]));
                        }
                        CloseHandle(handle);
                    }
                    return Some(AncestorTerminalInfo { kind, exe_path });
                }

                if parent_pid == 0 || parent_pid == current_pid {
                    break;
                }
                current_pid = parent_pid;
            } else {
                break;
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_ancestor_terminal_info() -> Option<AncestorTerminalInfo> {
    None
}

/// 取得 Windows 向上追溯的祖先程序名稱清單（小寫）。
#[cfg(target_os = "windows")]
pub(crate) fn detect_ancestor_process_names() -> Vec<String> {
    use std::collections::HashMap;
    use std::mem::size_of;
    use std::os::windows::raw::HANDLE;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct PROCESSENTRY32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }

    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> HANDLE;
        fn Process32FirstW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> i32;
        fn Process32NextW(hSnapshot: HANDLE, lppe: *mut PROCESSENTRY32W) -> i32;
        fn CloseHandle(hObject: HANDLE) -> i32;
        fn GetCurrentProcessId() -> u32;
    }

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }

        let mut process_map = HashMap::new();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase();
                process_map.insert(entry.th32ProcessID, (entry.th32ParentProcessID, exe_name));

                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);

        let mut names = Vec::new();
        let mut current_pid = GetCurrentProcessId();
        for _ in 0..10 {
            if let Some(&(parent_pid, ref exe_name)) = process_map.get(&current_pid) {
                names.push(exe_name.clone());
                if parent_pid == 0 || parent_pid == current_pid {
                    break;
                }
                current_pid = parent_pid;
            } else {
                break;
            }
        }
        names
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_ancestor_process_names() -> Vec<String> {
    Vec::new()
}
