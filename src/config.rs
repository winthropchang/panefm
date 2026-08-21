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
    pub navigation: NavigationConfig,
    pub behavior: BehaviorConfig,
    pub actions: ActionsConfig,
}

/// 表示 UI 相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiConfig {
    pub theme_preset: ThemePreset,
    pub icons: IconsConfig,
    pub poll_rate: Duration,
    pub preview: PreviewConfig,
    pub dialogs: DialogsConfig,
}

/// 表示檔案列表圖示的顯示設定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconsConfig {
    /// 是否在檔名左側顯示跨平台 Unicode 圖示。
    pub enabled: bool,
    /// 圖示字元風格；`nerd-font` 接近 mature-reference，`ascii` 不依賴特殊字型。
    pub style: IconStyle,
}

/// 表示列表圖示所使用的字元集合。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconStyle {
    NerdFont,
    Ascii,
}

impl IconStyle {
    /// 將設定檔中的文字轉成圖示風格。
    ///
    /// 參數：
    /// - `name: &str`，設定檔中的圖示風格名稱。
    ///
    /// 回傳：`Option<IconStyle>`，名稱有效時回傳對應風格。
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "nerd-font" | "nerdfont" | "nerd" => Some(Self::NerdFont),
            "ascii" | "plain" => Some(Self::Ascii),
            _ => None,
        }
    }
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
    pub fzf_follow_links: bool,
}

/// 表示列表導航手感相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationConfig {
    pub fast_move_step: usize,
    pub panel_page_step: usize,
}

/// 表示互動行為相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BehaviorConfig {
    pub cancel_search_on_leave: bool,
}

/// 表示使用者在設定檔中定義的外部動作集合。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionsConfig {
    pub open_with: Vec<CustomOpenActionConfig>,
}

/// 表示單一自訂外部動作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomOpenActionConfig {
    pub name: String,
    pub scope: ActionTargetScope,
    pub mode: ActionLaunchMode,
    pub command: Option<String>,
    pub mac_command: Option<String>,
    pub windows_command: Option<String>,
}

/// 描述自訂動作適用於檔案、資料夾或兩者。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionTargetScope {
    File,
    Directory,
    Both,
}

/// 描述自訂動作要阻塞在終端中執行，還是背景分離執行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionLaunchMode {
    TerminalBlocking,
    Detached,
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
                theme_preset: ThemePreset::CatppuccinMocha,
                icons: IconsConfig {
                    enabled: true,
                    style: IconStyle::NerdFont,
                },
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
                        height: 20,
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
                fzf_follow_links: true,
            },
            navigation: NavigationConfig {
                fast_move_step: 5,
                panel_page_step: 10,
            },
            behavior: BehaviorConfig {
                cancel_search_on_leave: true,
            },
            actions: ActionsConfig {
                open_with: Vec::new(),
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

/// 將目前選取的主題名稱同步寫入設定檔。
///
/// 參數：
/// - `path: &Path`，要更新的 `config.toml` 路徑。
/// - `preset: ThemePreset`，要保存的主題預設值。
///
/// 回傳：`Result<()>`，成功寫入或建立設定檔時回傳 `Ok(())`。
///
/// 這個函數只修改 `[ui]` 區塊中的 `theme` 欄位，其他設定、註解與格式都會保留。
pub fn persist_theme(path: &Path, preset: ThemePreset) -> Result<()> {
    let theme_line = format!("theme = \"{}\"", preset.name());
    let contents = if path.exists() {
        fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?
    } else {
        String::new()
    };

    let mut output = String::new();
    let mut in_ui = false;
    let mut replaced = false;
    let mut has_ui = false;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_ui = trimmed == "[ui]";
            has_ui |= in_ui;
        }

        if in_ui && trimmed.starts_with("theme") && trimmed[5..].trim_start().starts_with('=') {
            let indentation = &line[..line.len() - line.trim_start().len()];
            output.push_str(indentation);
            output.push_str(&theme_line);
            output.push('\n');
            replaced = true;
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    if !replaced {
        if has_ui {
            let mut lines = output.lines().map(str::to_owned).collect::<Vec<_>>();
            let insert_at = lines
                .iter()
                .position(|line| line.trim() == "[ui]")
                .map(|index| index + 1)
                .unwrap_or(0);
            lines.insert(insert_at, theme_line);
            output = lines.join("\n");
            output.push('\n');
        } else {
            output = format!("[ui]\n{theme_line}\n{output}");
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    fs::write(path, output)
        .with_context(|| format!("failed to write config file {}", path.display()))?;
    Ok(())
}

/// 表示新版設定檔的原始格式。
///
/// 新版配置會盡量按功能分區，讓未來擴充時不容易失控。
#[derive(Debug, Default, Deserialize)]
struct AppConfigFile {
    ui: Option<UiConfigFile>,
    pane: Option<PaneConfigFile>,
    search: Option<SearchConfigFile>,
    navigation: Option<NavigationConfigFile>,
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
    icons: Option<IconsConfigFile>,
    poll_rate_ms: Option<u64>,
    preview: Option<PreviewConfigFile>,
    dialog: Option<DialogsConfigFile>,
}

/// 表示 `[ui.icons]` 在 TOML 中的可選設定欄位。
#[derive(Debug, Default, Deserialize)]
struct IconsConfigFile {
    enabled: Option<bool>,
    style: Option<String>,
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
    fzf_follow_links: Option<bool>,
}

/// 表示 `navigation` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct NavigationConfigFile {
    fast_move_step: Option<usize>,
    panel_page_step: Option<usize>,
}

/// 表示 `behavior` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct BehaviorConfigFile {
    cancel_search_on_leave: Option<bool>,
}

/// 表示 `plugins.toml` 的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct PluginsConfigFile {
    actions: Option<ActionsConfigFile>,
}

