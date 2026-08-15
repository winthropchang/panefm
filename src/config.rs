use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::theme::ThemePreset;

/// 描述啟動時要套用的預設排序方式。
///
/// 這個型別只負責保存設定檔中的語意，
/// 實際檔案列表要怎麼排序，會在檔案管理器啟動時再轉成對應邏輯。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupSort {
    Alphabetical,
    Natural,
    Size,
    Modified,
    Created,
    Extension,
    Random,
}

impl StartupSort {
    /// 將設定檔中的文字名稱轉成對應的排序種類。
    ///
    /// 參數：
    /// - `name: &str`，設定檔中寫的排序名稱。
    ///
    /// 回傳：`Option<StartupSort>`。
    /// - `Some(...)` 代表名稱有效。
    /// - `None` 代表名稱不在支援清單內。
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "alphabetical" | "alpha" => Some(Self::Alphabetical),
            "natural" => Some(Self::Natural),
            "size" => Some(Self::Size),
            "modified" | "mtime" => Some(Self::Modified),
            "created" | "birth" | "btime" => Some(Self::Created),
            "extension" | "ext" => Some(Self::Extension),
            "random" => Some(Self::Random),
            _ => None,
        }
    }

    /// 回傳適合顯示在說明文件中的名稱。
    ///
    /// 參數：無。
    /// 回傳：`&'static str`。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Alphabetical => "alphabetical",
            Self::Natural => "natural",
            Self::Size => "size",
            Self::Modified => "modified",
            Self::Created => "created",
            Self::Extension => "extension",
            Self::Random => "random",
        }
    }
}

/// 表示程式執行期間真正使用的完整設定。
///
/// 這個型別已經補齊預設值，並且通過基本驗證，
/// 因此後續畫面與邏輯層可以直接使用，不需要再處理 `Option`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub pane: PaneConfig,
    pub search: SearchConfig,
    pub behavior: BehaviorConfig,
}

/// 表示 UI 相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiConfig {
    pub theme_preset: ThemePreset,
    pub poll_rate: Duration,
    pub preview: PreviewConfig,
    pub dialogs: DialogsConfig,
}

/// 表示 pane 預設行為相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneConfig {
    pub show_hidden: bool,
    pub default_sort: StartupSort,
    pub default_sort_reverse: bool,
}

/// 表示搜尋行為相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchConfig {
    pub global_search_limit: usize,
    pub global_search_chunk_size: usize,
    pub show_loading: bool,
}

/// 表示互動行為相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BehaviorConfig {
    pub cancel_search_on_leave: bool,
}

/// 表示 preview 區塊的高度設定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewConfig {
    pub height: u16,
    pub focus_list_height: u16,
}

/// 表示所有 popup / dialog 類視窗的設定集合。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogsConfig {
    pub confirm: DialogConfig,
    pub theme_picker: DialogConfig,
}

/// 表示單一 dialog 的尺寸設定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogConfig {
    pub width_percent: u16,
    pub height: u16,
}

