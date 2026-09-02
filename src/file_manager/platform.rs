//! macOS、Windows 與未來 Linux 的系統整合命令邊界。
//!
//! Reveal、系統開啟、SMB 掛載等平台差異應集中在此層，其他模組只使用 `LaunchSpec`
//! 或抽象函數。這可讓命令建構在目前平台以單元測試驗證另一平台，而不必真的啟動。

use std::{
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
};

use super::open::{LaunchMode, LaunchSpec};

/// 描述目前平台命令要針對哪一種作業系統產生。
///
/// 目前正式支援目標是 `Windows` 與 `MacOs`；
/// `LinuxLike` 先保留做為未來擴充點，避免之後要補 Unix / Linux 時重寫整層抽象。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum PlatformKind {
    Windows,
    MacOs,
    LinuxLike,
}

/// 回傳目前執行中的平台種類，供外部開啟、Reveal 等功能分流使用。
pub(crate) fn current_platform() -> PlatformKind {
    #[cfg(target_os = "windows")]
    {
        PlatformKind::Windows
    }

    #[cfg(target_os = "macos")]
    {
        PlatformKind::MacOs
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        PlatformKind::LinuxLike
    }
}

/// 取得目前執行檔所在的目錄路徑。
///
/// 若無法取得執行檔路徑或其父目錄，回傳 `None`。
pub(crate) fn executable_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

pub(crate) fn new_terminal_spec_for_platform(
    path: &Path,
    platform: PlatformKind,
) -> io::Result<LaunchSpec> {
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let lc_terminal = std::env::var("LC_TERMINAL").ok();
    let ancestor_info = detect_ancestor_terminal_info();

    let alacritty_active = std::env::var_os("ALACRITTY_WINDOW_ID").is_some()
        || std::env::var_os("ALACRITTY_LOG").is_some()
        || std::env::var_os("ALACRITTY_SOCKET").is_some()
        || std::env::var("TERM")
            .map(|t| t.to_ascii_lowercase())
            .as_deref()
            == Ok("alacritty")
        || matches!(
            ancestor_info.as_ref().map(|i| i.kind),
            Some(AncestorTerminalKind::Alacritty)
        );
    let wezterm_active = std::env::var_os("WEZTERM_PANE").is_some()
        || std::env::var_os("WEZTERM_EXECUTABLE").is_some()
        || matches!(
            ancestor_info.as_ref().map(|i| i.kind),
            Some(AncestorTerminalKind::WezTerm)
        );
    let wt_session_active = std::env::var_os("WT_SESSION").is_some()
        || matches!(
            ancestor_info.as_ref().map(|i| i.kind),
            Some(AncestorTerminalKind::WindowsTerminal)
        );
    let wt_profile_id = std::env::var("WT_PROFILE_ID").ok();

    let alacritty_exe = ancestor_info
        .as_ref()
        .filter(|i| i.kind == AncestorTerminalKind::Alacritty)
        .and_then(|i| i.exe_path.clone());
    let wezterm_exe = ancestor_info
        .as_ref()
        .filter(|i| i.kind == AncestorTerminalKind::WezTerm)
        .and_then(|i| i.exe_path.clone());

    new_terminal_spec_for_platform_with_env_flags(
        path,
        platform,
        term_program.as_deref(),
        lc_terminal.as_deref(),
        alacritty_active,
        wezterm_active,
        wt_session_active,
        wt_profile_id.as_deref(),
        alacritty_exe.as_deref(),
        wezterm_exe.as_deref(),
    )
}

