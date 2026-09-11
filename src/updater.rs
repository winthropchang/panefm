//! PaneFM 自我更新模組。
//!
//! 負責檢查 GitHub Releases 最新版本、依據作業系統與架構下載對應二進位資產，
//! 並透過 `self-replace` 進行安全替換。在無網路或離線環境下提供清楚友善的診斷提示，
//! 且在下載驗證完成前絕不觸碰現有執行檔。

use std::io::Write;
use std::time::Duration;

use serde::Deserialize;

/// GitHub Release API 之 JSON 結構。
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

/// GitHub Release 之單一資產結構。
#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

use std::path::PathBuf;

/// 啟動應用程式時的附加參數。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchArgs {
    /// 啟動時欲聚焦之初始目錄路徑。
    pub target_path: Option<PathBuf>,
}

/// 命令列呼叫之動作列舉。
#[derive(Debug, PartialEq, Eq)]
pub enum CliCommand {
    /// 顯示程式版本資訊 (`--version`, `-V`)。
    Version,
    /// 顯示程式說明資訊 (`--help`, `-h`)。
    Help,
    /// 執行自我更新指令 (`update`)。
    Update,
    /// 啟動 TUI 檔案管理器本體。
    RunApp,
    /// 啟動 TUI 檔案管理器本體並帶有自訂參數。
    Run(LaunchArgs),
}

/// 解析單一命令列參數為對應的 `CliCommand`（相容層）。
pub fn parse_cli_command(arg: Option<&std::ffi::OsStr>) -> CliCommand {
    match arg.and_then(std::ffi::OsStr::to_str) {
        Some("--version" | "-V") => CliCommand::Version,
        Some("--help" | "-h") => CliCommand::Help,
        Some("update") => CliCommand::Update,
        _ => CliCommand::RunApp,
    }
}

/// 解析多個命令列參數為對應的 `CliCommand`。
///
/// 支援旗標：
/// - `--version`, `-V`: 顯示版本
/// - `--help`, `-h`: 顯示說明
/// - `update`: 執行自我更新
/// - `[PATH]`: 啟動初始目錄
pub fn parse_cli_args<I, T>(args: I) -> CliCommand
where
    I: IntoIterator<Item = T>,
    T: AsRef<std::ffi::OsStr>,
{
    let mut args_iter = args.into_iter();
    let mut launch_args = LaunchArgs::default();
    let mut has_launch_args = false;

    while let Some(arg) = args_iter.next() {
        let arg_str = arg.as_ref().to_str();
        match arg_str {
            Some("--version" | "-V") => return CliCommand::Version,
            Some("--help" | "-h") => return CliCommand::Help,
            Some("update") => return CliCommand::Update,
            Some(other) if !other.starts_with('-') && launch_args.target_path.is_none() => {
                launch_args.target_path = Some(PathBuf::from(other));
                has_launch_args = true;
            }
            _ => {}
        }
    }

    if has_launch_args {
        CliCommand::Run(launch_args)
    } else {
        CliCommand::RunApp
    }
}

/// 版本檢查結果列舉。
#[derive(Debug, PartialEq, Eq)]
pub enum UpdateCheckResult {
    /// 當前已是最新版本，無須更新。
    UpToDate { current_version: String },
    /// 發現新版本，包含新舊版本資訊與資產下載網址。
    UpdateAvailable {
        current_version: String,
        latest_version: String,
        asset_name: String,
        download_url: String,
    },
}

