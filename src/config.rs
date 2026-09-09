//! 設定檔模型、預設值、讀取順序與持久化邏輯。
//!
//! PaneFM 會把穩定的使用者偏好放在 `config.toml`，把可自行擴充的外部動作放在
//! `plugins.toml`。本模組只負責解析與驗證；檔案管理行為由 `file_manager` 套用，
//! 因此新增設定時應同時補上預設值、反序列化相容處理與測試。

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

/// 描述啟動時右側欄位預設要顯示的資訊模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupLinemode {
    Mtime,
    Btime,
    Size,
    Permissions,
    None,
}

impl StartupLinemode {
    /// 將設定檔中的文字名稱轉成對應的 linemode 種類。
    ///
    /// 參數：
    /// - `name: &str`，設定檔中寫的 linemode 名稱。
    ///
    /// 回傳：`Option<StartupLinemode>`。
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "mtime" | "modified" => Some(Self::Mtime),
            "btime" | "created" | "birth" => Some(Self::Btime),
            "size" => Some(Self::Size),
            "permissions" | "perms" | "perm" => Some(Self::Permissions),
            "none" | "off" => Some(Self::None),
            _ => None,
        }
    }

    /// 回傳適合顯示在說明文件中的名稱。
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mtime => "mtime",
            Self::Btime => "btime",
            Self::Size => "size",
            Self::Permissions => "permissions",
            Self::None => "none",
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
    pub watcher: WatcherConfig,
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
    /// 圖示字元風格；`nerd-font` 提供緊湊圖示，`ascii` 不依賴特殊字型。
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
    pub default_linemode: StartupLinemode,
}

/// 表示搜尋行為相關的設定群組。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchConfig {
    pub global_search_limit: usize,
    pub global_search_chunk_size: usize,
    pub show_loading: bool,
    pub fzf_follow_links: bool,
}

/// 表示外部檔案系統變更的自動刷新設定。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatcherConfig {
    /// 是否監看目前所有 panel 的目錄。
    pub enabled: bool,
    /// 同一批檔案事件合併後再刷新列表的等待時間。
    pub debounce: Duration,
    /// SMB 等無法可靠送出原生事件時，輪詢 fallback 的掃描間隔。
    pub fallback_poll_interval: Duration,
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
    /// 可選的新終端啟動器；公司保護環境可在 plugins.toml 指定 TrustView 等入口。
    pub terminal: Option<TerminalLauncherConfig>,
    /// 自訂或擴充的終端適配器清單。
    pub terminals: Vec<TerminalPluginConfig>,
}

/// 表示 `plugins.toml` 中可覆寫的平台終端啟動命令。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLauncherConfig {
    pub command: Option<String>,
    pub mac_command: Option<String>,
    pub windows_command: Option<String>,
}

/// 表示 `plugins.toml` 中自訂的終端適配器（Terminal Plugin）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPluginConfig {
    pub name: String,
    pub match_env: Vec<String>,
    pub match_process: Vec<String>,
    pub command: Option<String>,
    pub mac_command: Option<String>,
    pub windows_command: Option<String>,
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
                default_linemode: StartupLinemode::Mtime,
            },
            search: SearchConfig {
                global_search_limit: 200,
                global_search_chunk_size: 24,
                show_loading: true,
                fzf_follow_links: true,
            },
            watcher: WatcherConfig {
                enabled: true,
                debounce: Duration::from_millis(120),
                fallback_poll_interval: Duration::from_millis(2_000),
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
                terminal: None,
                terminals: Vec::new(),
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
    pub base_dir: PathBuf,
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
    watcher: Option<WatcherConfigFile>,
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
    default_linemode: Option<String>,
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
    default_linemode: Option<String>,
}

/// 表示 `search` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
struct SearchConfigFile {
    global_search_limit: Option<usize>,
    global_search_chunk_size: Option<usize>,
    show_loading: Option<bool>,
    fzf_follow_links: Option<bool>,
}

/// 表示 `[watcher]` 區塊中尚未驗證的可選欄位。
#[derive(Debug, Default, Deserialize)]
struct WatcherConfigFile {
    enabled: Option<bool>,
    debounce_ms: Option<u64>,
    fallback_poll_interval_ms: Option<u64>,
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
    terminal: Option<TerminalLauncherFile>,
    terminals: Option<Vec<TerminalPluginFile>>,
}

/// 表示 plugins.toml `[terminal]` 區塊尚未驗證的原始欄位。
#[derive(Debug, Default, Deserialize)]
struct TerminalLauncherFile {
    command: Option<String>,
    mac_command: Option<String>,
    windows_command: Option<String>,
}

