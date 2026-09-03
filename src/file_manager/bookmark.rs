//! 本機與 SMB 書籤模型，以及 `bookmark.toml` 的同步持久化。
//!
//! 書籤是使用者資料而不是靜態設定，因此每次新增、刪除或清空都必須立即寫檔。
//! UI 只保存目前選取位置；實際 key 分配、路徑正規化與序列化由 `BookmarkStore` 處理。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use super::smb::{parse_smb_location, percent_decode};

/// 描述書籤實際指向的目標類型，可能是本機路徑，也可能是遠端 SMB 位址。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum BookmarkTarget {
    LocalPath(PathBuf),
    SmbLocation(String),
}

impl BookmarkTarget {
    /// 回傳適合寫進 `bookmark.toml`，或交給實際跳轉流程使用的原始字串。
    ///
    /// 參數：無。
    ///
    /// 回傳：`String`。
    /// - 本機書籤會回傳平台原生路徑的顯示文字。
    /// - SMB 書籤會完整保留原始 percent-encoded URI，避免中文路徑解碼後被誤當成
    ///   尚未編碼的連線位址，或在重新儲存時改變原有資料。
    pub(crate) fn as_storage_value(&self) -> String {
        match self {
            Self::LocalPath(path) => path.display().to_string(),
            Self::SmbLocation(location) => location.clone(),
        }
    }

    /// 回傳目前書籤要顯示在列表、filter 與狀態訊息中的可讀文字。
    ///
    /// 參數：無。
    ///
    /// 回傳：`String`。
    /// - 本機路徑維持原本顯示方式。
    /// - SMB URI 僅在這個顯示邊界進行 percent decode，讓 `%E7%B6%B2...` 顯示成中文；
    ///   `BookmarkTarget` 內保存的原始 URI 不會被修改，實際連線仍使用 encoded URI。
    pub(crate) fn display_text(&self) -> String {
        match self {
            Self::LocalPath(path) => path.display().to_string(),
            Self::SmbLocation(location) => decode_smb_uri_for_display(location),
        }
    }
}

/// 將 SMB URI 的百分比編碼轉成僅供 UI 顯示的 UTF-8 文字。
///
/// 參數：
/// - `location: &str`，書籤中實際保存、可直接交給 SMB 連線流程的 encoded URI。
///
/// 回傳：`String`，可讀性較高的顯示文字。若解碼結果不是合法 UTF-8，會退回原始 URI，
/// 避免 UI 用替代字元隱藏實際位址內容。
fn decode_smb_uri_for_display(location: &str) -> String {
    percent_decode(location).unwrap_or_else(|_| location.to_string())
}

/// 表示單一書籤在列表中要顯示的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BookmarkEntry {
    pub(crate) key: char,
    pub(crate) target: BookmarkTarget,
}

/// 負責管理 `bookmark.toml` 的讀取、寫入與記憶體同步。
#[derive(Clone, Debug)]
pub(crate) struct BookmarkStore {
    path: PathBuf,
    entries: BTreeMap<char, BookmarkTarget>,
}

impl BookmarkStore {
    /// 建立書籤儲存物件，並嘗試從既有的 `bookmark.toml` 載入內容。
    ///
    /// 參數：
    /// - `path: PathBuf`，書籤檔案的完整路徑。
    ///
    /// 回傳：`Result<BookmarkStore>`。
    /// - 成功時回傳已載入完成的書籤儲存物件。
    /// - 失敗時代表讀檔或解析 TOML 時發生錯誤。
    pub(crate) fn load(path: PathBuf) -> Result<Self> {
        let entries = load_entries(&path)?;
        Ok(Self { path, entries })
    }

    /// 以排序後的方式回傳目前全部書籤項目，供 UI 列表使用。
    pub(crate) fn list(&self) -> Vec<BookmarkEntry> {
        self.entries
            .iter()
            .map(|(key, target)| BookmarkEntry {
                key: *key,
                target: target.clone(),
            })
            .collect()
    }

    /// 取得指定書籤按鍵對應的路徑。
    pub(crate) fn get(&self, key: char) -> Option<&BookmarkTarget> {
        self.entries.get(&key)
    }