/// 表示 `actions` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct ActionsConfigFile {
    open_with: Option<Vec<CustomOpenActionFile>>,
}

/// 表示單一自訂動作在設定檔中的原始欄位。
#[derive(Debug, Default, Deserialize)]
struct CustomOpenActionFile {
    name: Option<String>,
    scope: Option<String>,
    mode: Option<String>,
    command: Option<String>,
    mac_command: Option<String>,
    windows_command: Option<String>,
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
    let mut config = AppConfig::default();
    let config_source = config_search_paths(base_dir)
        .into_iter()
        .find(|path| path.exists());

    if let Some(path) = config_source.as_ref() {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;

        apply_new_file(
            &mut config,
            toml::from_str::<AppConfigFile>(&contents)
                .with_context(|| format!("failed to parse config file {}", path.display()))?,
        )?;
        apply_legacy_file(
            &mut config,
            toml::from_str::<LegacyAppConfigFile>(&contents).with_context(|| {
                format!("failed to parse legacy config file {}", path.display())
            })?,
        )?;
    }

    if let Some(path) = plugins_search_paths(base_dir, config_source.as_deref())
        .into_iter()
        .find(|path| path.exists())
    {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read plugins file {}", path.display()))?;
        let plugins = toml::from_str::<PluginsConfigFile>(&contents)
            .with_context(|| format!("failed to parse plugins file {}", path.display()))?;
        if let Some(actions) = plugins.actions {
            apply_actions_config(&mut config, actions)?;
        }
    }