/// 表示 plugins.toml `[[terminals]]` 區塊尚未驗證的原始欄位。
#[derive(Debug, Default, Deserialize)]
struct TerminalPluginFile {
    name: Option<String>,
    match_env: Option<Vec<String>>,
    match_process: Option<Vec<String>>,
    command: Option<String>,
    mac_command: Option<String>,
    windows_command: Option<String>,
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
        if let Some(terminal) = plugins.terminal {
            apply_terminal_launcher_config(&mut config, terminal)?;
        }
        if let Some(terminals) = plugins.terminals {
            apply_terminal_plugins_config(&mut config, terminals)?;
        }
    }

    Ok(LoadedConfig {
        config,
        source: config_source,
        base_dir: base_dir.to_path_buf(),
    })
}

/// 建立設定檔搜尋路徑清單。
///
/// 參數：
/// - `base_dir: &Path`，目前應用程式/可執行檔目錄。
///
/// 回傳：`Vec<PathBuf>`，依照優先順序排列的候選設定檔路徑。
fn config_search_paths(base_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(path) = env::var_os("PANE_FM_CONFIG") {
        paths.push(PathBuf::from(path));
    }

    if let Some(path) = env::var_os("TFM_CONFIG") {
        paths.push(PathBuf::from(path));
    }

    if !base_dir.as_os_str().is_empty() {
        paths.push(base_dir.join("config.toml"));
    }

    if let Some(xdg_home) = env::var_os("XDG_CONFIG_HOME") {
        let xdg_home = PathBuf::from(xdg_home);
        paths.push(app_config_file(&xdg_home, "panefm", "config.toml"));
        paths.push(app_config_file(
            &xdg_home,
            "terminal-file-manager",
            "config.toml",
        ));
    }

    if let Some(home) = env::var_os("HOME") {
        let config_home = PathBuf::from(home).join(".config");
        paths.push(app_config_file(&config_home, "panefm", "config.toml"));
        paths.push(app_config_file(
            &config_home,
            "terminal-file-manager",
            "config.toml",
        ));
    }

    if let Some(app_data) = env::var_os("APPDATA") {
        let app_data = PathBuf::from(app_data);
        paths.push(app_config_file(&app_data, "panefm", "config.toml"));
        paths.push(app_config_file(
            &app_data,
            "terminal-file-manager",
            "config.toml",
        ));
    }

    paths
}

/// 內建預設的 `config.toml` 內容模板，附帶完整區塊與繁體中文詳細註解。
pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# PaneFM (Pane File Manager) 設定檔
# 所有欄位皆具備預設值；您可在此自訂操作習慣、色彩主題與面板行為。

[ui]
# 介面色彩主題。
# 內建 10 款主題：
#   "catppuccin-mocha"（預設）、"dracula"、"tokyo-night"、"gruvbox"、"nord"、
#   "everforest"、"rose-pine"、"solarized-dark"、"monokai"、"one-dark"
theme = "catppuccin-mocha"

# 終端事件輪詢頻率（毫秒），數值越小反應越靈敏，120ms 為兼顧流暢與低 CPU 負擔。
poll_rate_ms = 120

[ui.icons]
# 是否在檔名左側顯示圖示。
enabled = true
# 圖示風格：
#   - "nerd-font": 精美現代的終端圖示（終端機需搭配 Nerd Font 字型）
#   - "ascii"    : 純文字方括號圖示 [D] [F] [S]，相容所有終端字型
style = "nerd-font"

[ui.preview]
# 底部快速預覽視窗開啟時的預設高度（列數）。
height = 8
# 預覽開啟時，主清單可見的最小列數。
focus_list_height = 6

[ui.dialog.confirm]
# 確認對話框（例如刪除檔案時）的寬度百分比與高度。
width_percent = 60
height = 5

[ui.dialog.theme_picker]
# 主題選擇器彈窗的寬度百分比與高度。
width_percent = 42
height = 20

[pane]
# 是否在啟動時預設顯示隱藏檔案與目錄（. 開頭）。
show_hidden = false

# 啟動時各面板的預設排序方式。
# 可選值：
#   - "natural"     : 自然名稱排序（預設，英數混合自然排列）
#   - "modified"    : 依最後修改時間排序
#   - "created"     : 依建立時間排序
#   - "size"        : 依檔案容量大小排序
#   - "extension"   : 依副檔名排序
#   - "alphabetical": 傳統純字母排序
#   - "random"      : 隨機打亂排序
default_sort = "natural"

# 是否反向排序（true: 新->舊 / 大->小 / Z->A；false: 正常由小到大）。
default_sort_reverse = false

# 列表右側欄位預設顯示的資訊類型。
# 可選值：
#   - "mtime"      : 顯示最後修改時間（預設，格式為 MM/DD HH:MM）
#   - "btime"      : 顯示建立時間 (Birth time)
#   - "size"       : 顯示檔案大小容量 (B/K/M/G/T)
#   - "permissions": 顯示 Unix/跨平台權限標記 (rwx / readonly)
#   - "none"       : 不顯示右側欄位，享受最寬敞純淨的檔名空間
default_linemode = "mtime"

