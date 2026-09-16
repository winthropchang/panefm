//! PaneFM 自我更新模組。
//!
//! 負責檢查 GitHub Releases 最新版本、依據作業系統與架構下載對應二進位資產，
//! 並透過 `self-replace` 進行安全替換。在無網路或離線環境下提供清楚友善的診斷提示，
//! 且在下載驗證完成前絕不觸碰現有執行檔。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// 預設遠端版本檢查間隔：24 小時（86400 秒）。
pub const DEFAULT_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// 預設自我更新下載逾時時間：300 秒（5 分鐘）。
pub const DEFAULT_DOWNLOAD_TIMEOUT_SECS: u64 = 300;

/// 本地更新狀態快取，記錄上一次向 GitHub 查詢的時間與最新版本資訊。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStateCache {
    /// 上一次向遠端檢查之 UNIX 時間戳記（秒）。
    pub last_check_timestamp: u64,
    /// 遠端已發布之最新版本號（例如 "0.1.15"）。
    pub latest_version: String,
    /// 對應當前平台之二進位資產檔名。
    pub asset_name: String,
    /// 對應當前平台之資產下載網址。
    pub download_url: String,
}

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
    let mut launch_args = LaunchArgs::default();
    let mut has_launch_args = false;

    for arg in args {
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
/// - `timeout_secs`: 下載逾時秒數（建議使用 `DEFAULT_DOWNLOAD_TIMEOUT_SECS`）。
/// - `on_progress`: 進度回呼函式，傳入 `(已下載位元組數, 預估總位元組數 Option<u64>)`。
///
/// 回傳：`Result<(), UpdateError>`。
pub fn download_and_install_with_progress<F>(
    download_url: &str,
    timeout_secs: u64,
    mut on_progress: F,
) -> Result<(), UpdateError>
where
    F: FnMut(usize, Option<u64>),
{
    let trimmed = download_url.trim();
    if trimmed.is_empty() || (!trimmed.starts_with("http://") && !trimmed.starts_with("https://")) {
        return Err(UpdateError::Network("下載網址無效或為空".to_string()));
    }

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

    // 嘗試由 HTTP 回應標頭讀取 Content-Length 取得二進位檔案總大小
    let total_bytes = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    // 建立臨時檔案儲存下載二進位資料
    let mut temp_file = tempfile::NamedTempFile::new()
        .map_err(|e| UpdateError::ReplacementFailed(format!("無法建立暫存檔案: {e}")))?;

    let mut reader = response.into_body().into_reader();
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded: usize = 0;

    // 初始通知進度 0
    on_progress(0, total_bytes);

    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| UpdateError::Network(format!("串流下載檔案中斷: {e}")))?;
        if n == 0 {
            break;
        }
        temp_file
            .write_all(&buffer[..n])
            .map_err(|e| UpdateError::ReplacementFailed(format!("寫入暫存檔案失敗: {e}")))?;
        downloaded += n;
        on_progress(downloaded, total_bytes);
    }

    // 確保所有位元組均已寫入磁碟並關閉寫入 handle
    temp_file
        .flush()
        .map_err(|e| UpdateError::ReplacementFailed(format!("寫入暫存檔案失敗: {e}")))?;

    let temp_path = temp_file.path().to_path_buf();

    // 在 Unix 系統上，確保暫存檔案具備可執行權限 (0755)
    // 預設 NamedTempFile 建立權限為 0600，若不調整會在 self_replace 後導致執行檔失去執行權限 (Permission Denied)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&temp_path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&temp_path, perms);
        }
    }

    // 在 macOS 上，清除可能附帶的隔離屬性並進行 ad-hoc 程式碼簽名
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&temp_path)
            .output();
        let _ = std::process::Command::new("xattr")
            .args(["-c"])
            .arg(&temp_path)
            .output();
        let _ = std::process::Command::new("codesign")
            .args(["-f", "-s", "-"])
            .arg(&temp_path)
            .output();
    }

    // 呼叫 self_replace 原子替換當前執行檔
    self_replace::self_replace(&temp_path)
        .map_err(|e| UpdateError::ReplacementFailed(e.to_string()))?;

    // 替換完成後，在 Unix 系統上確保當前執行檔保持 0755 執行權限，並在 macOS 上清理屬性與重新 ad-hoc 簽名
    #[cfg(unix)]
    if let Ok(current_exe) = std::env::current_exe() {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&current_exe) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&current_exe, perms);
        }

        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("xattr")
                .args(["-d", "com.apple.quarantine"])
                .arg(&current_exe)
                .output();
            let _ = std::process::Command::new("xattr")
                .args(["-c"])
                .arg(&current_exe)
                .output();
            let _ = std::process::Command::new("codesign")
                .args(["-f", "-s", "-"])
                .arg(&current_exe)
                .output();
        }
    }

    Ok(())
}