    /// 回傳目前可用的下一個自動書籤按鍵。
    ///
    /// 目前順序會優先使用：
    /// - `a..z`
    /// - `A..Z`
    /// - `0..9`
    ///
    /// 回傳：`Option<char>`。
    /// - `Some(char)` 代表找到尚未使用的書籤代號。
    /// - `None` 代表這批預設代號都已經被用完。
    pub(crate) fn next_available_key(&self) -> Option<char> {
        preferred_bookmark_keys()
            .into_iter()
            .find(|key| !self.entries.contains_key(key))
    }

    /// 設定或覆蓋單一書籤，並立即同步寫回 `bookmark.toml`。
    ///
    /// 參數：
    /// - `key: char`，書籤按鍵。
    /// - `path: PathBuf`，要記錄的目標路徑。
    ///
    /// 回傳：`Result<()>`。
    pub(crate) fn set(&mut self, key: char, path: PathBuf) -> Result<()> {
        self.entries.insert(key, BookmarkTarget::LocalPath(path));
        self.save()
    }

    /// 設定或覆蓋單一 SMB 書籤，並立即同步寫回 `bookmark.toml`。
    ///
    /// 參數：
    /// - `key: char`，書籤按鍵。
    /// - `location: String`，完整的 `smb://host/share[/path]` 目標。
    ///
    /// 回傳：`Result<()>`。
    pub(crate) fn set_smb(&mut self, key: char, location: String) -> Result<()> {
        self.entries
            .insert(key, BookmarkTarget::SmbLocation(location));
        self.save()
    }

    /// 刪除單一書籤，並立即同步寫回 `bookmark.toml`。
    ///
    /// 參數：
    /// - `key: char`，要刪除的書籤代號。
    ///
    /// 回傳：`Result<bool>`。
    /// - `Ok(true)` 代表確實刪除了既有書籤。
    /// - `Ok(false)` 代表該代號原本不存在。
    pub(crate) fn remove(&mut self, key: char) -> Result<bool> {
        let removed = self.entries.remove(&key).is_some();
        self.save()?;
        Ok(removed)
    }

    /// 清空全部書籤，並立即同步寫回 `bookmark.toml`。
    ///
    /// 回傳：`Result<()>`。
    pub(crate) fn clear(&mut self) -> Result<()> {
        self.entries.clear();
        self.save()
    }

    /// 將目前記憶體中的書籤完整寫回到 `bookmark.toml`。
    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create bookmark directory {}", parent.display())
            })?;
        }

        let raw = self
            .entries
            .iter()
            .map(|(key, target)| (key.to_string(), target.as_storage_value()))
            .collect::<BTreeMap<_, _>>();
        let content = toml::to_string_pretty(&BookmarkFile(raw))
            .context("failed to serialize bookmark.toml")?;
        fs::write(&self.path, content)
            .with_context(|| format!("failed to write bookmark file {}", self.path.display()))
    }
}

/// 決定目前這次啟動應該使用哪一個 `bookmark.toml` 路徑。
///
/// 參數：
/// - `base_dir: &Path`，目前專案工作目錄。
/// - `config_source: Option<&Path>`，實際載入的 `config.toml` 路徑。
///
/// 回傳：`PathBuf`，對應的 `bookmark.toml` 路徑。
pub(crate) fn bookmark_file_path(base_dir: &Path, config_source: Option<&Path>) -> PathBuf {
    config_source
        .and_then(Path::parent)
        .unwrap_or(base_dir)
        .join("bookmark.toml")
}

/// 將原始 TOML key/value 內容包成可序列化物件，讓輸出的格式維持單層表。
#[derive(Serialize)]
#[serde(transparent)]
struct BookmarkFile(BTreeMap<String, String>);

/// 回傳自動分配書籤時使用的預設代號順序。
fn preferred_bookmark_keys() -> Vec<char> {
    let mut keys = ('a'..='z').collect::<Vec<_>>();
    keys.extend('A'..='Z');
    keys.extend('0'..='9');
    keys
}