    Ok(LoadedConfig {
        config,
        source: config_source,
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

/// 建立 `plugins.toml` 的搜尋路徑清單。
///
/// 參數：
/// - `base_dir: &Path`，目前專案目錄。
/// - `config_source: Option<&Path>`，已找到的 `config.toml` 路徑，用來推導同層的 `plugins.toml`。
///
/// 回傳：`Vec<PathBuf>`，依照優先順序排列的候選 plugins 檔案路徑。
fn plugins_search_paths(base_dir: &Path, config_source: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(path) = env::var_os("TFM_PLUGINS") {
        paths.push(PathBuf::from(path));
    }

    if let Some(config_path) = config_source {
        if let Some(parent) = config_path.parent() {
            paths.push(parent.join("plugins.toml"));
        }
    } else {
        paths.push(base_dir.join("plugins.toml"));
    }

    if let Some(xdg_home) = env::var_os("XDG_CONFIG_HOME") {
        paths.push(
            PathBuf::from(xdg_home)
                .join("terminal-file-manager")
                .join("plugins.toml"),
        );
    }

    if let Some(home) = env::var_os("HOME") {
        paths.push(
            PathBuf::from(home)
                .join(".config")
                .join("terminal-file-manager")
                .join("plugins.toml"),
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
    if let Some(navigation) = file.navigation {
        apply_navigation_config(config, navigation)?;
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

    if let Some(icons) = ui.icons {
        if let Some(enabled) = icons.enabled {
            config.ui.icons.enabled = enabled;
        }
        if let Some(style) = icons.style {
            config.ui.icons.style = IconStyle::from_name(&style)
                .with_context(|| format!("unknown ui.icons.style: {}", style.trim()))?;
        }
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

    if let Some(fzf_follow_links) = search.fzf_follow_links {
        config.search.fzf_follow_links = fzf_follow_links;
    }

    Ok(())
}

/// 套用 `behavior` 區塊設定。
fn apply_behavior_config(config: &mut AppConfig, behavior: BehaviorConfigFile) {
    if let Some(cancel_search_on_leave) = behavior.cancel_search_on_leave {
        config.behavior.cancel_search_on_leave = cancel_search_on_leave;
    }
}

/// 套用並驗證 `actions` 區塊設定。
fn apply_actions_config(config: &mut AppConfig, actions: ActionsConfigFile) -> Result<()> {
    let Some(raw_actions) = actions.open_with else {
        return Ok(());
    };

    let mut parsed = Vec::with_capacity(raw_actions.len());
    for (index, raw) in raw_actions.into_iter().enumerate() {
        let name = raw
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .with_context(|| format!("actions.open_with[{index}].name is required"))?;

        if raw.command.as_deref().is_none()
            && raw.mac_command.as_deref().is_none()
            && raw.windows_command.as_deref().is_none()
        {
            bail!(
                "actions.open_with[{index}] must define at least one of command / mac_command / windows_command"
            );
        }

        let scope = match raw
            .scope
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
        {
            None => ActionTargetScope::Both,
            Some(value) if value == "both" => ActionTargetScope::Both,
            Some(value) if value == "file" => ActionTargetScope::File,
            Some(value) if value == "dir" || value == "directory" => ActionTargetScope::Directory,
            Some(value) => {
                bail!(
                    "unknown actions.open_with[{index}].scope: {value}. available: file, dir, both"
                );
            }
        };

        let mode = match raw
            .mode
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
        {
            None => ActionLaunchMode::Detached,
            Some(value) if value == "detached" => ActionLaunchMode::Detached,
            Some(value) if value == "terminal" || value == "terminal_blocking" => {
                ActionLaunchMode::TerminalBlocking
            }
            Some(value) => {
                bail!(
                    "unknown actions.open_with[{index}].mode: {value}. available: detached, terminal"
                );
            }
        };

        parsed.push(CustomOpenActionConfig {
            name,
            scope,
            mode,
            command: raw.command.map(|value| value.trim().to_string()),
            mac_command: raw.mac_command.map(|value| value.trim().to_string()),
            windows_command: raw.windows_command.map(|value| value.trim().to_string()),
        });
    }

    config.actions.open_with = parsed;
    Ok(())
}

/// 套用並驗證 `navigation` 區塊設定。
fn apply_navigation_config(config: &mut AppConfig, navigation: NavigationConfigFile) -> Result<()> {
    if let Some(value) = navigation.fast_move_step {
        if value == 0 {
            bail!("navigation.fast_move_step must be greater than 0");
        }
        config.navigation.fast_move_step = value;
    }

    if let Some(value) = navigation.panel_page_step {
        if value == 0 {
            bail!("navigation.panel_page_step must be greater than 0");
        }
        config.navigation.panel_page_step = value;
    }

    Ok(())
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
    /// 驗證保存主題時只更新 `[ui]` 的 `theme`，並保留其他設定與註解。
    fn persist_theme_updates_only_ui_theme() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "# keep this comment\n[ui]\ntheme = \"dracula\"\npoll_rate_ms = 90\n\n[pane]\ntheme = \"not-a-ui-theme\"\n",
        )
        .expect("write config");

        persist_theme(&path, ThemePreset::Nord).expect("persist theme");

        let contents = fs::read_to_string(path).expect("read config");
        assert!(contents.contains("# keep this comment"));
        assert!(contents.contains("theme = \"nord\""));
        assert!(contents.contains("poll_rate_ms = 90"));
        assert!(contents.contains("[pane]\ntheme = \"not-a-ui-theme\""));
    }

    #[test]
    /// 驗證沒有設定檔時保存主題會建立最小可讀的 `config.toml`。
    fn persist_theme_creates_missing_config() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        persist_theme(&path, ThemePreset::Kanagawa).expect("persist theme");

        assert_eq!(fs::read_to_string(path).expect("read config"), "[ui]\ntheme = \"kanagawa\"\n");
    }

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

[ui.icons]
enabled = false

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
fzf_follow_links = false

[navigation]
fast_move_step = 7
panel_page_step = 14

[behavior]
cancel_search_on_leave = false
"#,
        )
        .expect("config file");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.config.ui.theme_preset, ThemePreset::Everforest);
        assert!(!loaded.config.ui.icons.enabled);
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
        assert!(!loaded.config.search.fzf_follow_links);
        assert_eq!(loaded.config.navigation.fast_move_step, 7);
        assert_eq!(loaded.config.navigation.panel_page_step, 14);
        assert!(!loaded.config.behavior.cancel_search_on_leave);
        assert!(loaded.config.actions.open_with.is_empty());
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

        assert_eq!(loaded.config.ui.theme_preset, ThemePreset::Nord);
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
        assert!(loaded.config.actions.open_with.is_empty());
    }

    #[test]
    /// 驗證 `plugins.toml` 中的 `actions.open_with` 會正確載入自訂外部動作設定。
    fn load_config_reads_custom_open_actions_from_plugins_file() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("plugins.toml"),
            r#"
[actions]

[[actions.open_with]]
name = "Xcode"
scope = "dir"
mode = "detached"
mac_command = "open -a Xcode {path}"

[[actions.open_with]]
name = "Git log"
scope = "both"
mode = "terminal"
command = "git -C {parent} log --oneline"
windows_command = "git -C {parent} log --oneline"
"#,
        )
        .expect("config file");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.config.actions.open_with.len(), 2);
        assert_eq!(loaded.config.actions.open_with[0].name, "Xcode");
        assert_eq!(
            loaded.config.actions.open_with[0].scope,
            ActionTargetScope::Directory
        );
        assert_eq!(
            loaded.config.actions.open_with[1].mode,
            ActionLaunchMode::TerminalBlocking
        );
        assert_eq!(
            loaded.config.actions.open_with[1].command.as_deref(),
            Some("git -C {parent} log --oneline")
        );
    }

    #[test]
    /// 驗證即使沒有 `config.toml`，只要有 `plugins.toml` 也能載入自訂動作。
    fn load_config_reads_plugins_without_main_config() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("plugins.toml"),
            r#"