[navigation]
# 使用 Shift+J / Shift+K 快速大步移動時的單次跳躍列數。
fast_move_step = 5

# 使用 Ctrl+D / Ctrl+U 翻頁捲動時的移動列數。
panel_page_step = 10

[search]
# 全域內容全文搜尋 (rg) 時，最多載入的符合項目上限。
global_search_limit = 200

# 全域搜尋分批回傳的項目批次大小。
global_search_chunk_size = 24

# 搜尋處理時是否在 Pane 標題顯示旋轉 Loading 動畫。
show_loading = true

# 使用 z (fzf) 模糊跳轉目錄時，是否追蹤符號連結 (Symlink)。
fzf_follow_links = true

[watcher]
# 當外部程式（如 VS Code、Git、檔案總管）更動目錄時，是否自動刷新目前 Pane。
enabled = true

# 連續檔案更動事件的防抖合併延遲（毫秒），避免連環事件造成介面閃爍。
debounce_ms = 120

# SMB 掛載點等無法送出原生事件時，後備輪詢掃描間隔（毫秒）。
fallback_poll_interval_ms = 2000

[behavior]
# 當游標離開搜尋結果面板時，是否自動清空搜尋關鍵字並還原原目錄。
cancel_search_on_leave = true
"#;

/// 檢查目前環境是否已有任何有效的 `config.toml`；若無，則自動在適當位置建立預設設定檔。
///
/// 優先順序：
/// 1. 若可執行檔所在目錄可寫入，直接在該處建立 `config.toml`。
/// 2. 若不可寫入（如位於系統保護目錄），則回退到使用者設定目錄：
///    - Windows: `%APPDATA%\panefm\config.toml`
///    - macOS/Linux: `~/.config/panefm/config.toml` 或 `$XDG_CONFIG_HOME/panefm/config.toml`
///
/// 回傳：`Option<PathBuf>`，成功建立時回傳該檔案路徑；若已存在或無法寫入則回傳 `None`。
pub fn ensure_default_config_file(base_dir: &Path) -> Option<PathBuf> {
    if config_search_paths(base_dir)
        .into_iter()
        .any(|p| p.exists())
    {
        return None;
    }

    let candidates = default_config_creation_candidates(base_dir);
    for candidate in candidates {
        if let Some(parent) = candidate.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::write(&candidate, DEFAULT_CONFIG_TEMPLATE).is_ok() {
            return Some(candidate);
        }
    }

    None
}

/// 取得預設建立 `config.toml` 的候選路徑清單。
fn default_config_creation_candidates(base_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if !base_dir.as_os_str().is_empty() {
        candidates.push(base_dir.join("config.toml"));
    }

    if let Some(app_data) = env::var_os("APPDATA") {
        candidates.push(PathBuf::from(app_data).join("panefm").join("config.toml"));
    }

    if let Some(xdg_home) = env::var_os("XDG_CONFIG_HOME") {
        candidates.push(PathBuf::from(xdg_home).join("panefm").join("config.toml"));
    }

    if let Some(home) = env::var_os("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join(".config")
                .join("panefm")
                .join("config.toml"),
        );
    }

    candidates
}

/// 建立 `plugins.toml` 的搜尋路徑清單。
///
/// 參數：
/// - `base_dir: &Path`，目前應用程式/可執行檔目錄。
/// - `config_source: Option<&Path>`，已找到的 `config.toml` 路徑，用來推導同層的 `plugins.toml`。
///
/// 回傳：`Vec<PathBuf>`，依照優先順序排列的候選 plugins 檔案路徑。
fn plugins_search_paths(base_dir: &Path, config_source: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(path) = env::var_os("PANE_FM_PLUGINS") {
        paths.push(PathBuf::from(path));
    }

    if let Some(path) = env::var_os("TFM_PLUGINS") {
        paths.push(PathBuf::from(path));
    }

    if let Some(config_path) = config_source {
        if let Some(parent) = config_path.parent() {
            paths.push(parent.join("plugins.toml"));
        }
    } else if !base_dir.as_os_str().is_empty() {
        paths.push(base_dir.join("plugins.toml"));
    }

    if let Some(xdg_home) = env::var_os("XDG_CONFIG_HOME") {
        let xdg_home = PathBuf::from(xdg_home);
        paths.push(app_config_file(&xdg_home, "panefm", "plugins.toml"));
        paths.push(app_config_file(
            &xdg_home,
            "terminal-file-manager",
            "plugins.toml",
        ));
    }

    if let Some(home) = env::var_os("HOME") {
        let config_home = PathBuf::from(home).join(".config");
        paths.push(app_config_file(&config_home, "panefm", "plugins.toml"));
        paths.push(app_config_file(
            &config_home,
            "terminal-file-manager",
            "plugins.toml",
        ));
    }

    if let Some(app_data) = env::var_os("APPDATA") {
        let app_data = PathBuf::from(app_data);
        paths.push(app_config_file(&app_data, "panefm", "plugins.toml"));
        paths.push(app_config_file(
            &app_data,
            "terminal-file-manager",
            "plugins.toml",
        ));
    }

    paths
}