/// 從指定檔案讀取全部書籤；若檔案不存在則回傳空集合。
fn load_entries(path: &Path) -> Result<BTreeMap<char, BookmarkTarget>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read bookmark file {}", path.display()))?;
    let raw = toml::from_str::<BTreeMap<String, String>>(&content)
        .with_context(|| format!("failed to parse bookmark file {}", path.display()))?;

    let mut entries = BTreeMap::new();
    for (raw_key, raw_path) in raw {
        let key = parse_bookmark_key(&raw_key)?;
        let target = parse_bookmark_target(&raw_path)?;
        entries.insert(key, target);
    }

    Ok(entries)
}

/// 將 `bookmark.toml` 中的字串解析成書籤目標。
fn parse_bookmark_target(raw: &str) -> Result<BookmarkTarget> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("bookmark path cannot be empty");
    }

    if trimmed.starts_with("smb://") {
        Ok(BookmarkTarget::SmbLocation(trimmed.to_string()))
    } else if trimmed.starts_with("//") || trimmed.starts_with(r"\\") {
        if let Ok(location) = parse_smb_location(trimmed) {
            Ok(BookmarkTarget::SmbLocation(location.url))
        } else {
            Ok(BookmarkTarget::LocalPath(PathBuf::from(trimmed)))
        }
    } else {
        Ok(BookmarkTarget::LocalPath(PathBuf::from(trimmed)))
    }
}

