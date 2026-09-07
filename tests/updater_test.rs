//! `updater` 自我更新模組的整合與回歸測試。
//!
//! 依據 `DEVELOPMENT_GUIDELINES.md` 規範，本檔案存放於 `tests/` 目錄中，
//! 與核心實作完全分離，涵蓋命令列參數解析、雙平台資產對照、Release JSON 解析、
//! 語意化版本比對以及各類異常錯誤格式化驗證。

use std::ffi::OsStr;

use panefm::updater::{
    CliCommand, UpdateCheckResult, UpdateError, match_platform_asset, parse_cli_command,
    parse_latest_release,
};

#[test]
/// 驗證命令列參數解析能精準辨識各類旗標與子指令。
/// 保護目的：避免 CLI 參數重構或擴充時，誤將版本/幫助/更新指令當作開啟檔案管理器，或導致簡寫旗標失效。
fn cli_command_parsing_recognizes_all_variants() {
    // 驗證版本旗標
    assert_eq!(
        parse_cli_command(Some(OsStr::new("--version"))),
        CliCommand::Version
    );
    assert_eq!(
        parse_cli_command(Some(OsStr::new("-V"))),
        CliCommand::Version
    );

    // 驗證說明旗標
    assert_eq!(
        parse_cli_command(Some(OsStr::new("--help"))),
        CliCommand::Help
    );
    assert_eq!(parse_cli_command(Some(OsStr::new("-h"))), CliCommand::Help);

    // 驗證更新子指令
    assert_eq!(
        parse_cli_command(Some(OsStr::new("update"))),
        CliCommand::Update
    );

    // 驗證未帶參數或無效參數時進入預設 TUI
    assert_eq!(parse_cli_command(None), CliCommand::RunApp);
    assert_eq!(
        parse_cli_command(Some(OsStr::new("unknown-command"))),
        CliCommand::RunApp
    );
}

#[test]
/// 驗證雙平台（Windows、macOS ARM64、macOS Intel）資產命名精確符合 GitHub Actions CI 產物。
/// 保護目的：確保正式發布的三個目標環境均能下載到正確相容的二進位檔案，避免執行檔架構錯誤。
fn platform_asset_matching_supports_windows_and_macos() {
    assert_eq!(
        match_platform_asset("windows", "x86_64").unwrap(),
        "panefm-windows-x64.exe"
    );
    assert_eq!(
        match_platform_asset("macos", "aarch64").unwrap(),
        "panefm-macos-arm64"
    );
    assert_eq!(
        match_platform_asset("macos", "x86_64").unwrap(),
        "panefm-macos-x64"
    );
}

#[test]
/// 驗證未支援的作業系統或 CPU 架構會回傳清晰的 UnsupportedPlatform 錯誤。
/// 保護目的：防止在尚未支援的架構上誤下載不相容的檔案，並提供友善的手動下載指引。
fn platform_asset_matching_rejects_unsupported_platforms() {
    let result = match_platform_asset("linux", "x86_64");
    assert!(matches!(
        result,
        Err(UpdateError::UnsupportedPlatform { os, arch }) if os == "linux" && arch == "x86_64"
    ));

    let result = match_platform_asset("windows", "i686");
    assert!(matches!(
        result,
        Err(UpdateError::UnsupportedPlatform { os, arch }) if os == "windows" && arch == "i686"
    ));
}

#[test]
/// 驗證當 GitHub 最新版本大於本機版本時，能正確判定有新版並提取該平台資產的下載連結。
/// 保護目的：確保語意化版本比對正常運作，且下載網址不被混淆或錯誤截斷。
fn release_parser_detects_newer_version() {
    let json_data = r#"{
        "tag_name": "v0.1.7",
        "assets": [
            {
                "name": "panefm-windows-x64.exe",
                "browser_download_url": "https://github.com/winthropchang/panefm/releases/download/v0.1.7/panefm-windows-x64.exe"
            },
            {
                "name": "panefm-macos-arm64",
                "browser_download_url": "https://github.com/winthropchang/panefm/releases/download/v0.1.7/panefm-macos-arm64"
            }
        ]
    }"#;

    let result = parse_latest_release(json_data, "panefm-windows-x64.exe", "0.1.6")
        .expect("should parse successfully");

    match result {
        UpdateCheckResult::UpdateAvailable {
            current_version,
            latest_version,
            asset_name,
            download_url,
        } => {
            assert_eq!(current_version, "0.1.6");
            assert_eq!(latest_version, "0.1.7");
            assert_eq!(asset_name, "panefm-windows-x64.exe");
            assert_eq!(
                download_url,
                "https://github.com/winthropchang/panefm/releases/download/v0.1.7/panefm-windows-x64.exe"
            );
        }
        _ => panic!("expected UpdateAvailable, got {:?}", result),
    }
}