/// 依平台與終端識別環境建立新終端規格，並讓測試不必修改程序的全域環境變數。
#[cfg(test)]
fn new_terminal_spec_for_platform_with_env(
    path: &Path,
    platform: PlatformKind,
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
) -> io::Result<LaunchSpec> {
    let alacritty_active = matches!(term_program, Some(p) if p.eq_ignore_ascii_case("alacritty"));
    let wezterm_active = matches!(term_program, Some(p) if p.eq_ignore_ascii_case("wezterm"));
    let wt_session_active =
        matches!(term_program, Some(p) if p.eq_ignore_ascii_case("windowsterminal"));

    new_terminal_spec_for_platform_with_env_flags(
        path,
        platform,
        term_program,
        lc_terminal,
        alacritty_active,
        wezterm_active,
        wt_session_active,
        None,
        None,
        None,
    )
}

/// 依平台、終端識別變數及各終端旗標建立新終端規格。
fn new_terminal_spec_for_platform_with_env_flags(
    path: &Path,
    platform: PlatformKind,
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
    alacritty_active: bool,
    wezterm_active: bool,
    wt_session_active: bool,
    wt_profile_id: Option<&str>,
    alacritty_exe: Option<&str>,
    wezterm_exe: Option<&str>,
) -> io::Result<LaunchSpec> {
    let launch = match platform {
        PlatformKind::Windows => {
            if wezterm_active
                || matches!(term_program, Some(p) if p.eq_ignore_ascii_case("wezterm"))
            {
                LaunchSpec {
                    program: resolve_wezterm_program(wezterm_exe),
                    args: vec![
                        "cli".to_string(),
                        "spawn".to_string(),
                        "--cwd".to_string(),
                        path.display().to_string(),
                    ],
                    mode: LaunchMode::Detached,
                }
            } else if alacritty_active
                || matches!(term_program, Some(p) if p.eq_ignore_ascii_case("alacritty"))
            {
                LaunchSpec {
                    program: resolve_alacritty_program(alacritty_exe),
                    args: vec![
                        "--working-directory".to_string(),
                        path.display().to_string(),
                    ],
                    mode: LaunchMode::Detached,
                }
            } else if wt_session_active {
                let mut args = vec!["-w".to_string(), "0".to_string(), "nt".to_string()];
                if let Some(profile_id) = wt_profile_id.filter(|p| !p.trim().is_empty()) {
                    args.push("-p".to_string());
                    args.push(profile_id.to_string());
                }
                args.push("-d".to_string());
                args.push(path.display().to_string());
                LaunchSpec {
                    program: "wt.exe".to_string(),
                    args,
                    mode: LaunchMode::Detached,
                }
            } else {
                let shell = default_windows_shell();
                LaunchSpec {
                    program: shell,
                    args: Vec::new(),
                    mode: LaunchMode::NewTerminal {
                        current_dir: path.to_path_buf(),
                    },
                }
            }
        }
        PlatformKind::MacOs => {
            let application = mac_terminal_application(term_program, lc_terminal);
            LaunchSpec {
                program: "open".to_string(),
                args: vec![
                    "-a".to_string(),
                    application.to_string(),
                    path.display().to_string(),
                ],
                mode: LaunchMode::Detached,
            }
        }
        PlatformKind::LinuxLike => LaunchSpec {
            program: "x-terminal-emulator".to_string(),
            args: Vec::new(),
            mode: LaunchMode::NewTerminal {
                current_dir: path.to_path_buf(),
            },
        },
    };
    Ok(launch)
}

/// 尋找系統上可用的 Alacritty 執行檔名稱或路徑。
fn resolve_alacritty_program(hint_exe: Option<&str>) -> String {
    if let Some(hint) = hint_exe {
        if Path::new(hint).exists() {
            return hint.to_string();
        }
    }
    if let Some(cmd) = crate::file_manager::tools::find_system_command("alacritty") {
        return cmd.to_string_lossy().into_owned();
    }
    "alacritty.exe".to_string()
}