impl Default for AppConfig {
    /// 建立程式的預設設定值。
    ///
    /// 參數：無。
    /// 回傳：`AppConfig`，包含主題、poll 間隔、搜尋與 popup 尺寸預設值。
    fn default() -> Self {
        Self {
            ui: UiConfig {
                theme_preset: ThemePreset::Default,
                poll_rate: Duration::from_millis(150),
                preview: PreviewConfig {
                    height: 8,
                    focus_list_height: 6,
                },
                dialogs: DialogsConfig {
                    confirm: DialogConfig {
                        width_percent: 60,
                        height: 5,
                    },
                    theme_picker: DialogConfig {
                        width_percent: 42,
                        height: 8,
                    },
                },
            },
            pane: PaneConfig {
                show_hidden: false,
                default_sort: StartupSort::Natural,
                default_sort_reverse: false,
            },
            search: SearchConfig {
                global_search_limit: 200,
                global_search_chunk_size: 24,
                show_loading: true,
            },
            behavior: BehaviorConfig {
                cancel_search_on_leave: true,
            },
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

/// 表示新版設定檔的原始格式。
///
/// 新版配置會盡量按功能分區，讓未來擴充時不容易失控。
#[derive(Debug, Default, Deserialize)]
struct AppConfigFile {
    ui: Option<UiConfigFile>,
    pane: Option<PaneConfigFile>,
    search: Option<SearchConfigFile>,
    behavior: Option<BehaviorConfigFile>,
}

/// 表示舊版平鋪設定檔格式。
///
/// 這個型別只為了相容舊檔案存在，未來說明文件將以新版分區格式為主。
#[derive(Debug, Default, Deserialize)]
struct LegacyAppConfigFile {
    theme: Option<String>,
    poll_rate_ms: Option<u64>,
    show_hidden: Option<bool>,
    default_sort: Option<String>,
    default_sort_reverse: Option<bool>,
    preview: Option<PreviewConfigFile>,
    confirm_dialog: Option<DialogConfigFileRaw>,
    theme_picker: Option<DialogConfigFileRaw>,
}

/// 表示 `ui` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct UiConfigFile {
    theme: Option<String>,
    poll_rate_ms: Option<u64>,
    preview: Option<PreviewConfigFile>,
    dialog: Option<DialogsConfigFile>,
}

/// 表示 `pane` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct PaneConfigFile {
    show_hidden: Option<bool>,
    default_sort: Option<String>,
    default_sort_reverse: Option<bool>,
}

/// 表示 `search` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct SearchConfigFile {
    global_search_limit: Option<usize>,
    global_search_chunk_size: Option<usize>,
    show_loading: Option<bool>,
}

/// 表示 `behavior` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct BehaviorConfigFile {
    cancel_search_on_leave: Option<bool>,
}

/// 表示所有 dialog 群組的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct DialogsConfigFile {
    confirm: Option<DialogConfigFileRaw>,
    theme_picker: Option<DialogConfigFileRaw>,
}

/// 表示設定檔中 popup 類視窗的尺寸設定區塊。
#[derive(Debug, Default, Deserialize)]
struct DialogConfigFileRaw {
    width_percent: Option<u16>,
    height: Option<u16>,
}

/// 表示 preview 區塊的高度設定。
#[derive(Debug, Default, Deserialize)]
struct PreviewConfigFile {
    height: Option<u16>,
    focus_list_height: Option<u16>,
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