#[test]
/// 驗證當遠端版本與本機相同或更舊時，回傳 UpToDate 狀態。
/// 保護目的：避免使用者在已是最新版本時被無故重複下載覆蓋，或本地開發版被遠端舊版降級。
fn release_parser_recognizes_already_up_to_date() {
    let json_data = r#"{
        "tag_name": "v0.1.6",
        "assets": [
            {
                "name": "panefm-windows-x64.exe",
                "browser_download_url": "https://example.com/asset.exe"
            }
        ]
    }"#;

    // 相同版本
    let result = parse_latest_release(json_data, "panefm-windows-x64.exe", "0.1.6").unwrap();
    assert_eq!(
        result,
        UpdateCheckResult::UpToDate {
            current_version: "0.1.6".to_string()
        }
    );

    // 本地版本比遠端還新（例如開發分支 0.2.0）
    let result = parse_latest_release(json_data, "panefm-windows-x64.exe", "0.2.0").unwrap();
    assert_eq!(
        result,
        UpdateCheckResult::UpToDate {
            current_version: "0.2.0".to_string()
        }
    );
}

#[test]
/// 驗證當遠端有新版本但缺乏對應平台之資產時，會回報 AssetNotFound 錯誤。
/// 保護目的：防止 GitHub Actions CI 若某平台編譯中斷漏傳 binary，導致使用者下載到空檔案或無效連結。
fn release_parser_reports_missing_asset() {
    let json_data = r#"{
        "tag_name": "v0.1.7",
        "assets": [
            {
                "name": "panefm-macos-arm64",
                "browser_download_url": "https://example.com/macos"
            }
        ]
    }"#;

    let result = parse_latest_release(json_data, "panefm-windows-x64.exe", "0.1.6");
    assert!(matches!(
        result,
        Err(UpdateError::AssetNotFound { version, asset_name })
            if version == "v0.1.7" && asset_name == "panefm-windows-x64.exe"
    ));
}

#[test]
/// 驗證當伺服器回應無效 JSON 或非 SemVer 標籤時，能妥善捕捉並回報錯誤而不崩潰。
/// 保護目的：防止 GitHub API 回應格式微調或非預期 HTML 錯誤頁面造成程式 panic。
fn release_parser_handles_invalid_json_or_semver() {
    let bad_json = "{ invalid_json }";
    let result = parse_latest_release(bad_json, "panefm-windows-x64.exe", "0.1.6");
    assert!(matches!(result, Err(UpdateError::InvalidResponse(_))));

    let bad_tag_json = r#"{ "tag_name": "not-a-version", "assets": [] }"#;
    let result = parse_latest_release(bad_tag_json, "panefm-windows-x64.exe", "0.1.6");
    assert!(matches!(result, Err(UpdateError::InvalidVersion(_))));
}

#[test]
/// 驗證在離線、防火牆阻擋或存取頻率受限情況下，錯誤訊息格式化具備清楚繁體中文指引。
/// 保護目的：提供良好的終端使用者體驗，使離線或無網路環境下的錯誤訊息清晰易懂，絕不輸出崩潰堆疊。
fn update_error_display_formatting_is_friendly_and_actionable() {
    let net_err = UpdateError::Network("Connection timed out".to_string());
    let net_msg = net_err.to_string();
    assert!(net_msg.contains("無法連線至 GitHub"));
    assert!(net_msg.contains("Connection timed out"));
    assert!(net_msg.contains("請檢查網路連線"));

    let rate_err = UpdateError::RateLimited;
    let rate_msg = rate_err.to_string();
    assert!(rate_msg.contains("403 Rate Limit"));
    assert!(rate_msg.contains("Releases 頁面手動下載"));

    let platform_err = UpdateError::UnsupportedPlatform {
        os: "solaris".to_string(),
        arch: "sparc".to_string(),
    };
    let platform_msg = platform_err.to_string();
    assert!(platform_msg.contains("solaris sparc"));
    assert!(platform_msg.contains("尚未提供自動更新資產"));
}
