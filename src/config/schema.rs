//! 執行期設定結構模型與預設值定義。

use std::{path::PathBuf, time::Duration};

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
    pub vcs: VcsConfig,
    pub poll_rate: Duration,
    pub preview: PreviewConfig,
    pub dialogs: DialogsConfig,
}

/// 表示版本控制（Git 與 SVN）狀態整合設定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VcsConfig {
    /// 是否在標題列顯示分支/版本號，以及在檔案清單顯示 M/A/D 等狀態標籤。
    pub enabled: bool,
}

impl Default for VcsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
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
    pub scroll_acceleration: bool,
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
                vcs: VcsConfig { enabled: true },
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
                scroll_acceleration: true,
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