    let mut config = AppConfig::default();
    apply_new_file(
        &mut config,
        toml::from_str::<AppConfigFile>(&contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?,
    )?;
    apply_legacy_file(
        &mut config,
        toml::from_str::<LegacyAppConfigFile>(&contents)
            .with_context(|| format!("failed to parse legacy config file {}", path.display()))?,
    )?;

    Ok(LoadedConfig {
        config,
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

    if let Some(path) = env::var_os("TFM_CONFIG") {
        paths.push(PathBuf::from(path));
    }

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

/// 將新版分區設定套用到執行期設定。
///
/// 參數：
/// - `config: &mut AppConfig`，要被更新的設定。
/// - `file: AppConfigFile`，新版分區設定檔內容。
///
/// 回傳：`Result<()>`。
fn apply_new_file(config: &mut AppConfig, file: AppConfigFile) -> Result<()> {
    if let Some(ui) = file.ui {
        apply_ui_config(config, ui)?;
    }
    if let Some(pane) = file.pane {
        apply_pane_config(config, pane)?;
    }
    if let Some(search) = file.search {
        apply_search_config(config, search)?;
    }
    if let Some(behavior) = file.behavior {
        apply_behavior_config(config, behavior);
    }
    Ok(())
}

/// 套用舊版平鋪設定，保留向下相容能力。
///
/// 參數：
/// - `config: &mut AppConfig`，要被更新的設定。
/// - `file: LegacyAppConfigFile`，舊版平鋪設定檔內容。
///
/// 回傳：`Result<()>`。
fn apply_legacy_file(config: &mut AppConfig, file: LegacyAppConfigFile) -> Result<()> {
    let mut ui = UiConfigFile::default();
    ui.theme = file.theme;
    ui.poll_rate_ms = file.poll_rate_ms;
    ui.preview = file.preview;
    if file.confirm_dialog.is_some() || file.theme_picker.is_some() {
        ui.dialog = Some(DialogsConfigFile {
            confirm: file.confirm_dialog,
            theme_picker: file.theme_picker,
        });
    }
    apply_ui_config(config, ui)?;

    apply_pane_config(
        config,
        PaneConfigFile {
            show_hidden: file.show_hidden,
            default_sort: file.default_sort,
            default_sort_reverse: file.default_sort_reverse,
        },
    )?;

    Ok(())
}

/// 套用並驗證 `ui` 區塊設定。
fn apply_ui_config(config: &mut AppConfig, ui: UiConfigFile) -> Result<()> {
    if let Some(name) = ui.theme {
        config.ui.theme_preset = ThemePreset::from_name(name.trim())
            .with_context(|| format!("unknown theme preset: {}", name.trim()))?;
    }

    if let Some(poll_rate_ms) = ui.poll_rate_ms {
        if poll_rate_ms == 0 {
            bail!("ui.poll_rate_ms must be greater than 0");
        }
        config.ui.poll_rate = Duration::from_millis(poll_rate_ms);
    }

    if let Some(preview) = ui.preview {
        apply_preview_config(&mut config.ui.preview, preview)?;
    }

    if let Some(dialogs) = ui.dialog {
        if let Some(confirm) = dialogs.confirm {
            apply_dialog_config(&mut config.ui.dialogs.confirm, confirm, "ui.dialog.confirm")?;
        }
        if let Some(theme_picker) = dialogs.theme_picker {
            apply_dialog_config(
                &mut config.ui.dialogs.theme_picker,
                theme_picker,
                "ui.dialog.theme_picker",
            )?;
        }
    }

    Ok(())
}

/// 套用並驗證 `pane` 區塊設定。
fn apply_pane_config(config: &mut AppConfig, pane: PaneConfigFile) -> Result<()> {
    if let Some(show_hidden) = pane.show_hidden {
        config.pane.show_hidden = show_hidden;
    }

    if let Some(name) = pane.default_sort {
        config.pane.default_sort = StartupSort::from_name(name.trim()).with_context(|| {
            format!(
                "unknown pane.default_sort: {}. available: alphabetical, natural, size, modified, created, extension, random",
                name.trim()
            )
        })?;
    }

    if let Some(reverse) = pane.default_sort_reverse {
        config.pane.default_sort_reverse = reverse;
    }

    Ok(())
}

/// 套用並驗證 `search` 區塊設定。
fn apply_search_config(config: &mut AppConfig, search: SearchConfigFile) -> Result<()> {
    if let Some(limit) = search.global_search_limit {
        if limit == 0 {
            bail!("search.global_search_limit must be greater than 0");
        }
        config.search.global_search_limit = limit;
    }

    if let Some(chunk_size) = search.global_search_chunk_size {
        if chunk_size == 0 {
            bail!("search.global_search_chunk_size must be greater than 0");
        }
        config.search.global_search_chunk_size = chunk_size;
    }

    if let Some(show_loading) = search.show_loading {
        config.search.show_loading = show_loading;
    }

    Ok(())
}

/// 套用 `behavior` 區塊設定。
fn apply_behavior_config(config: &mut AppConfig, behavior: BehaviorConfigFile) {
    if let Some(cancel_search_on_leave) = behavior.cancel_search_on_leave {
        config.behavior.cancel_search_on_leave = cancel_search_on_leave;
    }
}

/// 套用並驗證 preview 區塊的高度設定。
fn apply_preview_config(config: &mut PreviewConfig, preview: PreviewConfigFile) -> Result<()> {
    if let Some(height) = preview.height {
        if !(4..=30).contains(&height) {
            bail!("ui.preview.height must be between 4 and 30");
        }
        config.height = height;
    }

    if let Some(height) = preview.focus_list_height {
        if !(3..=20).contains(&height) {
            bail!("ui.preview.focus_list_height must be between 3 and 20");
        }
        config.focus_list_height = height;
    }

    Ok(())
}

/// 套用並驗證一組 popup 類視窗的尺寸設定。
fn apply_dialog_config(
    dialog: &mut DialogConfig,
    file: DialogConfigFileRaw,
    field_name: &str,
) -> Result<()> {
    if let Some(value) = file.width_percent {
        if !(1..=100).contains(&value) {
            bail!("{field_name}.width_percent must be between 1 and 100");
        }
        dialog.width_percent = value;
    }

    if let Some(value) = file.height {
        if value == 0 {
            bail!("{field_name}.height must be greater than 0");
        }
        dialog.height = value;
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
    fn load_config_returns_defaults_when_missing() {
        let dir = tempdir().expect("tempdir");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.config, AppConfig::default());
        assert!(loaded.source.is_none());
    }

    #[test]
    /// 驗證新版分區式 `config.toml` 內容可以正確解析成設定。
    fn load_config_reads_project_file() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("config.toml"),
            r#"
[ui]
theme = "forest"
poll_rate_ms = 90

[ui.preview]
height = 11
focus_list_height = 5

[ui.dialog.confirm]
width_percent = 66
height = 7

[ui.dialog.theme_picker]
width_percent = 55
height = 9

[pane]
show_hidden = true
default_sort = "size"
default_sort_reverse = true

[search]
global_search_limit = 120
global_search_chunk_size = 16
show_loading = false

[behavior]
cancel_search_on_leave = false
"#,
        )
        .expect("config file");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.config.ui.theme_preset, ThemePreset::Forest);
        assert_eq!(loaded.config.ui.poll_rate, Duration::from_millis(90));
        assert_eq!(loaded.config.ui.preview.height, 11);
        assert_eq!(loaded.config.ui.preview.focus_list_height, 5);
        assert_eq!(loaded.config.ui.dialogs.confirm.width_percent, 66);
        assert_eq!(loaded.config.ui.dialogs.confirm.height, 7);
        assert_eq!(loaded.config.ui.dialogs.theme_picker.width_percent, 55);
        assert_eq!(loaded.config.ui.dialogs.theme_picker.height, 9);
        assert!(loaded.config.pane.show_hidden);
        assert_eq!(loaded.config.pane.default_sort, StartupSort::Size);
        assert!(loaded.config.pane.default_sort_reverse);
        assert_eq!(loaded.config.search.global_search_limit, 120);
        assert_eq!(loaded.config.search.global_search_chunk_size, 16);
        assert!(!loaded.config.search.show_loading);
        assert!(!loaded.config.behavior.cancel_search_on_leave);
        assert_eq!(loaded.source, Some(dir.path().join("config.toml")));
    }

    #[test]
    /// 驗證舊版平鋪設定格式仍可繼續使用。
    fn load_config_reads_legacy_flat_keys() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("config.toml"),
            r#"
theme = "ocean"
poll_rate_ms = 80
show_hidden = true
default_sort = "extension"
default_sort_reverse = true

[preview]
height = 10
focus_list_height = 4

[confirm_dialog]
width_percent = 58
height = 6

[theme_picker]
width_percent = 44
height = 9
"#,
        )
        .expect("config file");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.config.ui.theme_preset, ThemePreset::Ocean);
        assert_eq!(loaded.config.ui.poll_rate, Duration::from_millis(80));
        assert!(loaded.config.pane.show_hidden);
        assert_eq!(loaded.config.pane.default_sort, StartupSort::Extension);
        assert!(loaded.config.pane.default_sort_reverse);
        assert_eq!(loaded.config.ui.preview.height, 10);
        assert_eq!(loaded.config.ui.preview.focus_list_height, 4);
        assert_eq!(loaded.config.ui.dialogs.confirm.width_percent, 58);
        assert_eq!(loaded.config.ui.dialogs.confirm.height, 6);
        assert_eq!(loaded.config.ui.dialogs.theme_picker.width_percent, 44);
        assert_eq!(loaded.config.ui.dialogs.theme_picker.height, 9);
    }
}
