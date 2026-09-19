//! 預設設定檔模板與自動建立初始化。

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use super::paths::config_search_paths;

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

[ui.vcs]
# 是否在標題列顯示版本控制（Git 與 SVN）分支/版本號，以及在檔案清單顯示 M/A/D 等狀態標籤。
enabled = true

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
