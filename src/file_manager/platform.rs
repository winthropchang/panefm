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

/// 依指定平台建立「在目錄中開新終端」的命令規格。
///
/// Windows 不呼叫可能轉送到既有程序的 `wt.exe`，而是要求事件迴圈直接以新 console
/// 啟動 `cmd.exe`；子程序因此繼承 PaneFM 的環境與安全權杖。macOS 優先延續目前終端
/// App，無法辨識才交給 Terminal.app。LinuxLike 僅保留未來擴充用預設。
///
/// 參數：`path: &Path`，active panel cwd；`platform: PlatformKind`，目標平台。
/// 回傳：`io::Result<LaunchSpec>`，成功時可交給統一外部程序執行層。
pub(crate) fn new_terminal_spec_for_platform(
    path: &Path,
    platform: PlatformKind,
) -> io::Result<LaunchSpec> {
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let lc_terminal = std::env::var("LC_TERMINAL").ok();
    new_terminal_spec_for_platform_with_env(
        path,
        platform,
        term_program.as_deref(),
        lc_terminal.as_deref(),
    )
}

/// 依平台與終端識別環境建立新終端規格，並讓測試不必修改程序的全域環境變數。
///
/// macOS 終端通常透過 `TERM_PROGRAM` 或 `LC_TERMINAL` 告知子程序自己的名稱。PaneFM
/// 只把已知值轉成可交給 `open -a` 的 App 名稱；未知值回退到系統 Terminal.app，避免
/// 把任意環境內容當成應用程式名稱執行。Windows 不使用這兩個欄位。
///
/// 參數：`path: &Path`，active panel cwd；`platform: PlatformKind`，目標平台；
/// `term_program: Option<&str>` 與 `lc_terminal: Option<&str>`，目前終端提供的識別值。
/// 回傳：`io::Result<LaunchSpec>`，成功時包含可交給統一程序層執行的規格。
fn new_terminal_spec_for_platform_with_env(
    path: &Path,
    platform: PlatformKind,
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
) -> io::Result<LaunchSpec> {
    let launch = match platform {
        PlatformKind::Windows => LaunchSpec {
            program: "cmd.exe".to_string(),
            args: Vec::new(),
            mode: LaunchMode::NewTerminal {
                current_dir: path.to_path_buf(),
            },
        },
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

/// 將 macOS 終端環境變數轉成 Launch Services 可辨識的應用程式名稱。
///
/// 參數：`term_program: Option<&str>`、`lc_terminal: Option<&str>`，由目前 PaneFM 程序
/// 繼承的終端識別值。回傳：`&'static str`，已知終端的 App 名稱；無法辨識時回傳
/// `Terminal` 作為安全且普遍存在的預設值。
fn mac_terminal_application(term_program: Option<&str>, lc_terminal: Option<&str>) -> &'static str {
    for value in [term_program, lc_terminal].into_iter().flatten() {
        match value.trim().to_ascii_lowercase().as_str() {
            "iterm.app" | "iterm2" => return "iTerm",
            "apple_terminal" | "terminal.app" => return "Terminal",
            "wezterm" | "wezterm.app" => return "WezTerm",
            "ghostty" | "ghostty.app" => return "Ghostty",
            "warp" | "warpterminal" | "warp.app" => return "Warp",
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
        PlatformKind, new_terminal_spec_for_platform, new_terminal_spec_for_platform_with_env,
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
    /// 驗證 Windows 新終端直接使用新 console 並把 active panel cwd 保存於執行規格。
    /// 保護目的：避免未來改用 `wt.exe` broker，讓 TrustView 等父程序權杖意外遺失。
    fn windows_terminal_inherits_context_and_active_directory() {
        let path = PathBuf::from(r"C:\project\foo");
        let spec = new_terminal_spec_for_platform(&path, PlatformKind::Windows).expect("spec");

        assert_eq!(spec.program, "cmd.exe");
        assert_eq!(spec.mode, LaunchMode::NewTerminal { current_dir: path });
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
}