/// 驗證單一書籤名稱，確保它符合單鍵操作的設計。
fn parse_bookmark_key(raw_key: &str) -> Result<char> {
    let trimmed = raw_key.trim();
    let mut chars = trimmed.chars();
    let Some(key) = chars.next() else {
        bail!("bookmark key cannot be empty");
    };
    if chars.next().is_some() {
        bail!("bookmark key must be exactly one character: {trimmed}");
    }
    if key.is_whitespace() {
        bail!("bookmark key cannot be whitespace");
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    /// 驗證不存在的 `bookmark.toml` 會被視為空集合，而不是報錯。
    /// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
    fn load_missing_bookmark_file_returns_empty_store() {
        let dir = tempdir().expect("tempdir");
        let store = BookmarkStore::load(dir.path().join("bookmark.toml")).expect("load store");

        assert!(store.list().is_empty());
    }

    #[test]
    /// 驗證設定書籤後會立刻寫回 `bookmark.toml`，之後重新載入仍能讀回相同內容。
    /// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
    fn set_bookmark_persists_to_file() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("bookmark.toml");

        let mut store = BookmarkStore::load(file.clone()).expect("load");
        store
            .set('a', PathBuf::from("/tmp/demo"))
            .expect("save bookmark");

        let reloaded = BookmarkStore::load(file).expect("reload");
        assert_eq!(
            reloaded.get('a'),
            Some(&BookmarkTarget::LocalPath(PathBuf::from("/tmp/demo")))
        );
    }

    #[test]
    /// 驗證 SMB 書籤會以原始 `smb://...` 字串寫回檔案，重新載入後仍能辨識成 SMB 目標。
    /// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
    fn set_smb_bookmark_persists_to_file() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("bookmark.toml");

        let mut store = BookmarkStore::load(file.clone()).expect("load");
        store
            .set_smb('s', String::from("smb://192.0.2.10/shared/docs"))
            .expect("save smb bookmark");

        let reloaded = BookmarkStore::load(file.clone()).expect("reload");
        assert_eq!(
            reloaded.get('s'),
            Some(&BookmarkTarget::SmbLocation(String::from(
                "smb://192.0.2.10/shared/docs"
            )))
        );
        assert!(
            fs::read_to_string(file)
                .expect("bookmark file")
                .contains("smb://192.0.2.10/shared/docs")
        );
    }

    #[test]
    /// 驗證 SMB 中文路徑在 UI 會顯示解碼後文字，但持久化值仍保持原始 percent-encoded URI。
    /// 保護目的：避免為了改善書籤可讀性，意外破壞 `bookmark.toml` 或實際 SMB 跳轉所需的 URI。
    fn smb_bookmark_decodes_only_its_display_text() {
        let encoded = "smb://192.0.2.10/shared/%E7%B6%B2%E8%B7%AF%E4%BA%8B%E6%A5%AD%E9%83%A8/otto";
        let target = BookmarkTarget::SmbLocation(encoded.to_string());

        assert_eq!(
            target.display_text(),
            "smb://192.0.2.10/shared/網路事業部/otto"
        );
        assert_eq!(target.as_storage_value(), encoded);
    }

    #[test]
    /// 驗證不合法的 UTF-8 percent encoding 不會在 UI 中被替代字元悄悄改寫。
    /// 保護目的：遇到非 UTF-8 SMB 名稱時仍顯示可供除錯的原始 URI，並確保連線資料不受影響。
    fn smb_bookmark_display_falls_back_for_invalid_utf8() {
        let encoded = "smb://192.0.2.10/shared/%FF";
        let target = BookmarkTarget::SmbLocation(encoded.to_string());

        assert_eq!(target.display_text(), encoded);
        assert_eq!(target.as_storage_value(), encoded);
    }

    #[test]
    /// 驗證書籤檔會放在 `config.toml` 同一個目錄，若沒有設定檔則退回工作目錄旁邊。
    /// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
    fn bookmark_file_path_prefers_config_directory() {
        let base = Path::new("/workspace/project");
        let config = Path::new("/workspace/settings/config.toml");

        assert_eq!(
            bookmark_file_path(base, Some(config)),
            PathBuf::from("/workspace/settings/bookmark.toml")
        );
        assert_eq!(
            bookmark_file_path(base, None),
            PathBuf::from("/workspace/project/bookmark.toml")
        );
    }

    #[test]
    /// 驗證自動分配書籤代號時，會挑出目前尚未使用的第一個預設按鍵。
    /// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
    fn next_available_key_uses_first_free_preferred_key() {
        let dir = tempdir().expect("tempdir");
        let mut store = BookmarkStore::load(dir.path().join("bookmark.toml")).expect("load");
        store
            .set('a', PathBuf::from("/tmp/a"))
            .expect("save a bookmark");
        store
            .set('b', PathBuf::from("/tmp/b"))
            .expect("save b bookmark");

        assert_eq!(store.next_available_key(), Some('c'));
    }

    #[test]
    /// 驗證刪除單一書籤後，記憶體與檔案都會同步更新。
    /// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
    fn remove_bookmark_persists_to_file() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("bookmark.toml");

        let mut store = BookmarkStore::load(file.clone()).expect("load");
        store
            .set('a', PathBuf::from("/tmp/demo"))
            .expect("save bookmark");

        assert!(store.remove('a').expect("remove bookmark"));
        assert_eq!(store.get('a'), None);
        assert!(
            !fs::read_to_string(file)
                .expect("bookmark file")
                .contains("/tmp/demo")
        );
    }

    #[test]
    /// 驗證清空全部書籤後，列表會變成空集合。
    /// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
    fn clear_bookmarks_removes_all_entries() {
        let dir = tempdir().expect("tempdir");
        let mut store = BookmarkStore::load(dir.path().join("bookmark.toml")).expect("load");
        store
            .set('a', PathBuf::from("/tmp/demo"))
            .expect("save bookmark");
        store
            .set('b', PathBuf::from("/tmp/demo2"))
            .expect("save bookmark");

        store.clear().expect("clear bookmarks");

        assert!(store.list().is_empty());
        assert_eq!(store.next_available_key(), Some('a'));
    }

    #[test]
    /// 驗證在 bookmark.toml 設定 UNC 格式時，能自動轉化並識別為 SMB 目標。
    fn unc_bookmark_parses_to_smb_location() {
        let target = parse_bookmark_target("//192.0.2.10/shared/docs").expect("parse unc");
        assert_eq!(
            target,
            BookmarkTarget::SmbLocation(String::from("smb://192.0.2.10/shared/docs"))
        );

        let backslash_target =
            parse_bookmark_target(r"\\192.0.2.10\shared\docs").expect("parse backslash");
        assert_eq!(
            backslash_target,
            BookmarkTarget::SmbLocation(String::from("smb://192.0.2.10/shared/docs"))
        );
    }
}