/// 下載指定 URL 的二進位檔案至暫存檔，並透過 `self-replace` 替換當前執行檔（不帶進度回呼的相容介面）。
///
/// 參數：
/// - `download_url`: 二進位檔案下載網址。
/// - `timeout_secs`: 下載逾時秒數。
///
/// 回傳：`Result<(), UpdateError>`。
pub fn download_and_install(download_url: &str, timeout_secs: u64) -> Result<(), UpdateError> {
    download_and_install_with_progress(download_url, timeout_secs, |_, _| {})
}

/// 命令列 `panefm update` 的主進入點函式。
///
/// 負責輸出友善進度與文字回饋，並處理成功與失敗狀態。
pub fn run_cli_update() -> Result<(), UpdateError> {
    println!("正在向 GitHub 檢查最新版本...");

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    match check_for_update(8)? {
        UpdateCheckResult::UpToDate { current_version } => {
            println!("目前版本 v{current_version} 已是最新版本，無須更新。");
            let cache = UpdateStateCache {
                last_check_timestamp: now_secs,
                latest_version: current_version,
                asset_name: String::new(),
                download_url: String::new(),
            };
            let _ = save_update_cache(&cache, None);
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

            let start_time = Instant::now();
            let mut last_render = Instant::now();

            download_and_install_with_progress(
                &download_url,
                DEFAULT_DOWNLOAD_TIMEOUT_SECS,
                |downloaded, total| {
                    let now = Instant::now();
                    let is_done = total.map(|t| downloaded as u64 >= t).unwrap_or(false);
                    if !is_done && now.duration_since(last_render).as_millis() < 80 {
                        return;
                    }
                    last_render = now;

                    let elapsed_secs = start_time.elapsed().as_secs_f64().max(0.001);
                    let speed_bytes_sec = downloaded as f64 / elapsed_secs;
                    let speed_str = if speed_bytes_sec >= 1024.0 * 1024.0 {
                        format!("{:.2} MB/s", speed_bytes_sec / (1024.0 * 1024.0))
                    } else {
                        format!("{:.1} KB/s", speed_bytes_sec / 1024.0)
                    };

                    let downloaded_mb = downloaded as f64 / (1024.0 * 1024.0);

                    if let Some(total_bytes) = total {
                        let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
                        let percent =
                            ((downloaded as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0);
                        let bar_width = 25;
                        let filled = ((percent / 100.0) * bar_width as f64).round() as usize;
                        let bar: String = (0..bar_width)
                            .map(|i| {
                                if i < filled {
                                    '='
                                } else if i == filled {
                                    '>'
                                } else {
                                    ' '
                                }
                            })
                            .collect();

                        print!(
                            "\r下載進度: [{bar}] {:.1} MB / {:.1} MB ({:.1}%) [{speed_str}]   ",
                            downloaded_mb, total_mb, percent
                        );
                    } else {
                        print!("\r下載進度: {:.1} MB [{speed_str}]   ", downloaded_mb);
                    }
                    let _ = std::io::stdout().flush();
                },
            )?;

            // 印出換行讓進度條與完成訊息分開
            println!("\n更新完成！已成功升級至 v{latest_version}。");

            let cache = UpdateStateCache {
                last_check_timestamp: now_secs,
                latest_version: latest_version.clone(),
                asset_name: asset_name.clone(),
                download_url: download_url.clone(),
            };
            let _ = save_update_cache(&cache, None);

            Ok(())
        }
    }
}

/// 回傳預設存放本地快取的檔案路徑（優先與可執行檔同層，命名為 `.panefm_update.json`）。
pub fn default_update_cache_path() -> PathBuf {
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(parent) = exe_path.parent()
    {
        parent.join(".panefm_update.json")
    } else {
        PathBuf::from(".panefm_update.json")
    }
}

/// 當可執行檔所在目錄為唯讀時（例如系統 `/usr/bin`），備用的使用者設定目錄路徑。
pub fn fallback_config_update_cache_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("panefm").join(".panefm_update.json"))
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from) {
            Some(xdg.join("panefm").join(".panefm_update.json"))
        } else {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".config").join("panefm").join(".panefm_update.json"))
        }
    }
}

