use std::path::{Path, PathBuf};

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
    let spec =
        system_open_spec_for_platform(&PathBuf::from(r"C:\work\notes.txt"), PlatformKind::Windows)
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
    let spec =
        reveal_in_system_spec_for_platform(&PathBuf::from("/tmp/notes.txt"), PlatformKind::MacOs)
            .expect("spec");

    assert_eq!(spec.program, "open");
    assert_eq!(spec.args, vec!["-R", "/tmp/notes.txt"]);
}

#[test]
/// 驗證 Windows 在未知終端環境時使用新 console 並把 active panel cwd 保存於執行規格。
/// 保護目的：避免改用特定 broker 時讓 TrustView 等父程序權杖意外遺失。
fn windows_terminal_inherits_context_and_active_directory() {
    let path = PathBuf::from(r"C:\project\foo");
    let spec = new_terminal_spec_for_platform_with_env(&path, PlatformKind::Windows, None, None)
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

    assert!(spec.program.to_lowercase().ends_with("alacritty.exe") || spec.program == "alacritty");
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
/// 驗證系統剪貼簿讀寫在當前平台能夠正常寫入與取出純文字。
fn clipboard_roundtrip_test() {
    let _lock = super::TEST_CLIPBOARD_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let text = "panefm_test_clipboard_roundtrip_42";
    let write_res = super::write_text_to_system_clipboard(text);
    assert!(
        write_res.is_ok(),
        "failed to write to clipboard: {:?}",
        write_res
    );
    let read = super::read_text_from_system_clipboard();
    assert_eq!(read.as_deref(), Some(text));
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

#[test]
/// 驗證 is_network_path 能正確辨識 UNC 與網路磁碟/掛載點。
fn is_network_path_detects_unc_and_remote_volumes() {
    assert!(super::is_network_path(Path::new(
        r"\\server\share\file.zip"
    )));
    assert!(super::is_network_path(Path::new("//server/share/file.zip")));
    #[cfg(target_os = "macos")]
    {
        assert!(super::is_network_path(Path::new(
            "/Volumes/Shared/file.zip"
        )));
        assert!(!super::is_network_path(Path::new("/Users/otto/file.zip")));
    }
    #[cfg(windows)]
    {
        assert!(!super::is_network_path(Path::new(r"C:\Windows\System32")));
    }
}