/// 尋找系統上可用的 WezTerm 執行檔名稱或路徑。
///
/// 若祖先程序或環境變數提供的是 GUI 主程式 `wezterm-gui.exe`，自動換成同目錄下的 CLI 程式 `wezterm.exe`。
fn resolve_wezterm_program(hint_exe: Option<&str>) -> String {
    let env_exe = std::env::var("WEZTERM_EXECUTABLE").ok();
    let candidates = [hint_exe, env_exe.as_deref()];
    for candidate in candidates.into_iter().flatten() {
        let path = Path::new(candidate);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_ascii_lowercase());
        if file_name.as_deref() == Some("wezterm-gui.exe")
            || file_name.as_deref() == Some("wezterm-gui")
        {
            if let Some(parent) = path.parent() {
                let cli = parent.join("wezterm.exe");
                if cli.exists() {
                    return cli.to_string_lossy().into_owned();
                }
                let cli_no_ext = parent.join("wezterm");
                if cli_no_ext.exists() {
                    return cli_no_ext.to_string_lossy().into_owned();
                }
            }
        }
        if path.exists() {
            return candidate.to_string();
        }
    }
    if let Some(cmd) = crate::file_manager::tools::find_system_command("wezterm") {
        return cmd.to_string_lossy().into_owned();
    }
    "wezterm.exe".to_string()
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct AncestorTerminalInfo {
    kind: AncestorTerminalKind,
    exe_path: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AncestorTerminalKind {
    Alacritty,
    WezTerm,
    WindowsTerminal,
}

/// 透過祖先程序樹追蹤 Windows 當前正在運行的宿主終端名稱與執行檔絕對路徑。
#[cfg(target_os = "windows")]
fn detect_ancestor_terminal_info() -> Option<AncestorTerminalInfo> {
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
fn detect_ancestor_terminal_info() -> Option<AncestorTerminalInfo> {
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

/// 偵測 Windows 預設使用的命令解譯器（PowerShell 7 / Windows PowerShell / CMD）。
fn default_windows_shell() -> String {
    if std::env::var_os("POWERSHELL_DISTRIBUTION_CHANNEL").is_some() {
        return "pwsh.exe".to_string();
    }
    if std::env::var_os("PSModulePath").is_some() {
        if crate::file_manager::tools::find_system_command("pwsh").is_some() {
            return "pwsh.exe".to_string();
        }
        if crate::file_manager::tools::find_system_command("powershell").is_some() {
            return "powershell.exe".to_string();
        }
    }
    "cmd.exe".to_string()
}

/// 將 macOS 終端環境變數轉成 Launch Services 可辨識的應用程式名稱。
fn mac_terminal_application(term_program: Option<&str>, lc_terminal: Option<&str>) -> &'static str {
    for value in [term_program, lc_terminal].into_iter().flatten() {
        match value.trim().to_ascii_lowercase().as_str() {
            "iterm.app" | "iterm2" => return "iTerm",
            "apple_terminal" | "terminal.app" => return "Terminal",
            _ => {}
        }
    }
    "Terminal"
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

/// 將指定文字寫入系統剪貼簿。
///
/// 目前正式支援：
/// - macOS：`pbcopy`
/// - Windows：`clip.exe`
///
/// `LinuxLike` 先保留常見 `xclip` 路徑，方便未來擴充，但不是正式支援目標。
pub(crate) fn write_text_to_system_clipboard_for_platform(
    text: &str,
    platform: PlatformKind,
) -> io::Result<()> {
    let mut child = match platform {
        PlatformKind::MacOs => Command::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
        PlatformKind::Windows => Command::new("clip.exe")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
        PlatformKind::LinuxLike => Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    };

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("clipboard command failed"))
    }
}

/// 用目前實際執行的平台把文字寫入系統剪貼簿。
pub(crate) fn write_text_to_system_clipboard(text: &str) -> io::Result<()> {
    write_text_to_system_clipboard_for_platform(text, current_platform())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        PlatformKind, default_windows_shell, new_terminal_spec_for_platform_with_env,
        new_terminal_spec_for_platform_with_env_flags, resolve_wezterm_program,
        reveal_in_system_spec_for_platform, system_open_spec_for_platform,
    };
    use crate::file_manager::open::LaunchMode;

    #[test]
    /// 驗證 Windows 系統開啟會透過 `cmd /C start`，並保留目標路徑為獨立參數。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn windows_system_open_uses_cmd_start() {
        let spec = system_open_spec_for_platform(
            &PathBuf::from(r"C:\work\notes.txt"),
            PlatformKind::Windows,
        )
        .expect("spec");

        assert_eq!(spec.program, "cmd.exe");
        assert_eq!(spec.args[0], "/C");
        assert_eq!(spec.args[1], "start");
        assert_eq!(spec.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證 Windows Reveal 會使用 Explorer `/select,` 聚焦指定檔案。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn windows_reveal_uses_explorer_select() {
        let spec = reveal_in_system_spec_for_platform(
            &PathBuf::from(r"C:\work\notes.txt"),
            PlatformKind::Windows,
        )
        .expect("spec");

        assert_eq!(spec.program, "explorer.exe");
        assert_eq!(spec.args, vec![r"/select,C:\work\notes.txt"]);
        assert_eq!(spec.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證 macOS Reveal 會產生 `open -R`，而不是只打開父目錄。
    /// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
    fn mac_reveal_uses_open_r() {
        let spec = reveal_in_system_spec_for_platform(
            &PathBuf::from("/tmp/notes.txt"),
            PlatformKind::MacOs,
        )
        .expect("spec");

        assert_eq!(spec.program, "open");
        assert_eq!(spec.args, vec!["-R", "/tmp/notes.txt"]);
    }

    #[test]
    /// 驗證 Windows 在未知終端環境時使用新 console 並把 active panel cwd 保存於執行規格。
    /// 保護目的：避免改用特定 broker 時讓 TrustView 等父程序權杖意外遺失。
    fn windows_terminal_inherits_context_and_active_directory() {
        let path = PathBuf::from(r"C:\project\foo");
        let spec =
            new_terminal_spec_for_platform_with_env(&path, PlatformKind::Windows, None, None)
                .expect("spec");

        assert_eq!(spec.program, default_windows_shell());
        assert_eq!(spec.mode, LaunchMode::NewTerminal { current_dir: path });
    }

    #[test]
    /// 驗證 Windows 在 Alacritty 環境中會以 --working-directory 啟動新 Alacritty 視窗。
    fn windows_terminal_in_alacritty_opens_alacritty_window() {
        let path = PathBuf::from(r"C:\project\foo");
        let spec = new_terminal_spec_for_platform_with_env_flags(
            &path,
            PlatformKind::Windows,
            Some("Alacritty"),
            None,
            true,
            false,
            false,
            None,
            None,
            None,
        )
        .expect("spec");

        assert!(
            spec.program.to_lowercase().ends_with("alacritty.exe") || spec.program == "alacritty"
        );
        assert_eq!(spec.args, vec!["--working-directory", r"C:\project\foo"]);
        assert_eq!(spec.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證 Windows 在 WezTerm 環境中會以 wezterm cli spawn 在目前視窗開啟新 Tab。
    fn windows_terminal_in_wezterm_spawns_tab() {
        let path = PathBuf::from(r"C:\project\foo");
        let spec = new_terminal_spec_for_platform_with_env_flags(
            &path,
            PlatformKind::Windows,
            Some("WezTerm"),
            None,
            false,
            true,
            false,
            None,
            None,
            None,
        )
        .expect("spec");

        assert!(spec.program.to_lowercase().ends_with("wezterm.exe") || spec.program == "wezterm");
        assert_eq!(spec.args, vec!["cli", "spawn", "--cwd", r"C:\project\foo"]);
        assert_eq!(spec.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證祖先程序若為 wezterm-gui.exe 時，自動解析為同目錄下的 wezterm.exe CLI。
    fn wezterm_program_resolves_gui_to_cli() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cli = dir.path().join("wezterm.exe");
        std::fs::write(&cli, b"").expect("write dummy cli");
        let gui = dir.path().join("wezterm-gui.exe");
        std::fs::write(&gui, b"").expect("write dummy gui");

        let resolved = resolve_wezterm_program(Some(&gui.to_string_lossy()));
        assert_eq!(resolved, cli.to_string_lossy());
    }

    #[test]
    /// 驗證 Windows 在 Windows Terminal 環境中會以 wt -w 0 nt -p <profile> -d 開啟同 Profile 的新 Tab。
    fn windows_terminal_in_wt_spawns_tab_with_same_profile() {
        let path = PathBuf::from(r"C:\project\foo");
        let spec = new_terminal_spec_for_platform_with_env_flags(
            &path,
            PlatformKind::Windows,
            None,
            None,
            false,
            false,
            true,
            Some("{574e775e-4f2a-5b96-ac1e-a2962a402336}"),
            None,
            None,
        )
        .expect("spec");

        assert_eq!(spec.program, "wt.exe");
        assert_eq!(
            spec.args,
            vec![
                "-w",
                "0",
                "nt",
                "-p",
                "{574e775e-4f2a-5b96-ac1e-a2962a402336}",
                "-d",
                r"C:\project\foo"
            ]
        );
        assert_eq!(spec.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證無法辨識目前終端時，macOS 會安全回退到 Terminal.app。
    /// 保護目的：多 panel 時不可誤用其他 panel 的 cwd，未知環境也不可造成啟動失敗。
    fn mac_terminal_opens_active_directory() {
        let spec = new_terminal_spec_for_platform_with_env(
            &PathBuf::from("/Users/otto/project/foo"),
            PlatformKind::MacOs,
            None,
            None,
        )
        .expect("spec");

        assert_eq!(spec.program, "open");
        assert_eq!(spec.args, vec!["-a", "Terminal", "/Users/otto/project/foo"]);
        assert_eq!(spec.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證從 iTerm2 啟動 PaneFM 時，`wt` 會延續使用 iTerm，而非固定開 Terminal.app。
    /// 保護目的：避免未來平台重構再次把 macOS 終端寫死為系統內建 Terminal。
    fn mac_terminal_spec_preserves_iterm_from_term_program() {
        let spec = new_terminal_spec_for_platform_with_env(
            &PathBuf::from("/Users/otto/project/foo"),
            PlatformKind::MacOs,
            Some("iTerm.app"),
            Some("iTerm2"),
        )
        .expect("spec");

        assert_eq!(spec.program, "open");
        assert_eq!(spec.args, vec!["-a", "iTerm", "/Users/otto/project/foo"]);
        assert_eq!(spec.mode, LaunchMode::Detached);
    }

    #[test]
    /// 驗證主要識別值未知時仍可採用 `LC_TERMINAL` 的已知值。
    /// 保護目的：支援不同終端版本只設定其中一種識別環境變數的情況。
    fn mac_terminal_spec_uses_known_lc_terminal_as_fallback() {
        let spec = new_terminal_spec_for_platform_with_env(
            &PathBuf::from("/Users/otto/project/foo"),
            PlatformKind::MacOs,
            Some("unknown-wrapper"),
            Some("iTerm2"),
        )
        .expect("spec");

        assert_eq!(spec.args, vec!["-a", "iTerm", "/Users/otto/project/foo"]);
    }

    #[test]
    /// 驗證可正確取得當前執行檔所在的目錄路徑。
    /// 保護目的：避免應用程式狀態（如 bookmark、config）錯誤寫入工作目錄而非執行檔目錄。
    fn executable_dir_returns_valid_path_in_test_environment() {
        let dir = super::executable_dir();
        assert!(dir.is_some());
        let dir = dir.unwrap();
        assert!(dir.exists());
    }
}