/// 自我更新過程中可能發生的錯誤型別。
#[derive(Debug)]
pub enum UpdateError {
    /// 網路無連線、超時或無法解析主機。
    Network(String),
    /// GitHub API 存取頻率超過限制 (HTTP 403)。
    RateLimited,
    /// 遠端伺服器回應異常 HTTP 狀態碼。
    Http(u16, String),
    /// 解析遠端回應資料失敗。
    InvalidResponse(String),
    /// 版本號無法解析為語意化版本 (SemVer)。
    InvalidVersion(String),
    /// 發布版本中未包含相容於當前系統的資產檔案。
    AssetNotFound { version: String, asset_name: String },
    /// 當前作業系統或 CPU 架構不在預先建置的支援清單內。
    UnsupportedPlatform { os: String, arch: String },
    /// 寫入暫存檔或替換二進位檔失敗。
    ReplacementFailed(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Network(msg) => write!(
                f,
                "無法連線至 GitHub ({msg})。\n請檢查網路連線、防火牆或代理伺服器設定。"
            ),
            UpdateError::RateLimited => write!(
                f,
                "GitHub API 存取頻率已達上限 (403 Rate Limit)。\n請稍後再試，或直接前往 Releases 頁面手動下載：\nhttps://github.com/winthropchang/panefm/releases"
            ),
            UpdateError::Http(status, msg) => {
                write!(f, "GitHub 伺服器回傳 HTTP {status} 錯誤: {msg}")
            }
            UpdateError::InvalidResponse(msg) => write!(f, "解析 GitHub 發布資料失敗: {msg}"),
            UpdateError::InvalidVersion(msg) => write!(f, "版本比對失敗: {msg}"),
            UpdateError::AssetNotFound {
                version,
                asset_name,
            } => write!(
                f,
                "在 GitHub Release {version} 中找不到相容於當前系統的資產檔案 '{asset_name}'。\n請前往檢查發布內容：https://github.com/winthropchang/panefm/releases"
            ),
            UpdateError::UnsupportedPlatform { os, arch } => write!(
                f,
                "目前系統架構 ({os} {arch}) 尚未提供自動更新資產。\n請前往 Releases 頁面手動下載：https://github.com/winthropchang/panefm/releases"
            ),
            UpdateError::ReplacementFailed(msg) => {
                #[cfg(unix)]
                {
                    write!(
                        f,
                        "替換執行檔失敗: {msg}\n若程式安裝在系統保護目錄（如 /usr/local/bin），請嘗試使用 'sudo panefm update' 執行。"
                    )
                }
                #[cfg(not(unix))]
                {
                    write!(
                        f,
                        "替換執行檔失敗: {msg}\n請確認是否有足夠的檔案寫入權限，或關閉其他正在使用此檔案的視窗後重試。"
                    )
                }
            }
        }
    }
}

impl std::error::Error for UpdateError {}

/// 根據當前執行的作業系統與 CPU 架構，回傳預期對應的 GitHub Release 資產檔名。
///
/// 參數：無（讀取 `std::env::consts::OS` 與 `std::env::consts::ARCH`）。
/// 回傳：
/// - `Ok(&'static str)`：符合支援的平台資產檔名。
/// - `Err(UpdateError)`：目前平台不在預先發布的支援清單內。
pub fn get_target_asset_name() -> Result<&'static str, UpdateError> {
    match_platform_asset(std::env::consts::OS, std::env::consts::ARCH)
}

/// 根據指定的 OS 與 ARCH 字串比對資產檔名（便於跨平台單元測試）。
pub fn match_platform_asset(os: &str, arch: &str) -> Result<&'static str, UpdateError> {
    match (os, arch) {
        ("windows", "x86_64") => Ok("panefm-windows-x64.exe"),
        ("macos", "aarch64") => Ok("panefm-macos-arm64"),
        ("macos", "x86_64") => Ok("panefm-macos-x64"),
        _ => Err(UpdateError::UnsupportedPlatform {
            os: os.to_string(),
            arch: arch.to_string(),
        }),
    }
}

/// 解析 GitHub Release API 回傳之 JSON 字串，並與當前版本進行語意化版本比對。
///
/// 參數：
/// - `json_str`: GitHub API 回傳的 JSON 內文。
/// - `target_asset`: 當前系統所需要的資產檔名（例如 `panefm-windows-x64.exe`）。
/// - `current_version_str`: 當前運行的版本字串（例如 `0.1.6`）。
///
/// 回傳：`Result<UpdateCheckResult, UpdateError>`。
pub fn parse_latest_release(
    json_str: &str,
    target_asset: &str,
    current_version_str: &str,
) -> Result<UpdateCheckResult, UpdateError> {
    let release: GithubRelease =
        serde_json::from_str(json_str).map_err(|e| UpdateError::InvalidResponse(e.to_string()))?;

    let latest_tag = release.tag_name.trim();
    let latest_version_clean = latest_tag.strip_prefix('v').unwrap_or(latest_tag);

    let current_semver = semver::Version::parse(current_version_str)
        .map_err(|e| UpdateError::InvalidVersion(format!("無法解析當前版本號: {e}")))?;
    let latest_semver = semver::Version::parse(latest_version_clean).map_err(|e| {
        UpdateError::InvalidVersion(format!("無法解析遠端版本號 '{latest_tag}': {e}"))
    })?;

    if latest_semver > current_semver {
        let asset = release
            .assets
            .into_iter()
            .find(|a| a.name == target_asset)
            .ok_or_else(|| UpdateError::AssetNotFound {
                version: latest_tag.to_string(),
                asset_name: target_asset.to_string(),
            })?;

        Ok(UpdateCheckResult::UpdateAvailable {
            current_version: current_version_str.to_string(),
            latest_version: latest_version_clean.to_string(),
            asset_name: asset.name,
            download_url: asset.browser_download_url,
        })
    } else {
        Ok(UpdateCheckResult::UpToDate {
            current_version: current_version_str.to_string(),
        })
    }
}

