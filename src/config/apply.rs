//! 設定檔讀取、反序列化與分區驗證套用邏輯。

use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result, bail};

use crate::theme::ThemePreset;

use super::file_types::*;
use super::paths::{config_search_paths, plugins_search_paths};
use super::plugins::{
    apply_actions_config, apply_terminal_launcher_config, apply_terminal_plugins_config,
};
use super::schema::*;

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

    if let Some(vcs) = ui.vcs
        && let Some(enabled) = vcs.enabled
    {
        config.ui.vcs.enabled = enabled;
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
