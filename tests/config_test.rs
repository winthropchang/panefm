//! `config` 設定模組與 `default_linemode` 的整合與回歸測試。
//!
//! 依據 `DEVELOPMENT_GUIDELINES.md` 規範，測試程式碼獨立放置於 `tests/` 目錄，
//! 涵蓋 `StartupLinemode` 解析、預設值向後相容 fallback、首次啟動自動建立
//! 帶註解之 `config.toml` 範本，以及 pane 初始化狀態套用。

use std::fs;
use tempfile::tempdir;

use panefm::config::{StartupLinemode, ensure_default_config_file, load_config};

#[test]
/// 驗證 `StartupLinemode::from_name` 能精確解析各種名稱與常見別名，並拒絕無效名稱。
/// 保護目的：避免使用者在設定檔填寫常規別名（如 modified, btime, perms 等）時被誤判為錯誤。
fn startup_linemode_parsing_recognizes_all_variants_and_aliases() {
    // mtime / modified
    assert_eq!(
        StartupLinemode::from_name("mtime"),
        Some(StartupLinemode::Mtime)
    );
    assert_eq!(
        StartupLinemode::from_name("modified"),
        Some(StartupLinemode::Mtime)
    );
    assert_eq!(
        StartupLinemode::from_name(" MTIME "),
        Some(StartupLinemode::Mtime)
    );

    // btime / created / birth
    assert_eq!(
        StartupLinemode::from_name("btime"),
        Some(StartupLinemode::Btime)
    );
    assert_eq!(
        StartupLinemode::from_name("created"),
        Some(StartupLinemode::Btime)
    );
    assert_eq!(
        StartupLinemode::from_name("birth"),
        Some(StartupLinemode::Btime)
    );

    // size
    assert_eq!(
        StartupLinemode::from_name("size"),
        Some(StartupLinemode::Size)
    );

    // permissions / perms / perm
    assert_eq!(
        StartupLinemode::from_name("permissions"),
        Some(StartupLinemode::Permissions)
    );
    assert_eq!(
        StartupLinemode::from_name("perms"),
        Some(StartupLinemode::Permissions)
    );

    // none / off
    assert_eq!(
        StartupLinemode::from_name("none"),
        Some(StartupLinemode::None)
    );
    assert_eq!(
        StartupLinemode::from_name("off"),
        Some(StartupLinemode::None)
    );

    // 無效值
    assert_eq!(StartupLinemode::from_name("invalid_mode"), None);
}

#[test]
/// 驗證當設定檔存在但未指定 `default_linemode` 時，預設平滑 fallback 至 `StartupLinemode::Mtime`。
/// 保護目的：確保既有使用者升級版本後，舊設定檔完全相容且開箱即享修改時間呈現。
fn load_config_defaults_linemode_to_mtime_when_omitted() {
    let dir = tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[pane]
show_hidden = true
default_sort = "natural"
"#,
    )
    .expect("write config");

    let loaded = load_config(dir.path()).expect("load config");
    assert!(loaded.config.pane.show_hidden);
    assert_eq!(loaded.config.pane.default_linemode, StartupLinemode::Mtime);
}

#[test]
/// 驗證設定檔明確設定 `default_linemode` 時能正確解析生效。
/// 保護目的：確保使用者在 `[pane]` 區塊自訂的 linemode 能正確套用。
fn load_config_reads_custom_default_linemode() {
    let dir = tempdir().expect("tempdir");
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[pane]
default_sort = "natural"
default_linemode = "btime"
"#,
    )
    .expect("write config");

    let loaded = load_config(dir.path()).expect("load config");
    assert_eq!(loaded.config.pane.default_linemode, StartupLinemode::Btime);
}

#[test]
/// 驗證當環境中完全不存在 `config.toml` 時，`ensure_default_config_file` 會自動建立檔案且包含詳細註解。
/// 保護目的：讓使用者首次執行軟體時，能立即在目標目錄看見並自由編輯包含所有預設值的設定檔。
fn ensure_default_config_file_creates_full_template_when_missing() {
    let dir = tempdir().expect("tempdir");

    // 確認尚未建立
    let config_file = dir.path().join("config.toml");
    assert!(!config_file.exists());

    // 觸發自動建立
    let created_path = ensure_default_config_file(dir.path());
    assert_eq!(created_path, Some(config_file.clone()));
    assert!(config_file.exists());

    let content = fs::read_to_string(&config_file).expect("read created config");
    // 驗證包含完整區塊與繁體中文註解
    assert!(content.contains("[ui]"));
    assert!(content.contains("[pane]"));
    assert!(content.contains("default_linemode = \"mtime\""));
    assert!(content.contains("介面色彩主題"));
    assert!(content.contains("列表右側欄位預設顯示的資訊類型"));

    // 驗證自動建立的檔案可被正常解析載入
    let loaded = load_config(dir.path()).expect("load generated config");
    assert_eq!(loaded.config.pane.default_linemode, StartupLinemode::Mtime);
    assert_eq!(loaded.source, Some(config_file));
}

#[test]
/// 驗證當 `config.toml` 已經存在時，`ensure_default_config_file` 絕不覆寫或改動既有設定。
/// 保護目的：避免程式重新啟動時把使用者精心調整的設定檔沖刷覆蓋。
fn ensure_default_config_file_does_not_overwrite_existing() {
    let dir = tempdir().expect("tempdir");
    let config_file = dir.path().join("config.toml");
    let custom_content = r#"
[ui]
theme = "dracula"
"#;
    fs::write(&config_file, custom_content).expect("write existing config");

    let result = ensure_default_config_file(dir.path());
    assert_eq!(result, None);

    let current_content = fs::read_to_string(&config_file).expect("read config");
    assert_eq!(current_content, custom_content);
}

