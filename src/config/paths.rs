//! 設定檔與 plugins.toml 跨平台搜尋路徑推導。

use std::{
    env,
    path::{Path, PathBuf},
};

/// 建立設定檔搜尋路徑清單。
///
/// 參數：
/// - `base_dir: &Path`，目前應用程式/可執行檔目錄。
///
/// 回傳：`Vec<PathBuf>`，依照優先順序排列的候選設定檔路徑。
pub(crate) fn config_search_paths(base_dir: &Path) -> Vec<PathBuf> {
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

/// 建立 `plugins.toml` 的搜尋路徑清單。
///
/// 參數：
/// - `base_dir: &Path`，目前應用程式/可執行檔目錄。
/// - `config_source: Option<&Path>`，已找到的 `config.toml` 路徑，用來推導同層的 `plugins.toml`。
///
/// 回傳：`Vec<PathBuf>`，依照優先順序排列的候選 plugins 檔案路徑。
pub(crate) fn plugins_search_paths(base_dir: &Path, config_source: Option<&Path>) -> Vec<PathBuf> {
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