[actions]

[[actions.open_with]]
name = "Reveal in Finder"
scope = "both"
mode = "detached"
mac_command = "open -R {path}"
"#,
        )
        .expect("plugins file");

        let loaded = load_config(dir.path()).expect("config");

        assert_eq!(loaded.source, None);
        assert_eq!(loaded.config.actions.open_with.len(), 1);
        assert_eq!(loaded.config.actions.open_with[0].name, "Reveal in Finder");
    }

    #[test]
    /// 驗證 navigation 設定值不可為 0，避免快捷移動完全失效。
    fn load_config_rejects_zero_navigation_steps() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("config.toml"),
            r#"
[navigation]
fast_move_step = 0
"#,
        )
        .expect("config file");

        let error = load_config(dir.path()).expect_err("should reject zero step");
        assert!(
            error
                .to_string()
                .contains("navigation.fast_move_step must be greater than 0")
        );
    }

    #[test]
    /// 驗證字體設定不再是程式設定的一部分，避免使用者誤以為 TUI 能改外部 Terminal 字體。
    fn load_config_ignores_removed_font_settings() {
        let dir = tempdir().expect("tempdir");
        fs::write(
            dir.path().join("config.toml"),
            r#"
[ui.font]
enabled = true
family = "Some Font"
size = 42
"#,
        )
        .expect("config file");

        let loaded = load_config(dir.path()).expect("config should remain compatible");
        assert_eq!(loaded.config, AppConfig::default());
    }
}