/// 從指定路徑（或預設路徑）載入本地更新狀態快取。
///
/// 若指定的檔案不存在或內容毀損，回傳 `None`。
pub fn load_update_cache(custom_path: Option<&Path>) -> Option<UpdateStateCache> {
    let candidate = custom_path
        .map(PathBuf::from)
        .unwrap_or_else(default_update_cache_path);
    if let Ok(content) = std::fs::read_to_string(&candidate)
        && let Ok(cache) = serde_json::from_str::<UpdateStateCache>(&content)
    {
        return Some(cache);
    }
    if custom_path.is_none()
        && let Some(fallback) = fallback_config_update_cache_path()
        && let Ok(content) = std::fs::read_to_string(&fallback)
        && let Ok(cache) = serde_json::from_str::<UpdateStateCache>(&content)
    {
        return Some(cache);
    }
    None
}

/// 將更新快取儲存至磁碟（優先存於執行檔同層，若權限不足則嘗試備用目錄）。
pub fn save_update_cache(
    cache: &UpdateStateCache,
    custom_path: Option<&Path>,
) -> Result<PathBuf, std::io::Error> {
    let json = serde_json::to_string_pretty(cache)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let path = custom_path
        .map(PathBuf::from)
        .unwrap_or_else(default_update_cache_path);

    match std::fs::write(&path, &json) {
        Ok(()) => Ok(path),
        Err(e) if custom_path.is_none() => {
            if let Some(fallback) = fallback_config_update_cache_path() {
                if let Some(parent) = fallback.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(&fallback, &json)?;
                Ok(fallback)
            } else {
                Err(e)
            }
        }
        Err(e) => Err(e),
    }
}

/// 比對兩個版本字串，判斷 `latest_version` 是否大於（更新於）`current_version`。
pub fn is_newer_version(latest_version: &str, current_version: &str) -> bool {
    let latest_clean = latest_version
        .trim()
        .strip_prefix('v')
        .unwrap_or(latest_version.trim());
    let current_clean = current_version
        .trim()
        .strip_prefix('v')
        .unwrap_or(current_version.trim());
    match (
        semver::Version::parse(latest_clean),
        semver::Version::parse(current_clean),
    ) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => false,
    }
}

/// 判斷當前是否需要發起遠端 GitHub 檢查。
///
/// 邏輯：
/// - 若從未檢查過（`cache` 為 `None`）：回傳 `true`。
/// - 若距離上次檢查時間戳已達到或超過 `interval_secs`（預設 24 小時）：回傳 `true`。
/// - 否則回傳 `false`（不連網）。
pub fn should_check_remote(
    cache: Option<&UpdateStateCache>,
    current_time_secs: u64,
    interval_secs: u64,
) -> bool {
    match cache {
        None => true,
        Some(c) => current_time_secs.saturating_sub(c.last_check_timestamp) >= interval_secs,
    }
}

/// 在背景執行緒發起 GitHub API 版本檢查，並在完成時更新本地快取檔。
///
/// 參數：
/// - `timeout_secs`: 連線逾時秒數（建議 5 秒）。
/// - `cache_path`: 自訂快取路徑（一般傳入 `None` 即使用預設路徑）。
///
/// 回傳：`std::sync::mpsc::Receiver<UpdateCheckResult>`。
pub fn spawn_background_update_check(
    timeout_secs: u64,
    cache_path: Option<PathBuf>,
) -> std::sync::mpsc::Receiver<UpdateCheckResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if let Ok(check_result) = check_for_update(timeout_secs) {
            match &check_result {
                UpdateCheckResult::UpdateAvailable {
                    latest_version,
                    download_url,
                    asset_name,
                    ..
                } => {
                    let cache = UpdateStateCache {
                        last_check_timestamp: now_secs,
                        latest_version: latest_version.clone(),
                        asset_name: asset_name.clone(),
                        download_url: download_url.clone(),
                    };
                    let _ = save_update_cache(&cache, cache_path.as_deref());
                }
                UpdateCheckResult::UpToDate { current_version } => {
                    let cache = UpdateStateCache {
                        last_check_timestamp: now_secs,
                        latest_version: current_version.clone(),
                        asset_name: String::new(),
                        download_url: String::new(),
                    };
                    let _ = save_update_cache(&cache, cache_path.as_deref());
                }
            }
            let _ = tx.send(check_result);
        }
    });
    rx
}