/// 建立應用程式設定檔的完整候選路徑。
///
/// 參數：
/// - `config_root: &Path`，平台提供的設定根目錄，例如 XDG config home 或 `%APPDATA%`。
/// - `app_name: &str`，應用程式設定子目錄名稱。
/// - `file_name: &str`，要尋找的設定檔名稱。
///
/// 回傳：`PathBuf`，格式為 `<config_root>/<app_name>/<file_name>`。
pub fn app_config_file(config_root: &Path, app_name: &str, file_name: &str) -> PathBuf {
    config_root.join(app_name).join(file_name)
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
    if let Some(watcher) = file.watcher {
        apply_watcher_config(config, watcher)?;
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
    let mut ui = UiConfigFile {
        theme: file.theme,
        poll_rate_ms: file.poll_rate_ms,
        preview: file.preview,
        ..Default::default()
    };
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
            default_linemode: file.default_linemode,
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

    if let Some(name) = pane.default_linemode {
        config.pane.default_linemode = StartupLinemode::from_name(name.trim()).with_context(|| {
            format!(
                "unknown pane.default_linemode: {}. available: mtime, btime, size, permissions, none",
                name.trim()
            )
        })?;
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

/// 套用並驗證 `[watcher]` 外部變更監看設定。
///
/// 參數：
/// - `config: &mut AppConfig`，程式真正使用的完整設定。
/// - `watcher: WatcherConfigFile`，從 TOML 解析但尚未驗證的可選欄位。
///
/// 回傳：`Result<()>`；毫秒欄位為零時回傳設定錯誤，避免 watcher 忙迴圈耗盡 CPU。
fn apply_watcher_config(config: &mut AppConfig, watcher: WatcherConfigFile) -> Result<()> {
    if let Some(enabled) = watcher.enabled {
        config.watcher.enabled = enabled;
    }
    if let Some(milliseconds) = watcher.debounce_ms {
        if milliseconds == 0 {
            bail!("watcher.debounce_ms must be greater than 0");
        }
        config.watcher.debounce = Duration::from_millis(milliseconds);
    }
    if let Some(milliseconds) = watcher.fallback_poll_interval_ms {
        if milliseconds == 0 {
            bail!("watcher.fallback_poll_interval_ms must be greater than 0");
        }
        config.watcher.fallback_poll_interval = Duration::from_millis(milliseconds);
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

/// 驗證並套用 `[terminal]` 自訂啟動器。
///
/// 參數：`config: &mut AppConfig`，最終設定；`terminal: TerminalLauncherFile`，原始欄位。
/// 回傳：`Result<()>`；完全沒有命令時回傳設定錯誤，避免 `wt` 靜默失效。
fn apply_terminal_launcher_config(
    config: &mut AppConfig,
    terminal: TerminalLauncherFile,
) -> Result<()> {
    let normalize = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let launcher = TerminalLauncherConfig {
        command: normalize(terminal.command),
        mac_command: normalize(terminal.mac_command),
        windows_command: normalize(terminal.windows_command),
    };
    if launcher.command.is_none()
        && launcher.mac_command.is_none()
        && launcher.windows_command.is_none()
    {
        bail!("terminal must define at least one of command / mac_command / windows_command");
    }
    config.actions.terminal = Some(launcher);
    Ok(())
}

/// 套用並驗證 `terminals` 自訂終端外掛清單設定。
fn apply_terminal_plugins_config(
    config: &mut AppConfig,
    terminals: Vec<TerminalPluginFile>,
) -> Result<()> {
    let normalize = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    for item in terminals {
        let name = match item
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(name) => name.to_string(),
            None => bail!("terminal plugin must define a non-empty name"),
        };
        let match_env = item
            .match_env
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let match_process = item
            .match_process
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let command = normalize(item.command);
        let mac_command = normalize(item.mac_command);
        let windows_command = normalize(item.windows_command);

        if command.is_none() && mac_command.is_none() && windows_command.is_none() {
            bail!(
                "terminal plugin '{}' must define at least one of command / mac_command / windows_command",
                name
            );
        }

        config.actions.terminals.push(TerminalPluginConfig {
            name,
            match_env,
            match_process,
            command,
            mac_command,
            windows_command,
        });
    }
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
