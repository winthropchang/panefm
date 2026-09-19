use std::{io, path::Path};

use super::{
    PlatformKind,
    process::{AncestorTerminalKind, detect_ancestor_terminal_info},
};
use crate::file_manager::open::{LaunchMode, LaunchSpec};

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
pub(crate) fn new_terminal_spec_for_platform_with_env(
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_terminal_spec_for_platform_with_env_flags(
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
pub(crate) fn resolve_alacritty_program(hint_exe: Option<&str>) -> String {
    if let Some(hint) = hint_exe
        && Path::new(hint).exists()
    {
        return hint.to_string();
    }
    if let Some(cmd) = crate::file_manager::tools::find_system_command("alacritty") {
        return cmd.to_string_lossy().into_owned();
    }
    "alacritty.exe".to_string()
}

/// 尋找系統上可用的 WezTerm 執行檔名稱或路徑。
///
/// 若祖先程序或環境變數提供的是 GUI 主程式 `wezterm-gui.exe`，自動換成同目錄下的 CLI 程式 `wezterm.exe`。
pub(crate) fn resolve_wezterm_program(hint_exe: Option<&str>) -> String {
    let env_exe = std::env::var("WEZTERM_EXECUTABLE").ok();
    let candidates = [hint_exe, env_exe.as_deref()];
    for candidate in candidates.into_iter().flatten() {
        let path = Path::new(candidate);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_ascii_lowercase());
        if (file_name.as_deref() == Some("wezterm-gui.exe")
            || file_name.as_deref() == Some("wezterm-gui"))
            && let Some(parent) = path.parent()
        {
            let cli = parent.join("wezterm.exe");
            if cli.exists() {
                return cli.to_string_lossy().into_owned();
            }
            let cli_no_ext = parent.join("wezterm");
            if cli_no_ext.exists() {
                return cli_no_ext.to_string_lossy().into_owned();
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

/// 偵測 Windows 預設使用的命令解譯器（PowerShell 7 / Windows PowerShell / CMD）。
pub(crate) fn default_windows_shell() -> String {
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
pub(crate) fn mac_terminal_application(
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
) -> &'static str {
    for value in [term_program, lc_terminal].into_iter().flatten() {
        match value.trim().to_ascii_lowercase().as_str() {
            "iterm.app" | "iterm2" => return "iTerm",
            "apple_terminal" | "terminal.app" => return "Terminal",
            _ => {}
        }
    }
    "Terminal"
}
