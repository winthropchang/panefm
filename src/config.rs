use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::theme::ThemePreset;

/// 表示程式執行期間真正使用的完整設定。
///
/// 這個型別已經補齊預設值，並且通過基本驗證，
/// 因此後續畫面與邏輯層可以直接使用，不需要再處理 `Option`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppConfig {
    pub theme_preset: ThemePreset,
    pub poll_rate: Duration,
    pub confirm_dialog_width_percent: u16,
    pub confirm_dialog_height: u16,
    pub theme_picker_width_percent: u16,
    pub theme_picker_height: u16,
}

impl Default for AppConfig {
    /// 建立程式的預設設定值。
    ///
    /// 參數：無。
    /// 回傳：`AppConfig`，包含主題、poll 間隔與 popup 尺寸預設值。
    fn default() -> Self {
        Self {
            theme_preset: ThemePreset::Default,
            poll_rate: Duration::from_millis(150),
            confirm_dialog_width_percent: 60,
            confirm_dialog_height: 5,
            theme_picker_width_percent: 42,
            theme_picker_height: 8,
        }
    }
}

/// 表示設定檔載入後的結果。
///
/// 除了最終可用的 `AppConfig` 外，也保留設定來源路徑，
/// 方便啟動時在狀態列提示目前是從哪個檔案載入設定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedConfig {
    pub config: AppConfig,
    pub source: Option<PathBuf>,
}

/// 表示直接從 `toml` 反序列化出來的原始設定格式。
///
/// 每個欄位都使用 `Option`，因為外部設定檔允許只寫部分設定。
/// 之後會再轉換成 `AppConfig`，將缺省欄位補齊。
#[derive(Debug, Deserialize)]
struct AppConfigFile {
    theme: Option<String>,
    poll_rate_ms: Option<u64>,
    confirm_dialog: Option<DialogConfigFile>,
    theme_picker: Option<DialogConfigFile>,
}

/// 表示設定檔中 popup 類視窗的尺寸設定區塊。
///
/// 這個型別同時供刪除確認視窗與主題選擇視窗重複使用。
#[derive(Debug, Deserialize)]
struct DialogConfigFile {
    width_percent: Option<u16>,
    height: Option<u16>,
}

/// 依照既定搜尋順序載入設定檔。
///
/// 參數：
/// - `base_dir: &Path`，目前專案目錄，用於尋找本地 `config.toml`。
///
/// 回傳：`Result<LoadedConfig>`。
/// - 成功時回傳可直接使用的設定與來源路徑。
/// - 失敗時回傳讀檔、解析或驗證相關錯誤。
pub fn load_config(base_dir: &Path) -> Result<LoadedConfig> {
    // 這裡採用「找到第一個存在的設定檔就停止」的策略，
    // 行為會比較接近許多 CLI / TUI 工具常見的搜尋方式。
    let Some(path) = config_search_paths(base_dir)
        .into_iter()
        .find(|path| path.exists())
    else {
        return Ok(LoadedConfig {
            config: AppConfig::default(),
            source: None,
        });
    };

    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let parsed: AppConfigFile = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;

    Ok(LoadedConfig {
        config: AppConfig::from_file(parsed)?,
        source: Some(path),
    })
}

/// 建立設定檔搜尋路徑清單。
///
/// 參數：
/// - `base_dir: &Path`，目前專案目錄。
///
/// 回傳：`Vec<PathBuf>`，依照優先順序排列的候選設定檔路徑。
fn config_search_paths(base_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 讓使用者可以明確指定設定檔路徑，方便測試或切換不同配置。
    if let Some(path) = env::var_os("TFM_CONFIG") {
        paths.push(PathBuf::from(path));
    }

    // 專案根目錄下的 config.toml 適合這種「跟專案一起工作」的情境。
    paths.push(base_dir.join("config.toml"));

    if let Some(xdg_home) = env::var_os("XDG_CONFIG_HOME") {
        paths.push(
            PathBuf::from(xdg_home)
                .join("terminal-file-manager")
                .join("config.toml"),
        );
    }

    if let Some(home) = env::var_os("HOME") {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join("terminal-file-manager")
                .join("config.toml"),
        );
    }

    paths
}