use panefm::config::{
    ActionLaunchMode, ActionTargetScope, AppConfig, StartupSort, app_config_file, persist_theme,
};
use panefm::theme::ThemePreset;
use std::path::Path;
use std::time::Duration;

#[test]
/// 驗證 PaneFM 在 macOS、Windows 與 XDG 平台都能用相同規則建立設定路徑。
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
fn app_config_file_uses_brand_directory_and_requested_file() {
    assert_eq!(
        app_config_file(Path::new("/settings"), "panefm", "config.toml"),
        Path::new("/settings").join("panefm").join("config.toml")
    );
    assert_eq!(
        app_config_file(
            Path::new("C:/Users/Otto/AppData/Roaming"),
            "panefm",
            "plugins.toml",
        ),
        Path::new("C:/Users/Otto/AppData/Roaming")
            .join("panefm")
            .join("plugins.toml")
    );
}

#[test]
/// 驗證保存主題時只更新 `[ui]` 的 `theme`，並保留其他設定與註解。
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
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
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
fn persist_theme_creates_missing_config() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");

    persist_theme(&path, ThemePreset::Kanagawa).expect("persist theme");

    assert_eq!(
        fs::read_to_string(path).expect("read config"),
        "[ui]\ntheme = \"kanagawa\"\n"
    );
}

#[test]
/// 驗證當找不到設定檔時，系統會回退到預設設定。
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
fn load_config_returns_defaults_when_missing() {
    let dir = tempdir().expect("tempdir");

    let loaded = load_config(dir.path()).expect("config");

    assert_eq!(loaded.config, AppConfig::default());
    assert!(loaded.source.is_none());
}

#[test]
/// 驗證新版分區式 `config.toml` 內容可以正確解析成設定。
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
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

[watcher]
enabled = false
debounce_ms = 240
fallback_poll_interval_ms = 3500

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
    assert!(!loaded.config.watcher.enabled);
    assert_eq!(loaded.config.watcher.debounce, Duration::from_millis(240));
    assert_eq!(
        loaded.config.watcher.fallback_poll_interval,
        Duration::from_millis(3_500)
    );
    assert_eq!(loaded.config.navigation.fast_move_step, 7);
    assert_eq!(loaded.config.navigation.panel_page_step, 14);
    assert!(!loaded.config.behavior.cancel_search_on_leave);
    assert!(loaded.config.actions.open_with.is_empty());
    assert_eq!(loaded.source, Some(dir.path().join("config.toml")));
}

#[test]
/// 驗證舊版平鋪設定格式仍可繼續使用。
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
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
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
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
/// 驗證 plugins.toml 的 terminal 啟動器可分別保存 macOS 與 Windows 公司入口。
///
/// 保護目的：TrustView 等保護環境不能被固定的系統終端命令繞過，且路徑模板必須保留。
fn load_config_reads_custom_terminal_launcher() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("plugins.toml"),
        r#"
[terminal]
mac_command = "open -a 'Protected Terminal' {path}"
windows_command = "ProtectedTerminal.exe {path}"
"#,
    )
    .expect("plugins file");

    let loaded = load_config(dir.path()).expect("config");
    let terminal = loaded.config.actions.terminal.expect("terminal launcher");
    assert_eq!(
        terminal.mac_command.as_deref(),
        Some("open -a 'Protected Terminal' {path}")
    );
    assert_eq!(
        terminal.windows_command.as_deref(),
        Some("ProtectedTerminal.exe {path}")
    );
}

#[test]
/// 驗證即使沒有 `config.toml`，只要有 `plugins.toml` 也能載入自訂動作。
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
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
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
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
/// 驗證 watcher 的 debounce 與 fallback 掃描間隔不可為零。
/// 保護目的：避免錯誤設定讓背景監看執行緒忙迴圈，造成 PaneFM 持續占用 CPU。
fn load_config_rejects_zero_watcher_intervals() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("config.toml"),
        r#"
[watcher]
debounce_ms = 0
"#,
    )
    .expect("config file");

    let error = load_config(dir.path()).expect_err("should reject zero watcher interval");
    assert!(
        error
            .to_string()
            .contains("watcher.debounce_ms must be greater than 0")
    );
}

#[test]
/// 驗證字體設定不再是程式設定的一部分，避免使用者誤以為 TUI 能改外部 Terminal 字體。
/// 保護目的：避免設定格式演進或預設值調整時，破壞既有 config.toml 的相容性與驗證規則。
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

#[test]
/// 驗證 `plugins.toml` 的 `[[terminals]]` 自訂適配器清單可以正確載入。
fn load_config_reads_terminal_plugins() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("plugins.toml"),
        r#"
[[terminals]]
name = "kitty"
match_env = ["KITTY_WINDOW_ID", "KITTY_PID"]
mac_command = "kitty @ launch --type=tab --cwd={path}"

[[terminals]]
name = "ghostty"
match_process = ["ghostty.exe", "ghostty"]
windows_command = "ghostty.exe --working-directory {path}"
"#,
    )
    .expect("write plugins");

    let loaded = load_config(dir.path()).expect("config");
    assert_eq!(loaded.config.actions.terminals.len(), 2);
    assert_eq!(loaded.config.actions.terminals[0].name, "kitty");
    assert_eq!(
        loaded.config.actions.terminals[0].match_env,
        vec!["KITTY_WINDOW_ID", "KITTY_PID"]
    );
    assert_eq!(
        loaded.config.actions.terminals[0].mac_command.as_deref(),
        Some("kitty @ launch --type=tab --cwd={path}")
    );
    assert_eq!(loaded.config.actions.terminals[1].name, "ghostty");
    assert_eq!(
        loaded.config.actions.terminals[1].match_process,
        vec!["ghostty.exe", "ghostty"]
    );
}
