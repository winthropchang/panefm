//! TOML 設定檔反序列化原始格式模型。

use serde::Deserialize;

/// 表示新版設定檔的原始格式。
///
/// 新版配置會盡量按功能分區，讓未來擴充時不容易失控。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AppConfigFile {
    pub(crate) ui: Option<UiConfigFile>,
    pub(crate) pane: Option<PaneConfigFile>,
    pub(crate) search: Option<SearchConfigFile>,
    pub(crate) watcher: Option<WatcherConfigFile>,
    pub(crate) navigation: Option<NavigationConfigFile>,
    pub(crate) behavior: Option<BehaviorConfigFile>,
}

/// 表示舊版平鋪設定檔格式。
///
/// 這個型別只為了相容舊檔案存在，未來說明文件將以新版分區格式為主。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct LegacyAppConfigFile {
    pub(crate) theme: Option<String>,
    pub(crate) poll_rate_ms: Option<u64>,
    pub(crate) show_hidden: Option<bool>,
    pub(crate) default_sort: Option<String>,
    pub(crate) default_sort_reverse: Option<bool>,
    pub(crate) default_linemode: Option<String>,
    pub(crate) preview: Option<PreviewConfigFile>,
    pub(crate) confirm_dialog: Option<DialogConfigFileRaw>,
    pub(crate) theme_picker: Option<DialogConfigFileRaw>,
}

/// 表示 `ui` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct UiConfigFile {
    pub(crate) theme: Option<String>,
    pub(crate) icons: Option<IconsConfigFile>,
    pub(crate) vcs: Option<VcsConfigFile>,
    pub(crate) poll_rate_ms: Option<u64>,
    pub(crate) preview: Option<PreviewConfigFile>,
    pub(crate) dialog: Option<DialogsConfigFile>,
}

/// 表示 `[ui.icons]` 在 TOML 中的可選設定欄位。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct IconsConfigFile {
    pub(crate) enabled: Option<bool>,
    pub(crate) style: Option<String>,
}

/// 表示 `[ui.vcs]` 在 TOML 中的可選設定欄位。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct VcsConfigFile {
    pub(crate) enabled: Option<bool>,
}

/// 表示 `pane` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct PaneConfigFile {
    pub(crate) show_hidden: Option<bool>,
    pub(crate) default_sort: Option<String>,
    pub(crate) default_sort_reverse: Option<bool>,
    pub(crate) default_linemode: Option<String>,
}

/// 表示 `search` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct SearchConfigFile {
    pub(crate) global_search_limit: Option<usize>,
    pub(crate) global_search_chunk_size: Option<usize>,
    pub(crate) show_loading: Option<bool>,
    pub(crate) fzf_follow_links: Option<bool>,
}

/// 表示 `[watcher]` 區塊中尚未驗證的可選欄位。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct WatcherConfigFile {
    pub(crate) enabled: Option<bool>,
    pub(crate) debounce_ms: Option<u64>,
    pub(crate) fallback_poll_interval_ms: Option<u64>,
}

/// 表示 `navigation` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct NavigationConfigFile {
    pub(crate) fast_move_step: Option<usize>,
    pub(crate) panel_page_step: Option<usize>,
}

/// 表示 `behavior` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct BehaviorConfigFile {
    pub(crate) cancel_search_on_leave: Option<bool>,
}

/// 表示 `plugins.toml` 的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct PluginsConfigFile {
    pub(crate) actions: Option<ActionsConfigFile>,
    pub(crate) terminal: Option<TerminalLauncherFile>,
    pub(crate) terminals: Option<Vec<TerminalPluginFile>>,
}

/// 表示 plugins.toml `[terminal]` 區塊尚未驗證的原始欄位。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct TerminalLauncherFile {
    pub(crate) command: Option<String>,
    pub(crate) mac_command: Option<String>,
    pub(crate) windows_command: Option<String>,
}

/// 表示 plugins.toml `[[terminals]]` 區塊尚未驗證的原始欄位。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct TerminalPluginFile {
    pub(crate) name: Option<String>,
    pub(crate) match_env: Option<Vec<String>>,
    pub(crate) match_process: Option<Vec<String>>,
    pub(crate) command: Option<String>,
    pub(crate) mac_command: Option<String>,
    pub(crate) windows_command: Option<String>,
}

/// 表示 `actions` 區塊的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ActionsConfigFile {
    pub(crate) open_with: Option<Vec<CustomOpenActionFile>>,
}

/// 表示單一自訂動作在設定檔中的原始欄位。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct CustomOpenActionFile {
    pub(crate) name: Option<String>,
    pub(crate) scope: Option<String>,
    pub(crate) mode: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) mac_command: Option<String>,
    pub(crate) windows_command: Option<String>,
}

/// 表示所有 dialog 群組的原始設定格式。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct DialogsConfigFile {
    pub(crate) confirm: Option<DialogConfigFileRaw>,
    pub(crate) theme_picker: Option<DialogConfigFileRaw>,
}

/// 表示設定檔中 popup 類視窗的尺寸設定區塊。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct DialogConfigFileRaw {
    pub(crate) width_percent: Option<u16>,
    pub(crate) height: Option<u16>,
}

/// 表示 preview 區塊的高度設定。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct PreviewConfigFile {
    pub(crate) height: Option<u16>,
    pub(crate) focus_list_height: Option<u16>,
}