impl AppConfig {
    /// 將原始設定檔結構轉成執行期設定。
    ///
    /// 參數：
    /// - `file: AppConfigFile`，由 toml 解析得到的原始資料。
    ///
    /// 回傳：`Result<AppConfig>`。
    /// - 成功時回傳已補上預設值且通過驗證的設定。
    /// - 失敗時回傳非法主題名稱或錯誤數值造成的錯誤。
    fn from_file(file: AppConfigFile) -> Result<Self> {
        let mut config = AppConfig::default();

        // 先套預設值，再用設定檔覆蓋指定欄位。
        // 這樣新增設定時比較不容易打壞舊檔案相容性。
        if let Some(name) = file.theme {
            config.theme_preset = ThemePreset::from_name(name.trim())
                .with_context(|| format!("unknown theme preset: {}", name.trim()))?;
        }

        if let Some(poll_rate_ms) = file.poll_rate_ms {
            if poll_rate_ms == 0 {
                bail!("poll_rate_ms must be greater than 0");
            }
            config.poll_rate = Duration::from_millis(poll_rate_ms);
        }

        if let Some(confirm_dialog) = file.confirm_dialog {
            apply_dialog_config(
                &mut config.confirm_dialog_width_percent,
                &mut config.confirm_dialog_height,
                confirm_dialog,
                "confirm_dialog",
            )?;
        }

        if let Some(theme_picker) = file.theme_picker {
            apply_dialog_config(
                &mut config.theme_picker_width_percent,
                &mut config.theme_picker_height,
                theme_picker,
                "theme_picker",
            )?;
        }

        Ok(config)
    }
}

/// 套用並驗證一組 popup 類視窗的尺寸設定。
///
/// 參數：
/// - `width_percent: &mut u16`，要被覆蓋的寬度百分比。
/// - `height: &mut u16`，要被覆蓋的高度列數。
/// - `dialog: DialogConfigFile`，來源設定資料。
/// - `field_name: &str`，用於錯誤訊息中的欄位名稱。
///
/// 回傳：`Result<()>`。
/// - 成功時代表設定已合法套用。
/// - 失敗時代表設定超出允許範圍。
fn apply_dialog_config(
    width_percent: &mut u16,
    height: &mut u16,
    dialog: DialogConfigFile,
    field_name: &str,
) -> Result<()> {
    // 這裡集中處理 popup 類視窗的尺寸驗證，
    // 避免 confirm dialog 和 theme picker 各自維護一套重複邏輯。
    if let Some(value) = dialog.width_percent {
        if !(1..=100).contains(&value) {
            bail!("{field_name}.width_percent must be between 1 and 100");
        }
        *width_percent = value;
    }

    if let Some(value) = dialog.height {
        if value == 0 {
            bail!("{field_name}.height must be greater than 0");
        }
        *height = value;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    /// 驗證當找不到設定檔時，系統會回退到預設設定。
    ///
    /// 參數：無。
    /// 回傳：無；若未正確回退則測試失敗。
    fn load_config_returns_defaults_when_missing() {
        let dir = tempdir().expect("tempdir");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.config, AppConfig::default());
        assert!(loaded.source.is_none());
    }

    #[test]
    /// 驗證專案目錄下的 `config.toml` 內容可以正確解析成設定。
    ///
    /// 參數：無。
    /// 回傳：無；若設定值未正確載入則測試失敗。
    fn load_config_reads_project_file() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("config.toml"),
            r#"
theme = "forest"
poll_rate_ms = 90

[confirm_dialog]
width_percent = 66
height = 7

[theme_picker]
width_percent = 55
height = 9
"#,
        )
        .expect("config file");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.config.theme_preset, ThemePreset::Forest);
        assert_eq!(loaded.config.poll_rate, Duration::from_millis(90));
        assert_eq!(loaded.config.confirm_dialog_width_percent, 66);
        assert_eq!(loaded.config.confirm_dialog_height, 7);
        assert_eq!(loaded.config.theme_picker_width_percent, 55);
        assert_eq!(loaded.config.theme_picker_height, 9);
        assert_eq!(loaded.source, Some(dir.path().join("config.toml")));
    }
}