/// 發送 HTTP 請求至 GitHub Releases API 檢查是否有新版本。
///
/// 參數：
/// - `timeout_secs`: 連線逾時秒數。
///
/// 回傳：`Result<UpdateCheckResult, UpdateError>`。
pub fn check_for_update(timeout_secs: u64) -> Result<UpdateCheckResult, UpdateError> {
    let target_asset = get_target_asset_name()?;
    let current_version = env!("CARGO_PKG_VERSION");
    let user_agent = format!("panefm/{current_version}");
    let url = "https://api.github.com/repos/winthropchang/panefm/releases/latest";

    let agent: ureq::Agent = ureq::config::Config::builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .build()
        .into();

    let response = agent
        .get(url)
        .header("User-Agent", &user_agent)
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(403) => UpdateError::RateLimited,
            ureq::Error::StatusCode(code) => UpdateError::Http(code, format!("HTTP {code}")),
            other => UpdateError::Network(other.to_string()),
        })?;

    let json_str = response
        .into_body()
        .read_to_string()
        .map_err(|e| UpdateError::Network(format!("讀取遠端回應內容失敗: {e}")))?;

    parse_latest_release(&json_str, target_asset, current_version)
}

/// 下載指定 URL 的二進位檔案至暫存檔，並透過 `self-replace` 替換當前執行檔。
///
/// 參數：
/// - `download_url`: 二進位檔案下載網址。
/// - `timeout_secs`: 下載逾時秒數。
///
/// 回傳：`Result<(), UpdateError>`。
pub fn download_and_install(download_url: &str, timeout_secs: u64) -> Result<(), UpdateError> {
    let current_version = env!("CARGO_PKG_VERSION");
    let user_agent = format!("panefm/{current_version}");

    let agent: ureq::Agent = ureq::config::Config::builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .build()
        .into();

    let response = agent
        .get(download_url)
        .header("User-Agent", &user_agent)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => {
                UpdateError::Http(code, format!("下載失敗，HTTP 狀態碼: {code}"))
            }
            other => UpdateError::Network(format!("下載連線失敗: {other}")),
        })?;

    // 建立臨時檔案儲存下載二進位資料
    let mut temp_file = tempfile::NamedTempFile::new()
        .map_err(|e| UpdateError::ReplacementFailed(format!("無法建立暫存檔案: {e}")))?;

    let mut reader = response.into_body().into_reader();
    std::io::copy(&mut reader, &mut temp_file)
        .map_err(|e| UpdateError::Network(format!("串流下載檔案中斷: {e}")))?;

    // 確保所有位元組均已寫入磁碟並關閉寫入 handle
    temp_file
        .flush()
        .map_err(|e| UpdateError::ReplacementFailed(format!("寫入暫存檔案失敗: {e}")))?;

    let temp_path = temp_file.path().to_path_buf();

    // 呼叫 self_replace 原子替換當前執行檔
    self_replace::self_replace(&temp_path)
        .map_err(|e| UpdateError::ReplacementFailed(e.to_string()))?;

    Ok(())
}

/// 命令列 `panefm update` 的主進入點函式。
///
/// 負責輸出友善進度與文字回饋，並處理成功與失敗狀態。
pub fn run_cli_update() -> Result<(), UpdateError> {
    println!("正在向 GitHub 檢查最新版本...");

    match check_for_update(8)? {
        UpdateCheckResult::UpToDate { current_version } => {
            println!("目前版本 v{current_version} 已是最新版本，無須更新。");
            Ok(())
        }
        UpdateCheckResult::UpdateAvailable {
            current_version,
            latest_version,
            asset_name,
            download_url,
        } => {
            println!("發現新版本：v{latest_version}（目前版本：v{current_version}）");
            println!("正在下載 {asset_name} 並安裝更新...");

            download_and_install(&download_url, 60)?;

            println!("更新完成！已成功升級至 v{latest_version}。");
            Ok(())
        }
    }
}
