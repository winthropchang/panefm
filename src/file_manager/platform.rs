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

    use super::{PlatformKind, reveal_in_system_spec_for_platform, system_open_spec_for_platform};
    use crate::file_manager::open::LaunchMode;

    #[test]
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
    fn mac_reveal_uses_open_r() {
        let spec = reveal_in_system_spec_for_platform(
            &PathBuf::from("/tmp/notes.txt"),
            PlatformKind::MacOs,
        )
        .expect("spec");

        assert_eq!(spec.program, "open");
        assert_eq!(spec.args, vec!["-R", "/tmp/notes.txt"]);
    }
}
