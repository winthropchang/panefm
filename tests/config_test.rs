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
