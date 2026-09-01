//! PaneFM 私有 trash 儲存、metadata 與還原/永久刪除操作。
//!
//! 每筆項目由內容檔與 metadata 配對，還原時依 metadata 回到原位置。`App` 負責
//! 多選與確認視窗，本模組只執行原子化程度可控的單筆/批次儲存操作。

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(not(test))]
use std::env;

use serde::{Deserialize, Serialize};

/// 管理 terminal file manager 內部 trash 儲存區的主要物件。
///
/// 這一版採用「集中式 internal trash」設計：
/// - 被刪除的項目會先移到專用資料夾，而不是直接永久刪除。
/// - 每個項目都會保存原始路徑與刪除時間，方便之後 restore。
#[derive(Debug, Clone)]
pub(crate) struct TrashStore {
    root_dir: PathBuf,
    items_dir: PathBuf,
    metadata_dir: PathBuf,
}

/// 表示單一 trash 項目的紀錄內容。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrashRecord {
    id: String,
    original_path: PathBuf,
    trashed_path: PathBuf,
    display_name: String,
    deleted_at_unix_ms: u64,
}

/// 表示 restore 完成後回傳給呼叫端的資訊。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RestoreResult {
    pub(crate) restored_path: PathBuf,
    pub(crate) display_name: String,
}

/// 表示 trash 面板上可列出的單一項目摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrashListEntry {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) original_path: PathBuf,
    pub(crate) deleted_at_unix_ms: u64,
}

impl TrashStore {
    /// 建立 trash store，並預先計算資料夾位置。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<TrashStore>`。
    /// - 成功時回傳可直接使用的 trash store。
    /// - 失敗時回傳建立資料夾時的 I/O 錯誤。
    pub(crate) fn new(base_dir: &Path) -> io::Result<Self> {
        let root_dir = resolve_trash_root(base_dir);
        let items_dir = root_dir.join("items");
        let metadata_dir = root_dir.join("metadata");

        Ok(Self {
            root_dir,
            items_dir,
            metadata_dir,
        })
    }

    /// 檢查來源路徑是否與 trash store 位於不同磁碟機或掛載點。
    pub(crate) fn is_cross_device(&self, path: &Path) -> bool {
        #[cfg(windows)]
        {
            use std::path::Component;
            let s_prefix = path.components().next();
            let t_prefix = self.items_dir.components().next();
            match (s_prefix, t_prefix) {
                (Some(Component::Prefix(p1)), Some(Component::Prefix(p2))) => {
                    p1.as_os_str().to_ascii_lowercase() != p2.as_os_str().to_ascii_lowercase()
                }
                _ => false,
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let (Ok(m1), Ok(m2)) = (std::fs::metadata(path), std::fs::metadata(&self.items_dir)) {
                m1.dev() != m2.dev()
            } else {
                false
            }
        }
    }

    /// 將指定項目移到 internal trash。
    ///
    /// 參數：
    /// - `source_path: &Path`，要丟進 trash 的來源路徑。
    /// - `display_name: &str`，畫面上顯示用的名稱。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn trash_path(&self, source_path: &Path, display_name: &str) -> io::Result<()> {
        self.ensure_dirs()?;
        let id = new_record_id();
        let file_name = source_path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
        })?;
        let trashed_path = self
            .items_dir
            .join(format!("{}__{}", id, file_name.to_string_lossy()));

        move_path(source_path, &trashed_path)?;

        let record = TrashRecord {
            id: id.clone(),
            original_path: source_path.to_path_buf(),
            trashed_path,
            display_name: display_name.to_string(),
            deleted_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.write_record(&record)?;
        Ok(())
    }

    /// 還原最近一次被放進 trash 的項目。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<Option<RestoreResult>>`。
    /// - `Ok(Some(...))` 代表成功還原一個項目。
    /// - `Ok(None)` 代表目前 trash 是空的。
    pub(crate) fn restore_latest(&self) -> io::Result<Option<RestoreResult>> {
        let Some(record) = self.latest_record()? else {
            return Ok(None);
        };
        self.restore_record(record).map(Some)
    }

    /// 回傳目前 trash 中所有可顯示項目，已依刪除時間由新到舊排序。
    ///
    /// 參數：無。
    /// 回傳：`io::Result<Vec<TrashListEntry>>`。
    pub(crate) fn list_entries(&self) -> io::Result<Vec<TrashListEntry>> {
        let mut records = self.load_records()?;
        records.sort_by(|left, right| {
            right
                .deleted_at_unix_ms
                .cmp(&left.deleted_at_unix_ms)
                .then_with(|| right.id.cmp(&left.id))
        });

        Ok(records
            .into_iter()
            .map(|record| TrashListEntry {
                id: record.id,
                display_name: record.display_name,
                original_path: record.original_path,
                deleted_at_unix_ms: record.deleted_at_unix_ms,
            })
            .collect())
    }

    /// 依照指定 id 清單批次還原 trash 項目。
    ///
    /// 參數：
    /// - `ids: &[String]`，要還原的 trash 紀錄 id 清單。
    ///
    /// 回傳：`io::Result<Vec<RestoreResult>>`。
    /// - 成功時回傳實際成功還原的項目結果。
    /// - 若某些 id 已不存在，會自動略過，不視為整體失敗。
    pub(crate) fn restore_many_by_ids(&self, ids: &[String]) -> io::Result<Vec<RestoreResult>> {
        let records = self.load_records()?;
        let mut results = Vec::new();
        for id in ids {
            if let Some(record) = records.iter().find(|record| &record.id == id).cloned() {
                results.push(self.restore_record(record)?);
            }
        }
        Ok(results)
    }

    /// 依照指定 id 清單批次永久刪除 trash 項目。
    ///
    /// 參數：
    /// - `ids: &[String]`，要永久刪除的 trash 紀錄 id 清單。
    ///
    /// 回傳：`io::Result<Vec<String>>`，內容為實際被刪除的顯示名稱。
    pub(crate) fn delete_many_by_ids(&self, ids: &[String]) -> io::Result<Vec<String>> {
        let records = self.load_records()?;
        let mut deleted_names = Vec::new();
        for id in ids {
            if let Some(record) = records.iter().find(|record| &record.id == id).cloned() {
                deleted_names.push(self.delete_record(record)?);
            }
        }
        Ok(deleted_names)
    }

    /// 將單一 trash 紀錄還原回檔案系統。
    fn restore_record(&self, record: TrashRecord) -> io::Result<RestoreResult> {
        let target_parent = record
            .original_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&target_parent)?;

        let original_name = record.original_path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "original path has no file name",
            )
        })?;
        let restore_path = unique_target_path(&target_parent, original_name);

        move_path(&record.trashed_path, &restore_path)?;
        let metadata_path = self.metadata_path(&record.id);
        if metadata_path.exists() {
            fs::remove_file(metadata_path)?;
        }

        Ok(RestoreResult {
            restored_path: restore_path.clone(),
            display_name: display_name_for_path(&restore_path),
        })
    }

    /// 將單一 trash 紀錄永久移除，不再保留還原可能。
    fn delete_record(&self, record: TrashRecord) -> io::Result<String> {
        if record.trashed_path.exists() {
            remove_path(&record.trashed_path)?;
        }

        let metadata_path = self.metadata_path(&record.id);
        if metadata_path.exists() {
            fs::remove_file(metadata_path)?;
        }

        Ok(record.display_name)
    }

    /// 回傳目前 trash 根目錄，主要給測試或除錯使用。
    #[allow(dead_code)]
    pub(crate) fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// 載入目前所有 trash 紀錄，並回傳最新的一筆。
    fn latest_record(&self) -> io::Result<Option<TrashRecord>> {
        let mut records = self.load_records()?;
        records.sort_by(|left, right| {
            left.deleted_at_unix_ms
                .cmp(&right.deleted_at_unix_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(records.pop())
    }

    /// 載入目前 metadata 目錄中的所有 trash 紀錄。
    fn load_records(&self) -> io::Result<Vec<TrashRecord>> {
        if !self.metadata_dir.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.metadata_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }

            let contents = fs::read_to_string(&path)?;
            let record: TrashRecord = toml::from_str(&contents).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("failed to parse trash record {}: {error}", path.display()),
                )
            })?;
            records.push(record);
        }
        Ok(records)
    }

    /// 將單一 trash 紀錄寫入 metadata 目錄。
    fn write_record(&self, record: &TrashRecord) -> io::Result<()> {
        let contents = toml::to_string(record).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize trash record: {error}"),
            )
        })?;
        fs::write(self.metadata_path(&record.id), contents)?;
        Ok(())
    }

    /// 確保 trash 的 items 與 metadata 目錄都已存在。
    fn ensure_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(&self.items_dir)?;
        fs::create_dir_all(&self.metadata_dir)?;
        Ok(())
    }

    /// 組出某一筆 trash 紀錄對應的 metadata 路徑。
    fn metadata_path(&self, id: &str) -> PathBuf {
        self.metadata_dir.join(format!("{id}.toml"))
    }
}

/// 解析 internal trash 的根目錄。
fn resolve_trash_root(base_dir: &Path) -> PathBuf {
    // 測試必須把所有資料限制在 tempdir，否則平行測試會讀寫開發者真正的 Trash。
    #[cfg(test)]
    return base_dir.join(".tfm").join("trash");

    #[cfg(not(test))]
    {
        let custom = env::var_os("TFM_TRASH_DIR");
        let platform_root = resolve_trash_root_from_environment(
            base_dir,
            custom.clone(),
            env::var_os("PANEFM_STATE_DIR"),
            env::var_os("LOCALAPPDATA"),
            env::var_os("APPDATA"),
            env::var_os("HOME"),
            cfg!(target_os = "macos"),
        );
        prefer_existing_legacy_trash(base_dir, platform_root, custom.is_some())
    }
}

/// 在既有安裝仍有舊 Trash 時保留其可見性，不搬移也不刪除使用者資料。
///
/// 參數：`base_dir: &Path` 是啟動目錄；`platform_root: PathBuf` 是新式全域位置；
/// `has_custom_root: bool` 表示使用者是否明確指定 `TFM_TRASH_DIR`。回傳：實際使用路徑。
/// 明確設定永遠優先；否則只有舊 Trash 確實存在時才沿用它。這個相容路徑只負責避免
/// 舊資料突然消失；檔案傳輸仍會忠實複製 `.tfm`，使用者可清空舊 Trash 後改用新位置。
fn prefer_existing_legacy_trash(
    base_dir: &Path,
    platform_root: PathBuf,
    has_custom_root: bool,
) -> PathBuf {
    let legacy_root = base_dir.join(".tfm").join("trash");
    if !has_custom_root && legacy_root.exists() {
        legacy_root
    } else {
        platform_root
    }
}

/// 依跨平台環境變數決定 PaneFM Trash 的使用者資料位置。
///
/// Trash 不可預設放在目前工作目錄，否則使用者複製整個專案時會把 PaneFM 自己的
/// 刪除歷史一起複製；Trash 越大，後續每一次專案 copy 又會呈倍數放大。優先順序為：
/// 明確覆寫、PaneFM state、Windows local/app data、macOS Application Support，最後才是
/// 可攜式 fallback。
///
/// 參數：
/// - `base_dir: &Path`，所有平台資訊都缺失時的最後 fallback。
/// - 其餘 `Option<OsString>`，分別是已讀取的環境變數，方便單元測試不修改全域環境。
/// - `is_macos: bool`，指定 HOME 應採 macOS 或一般可攜式目錄規則。
///
/// 回傳：`PathBuf`，PaneFM 專用 Trash 根目錄；函數只計算路徑，不建立或搬移檔案。
#[allow(clippy::too_many_arguments)]
fn resolve_trash_root_from_environment(
    base_dir: &Path,
    custom: Option<std::ffi::OsString>,
    state_dir: Option<std::ffi::OsString>,
    local_app_data: Option<std::ffi::OsString>,
    app_data: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    is_macos: bool,
) -> PathBuf {
    if let Some(path) = custom {
        return PathBuf::from(path);
    }
    if let Some(path) = state_dir {
        return PathBuf::from(path).join("trash");
    }
    if let Some(path) = local_app_data.or(app_data) {
        return PathBuf::from(path).join("panefm").join("trash");
    }
    if let Some(home) = home {
        let home = PathBuf::from(home);
        return if is_macos {
            home.join("Library")
                .join("Application Support")
                .join("panefm")
                .join("trash")
        } else {
            home.join(".local")
                .join("share")
                .join("panefm")
                .join("trash")
        };
    }

    base_dir.join(".tfm").join("trash")
}

/// 產生新的 trash 紀錄識別值。
fn new_record_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

/// 將來源路徑移動到目標路徑，若跨裝置導致 rename 失敗，則退回 copy+delete。
fn move_path(source_path: &Path, target_path: &Path) -> io::Result<()> {
    match fs::rename(source_path, target_path) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_path(source_path, target_path)?;
            remove_path(source_path)
        }
    }
}

/// 將檔案或資料夾複製到指定目標路徑。
fn copy_path(source_path: &Path, target_path: &Path) -> io::Result<()> {
    if source_path.is_dir() {
        copy_dir_recursive(source_path, target_path)
    } else {
        fs::copy(source_path, target_path)?;
        Ok(())
    }
}

/// 遞迴複製整個資料夾。
fn copy_dir_recursive(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    fs::create_dir(target_dir)?;

    for item in fs::read_dir(source_dir)? {
        let item = item?;
        let item_path = item.path();
        let next_target = target_dir.join(item.file_name());

        if item.file_type()?.is_dir() {
            copy_dir_recursive(&item_path, &next_target)?;
        } else {
            fs::copy(&item_path, &next_target)?;
        }
    }

    Ok(())
}

fn ensure_path_writable(path: &Path) {
    if let Ok(mut perms) = fs::symlink_metadata(path).map(|m| m.permissions()) {
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

/// 依路徑型別安全刪除檔案、符號連結或整個資料夾樹，避免 Windows 重新分析點 (reparse point / symlink) 的 4395 錯誤。
fn remove_path(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        #[cfg(windows)]
        {
            if file_type.is_dir() {
                let _ = fs::remove_dir(path);
                return Ok(());
            }
        }
        let _ = fs::remove_file(path);
        return Ok(());
    }

    if file_type.is_dir() {
        ensure_path_writable(path);
        if let Ok(read_dir) = fs::read_dir(path) {
            for entry in read_dir.flatten() {
                let entry_path = entry.path();
                let _ = remove_path(&entry_path);
            }
        }
        ensure_path_writable(path);
        let _ = fs::remove_dir(path);
    } else {
        ensure_path_writable(path);
        if let Err(err) = fs::remove_file(path) {
            if err.kind() != io::ErrorKind::NotFound {
                ensure_path_writable(path);
                let _ = fs::remove_file(path);
            }
        }
    }

    if path.exists() {
        ensure_path_writable(path);
        let _ = fs::remove_dir(path);
        let _ = fs::remove_file(path);
    }
    Ok(())
}

/// 根據目標資料夾現況，產生一個不與既有項目衝突的新路徑。
fn unique_target_path(target_dir: &Path, original_name: &std::ffi::OsStr) -> PathBuf {
    let original_name = original_name.to_string_lossy();
    let initial_candidate = target_dir.join(original_name.as_ref());
    if !initial_candidate.exists() {
        return initial_candidate;
    }

    let (base_name, extension) = split_name_for_duplicate(&original_name);
    let mut duplicate_index = 1usize;

    loop {
        let candidate_name = if duplicate_index == 1 {
            duplicate_name(&base_name, extension.as_deref(), None)
        } else {
            duplicate_name(&base_name, extension.as_deref(), Some(duplicate_index))
        };
        let candidate_path = target_dir.join(candidate_name);
        if !candidate_path.exists() {
            return candidate_path;
        }
        duplicate_index += 1;
    }
}

/// 將檔名拆成主名稱與副檔名。
fn split_name_for_duplicate(name: &str) -> (String, Option<String>) {
    match name.rsplit_once('.') {
        Some((base, ext)) if !base.is_empty() => (base.to_string(), Some(ext.to_string())),
        _ => (name.to_string(), None),
    }
}

/// 產生 restore 時使用的重名檔案名稱。
fn duplicate_name(
    base_name: &str,
    extension: Option<&str>,
    duplicate_index: Option<usize>,
) -> String {
    let suffix = match duplicate_index {
        None => " restored".to_string(),
        Some(index) => format!(" restored {index}"),
    };

    match extension {
        Some(ext) => format!("{base_name}{suffix}.{ext}"),
        None => format!("{base_name}{suffix}"),
    }
}

/// 依照路徑組出畫面顯示用名稱。
fn display_name_for_path(path: &Path) -> String {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    if path.is_dir() {
        format!("{file_name}/")
    } else {
        file_name
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, fs, path::Path};

    use tempfile::tempdir;

    use super::{TrashStore, prefer_existing_legacy_trash, resolve_trash_root_from_environment};

    #[test]
    /// 驗證正式環境在 macOS 與 Windows 都會把 Trash 放在使用者資料區，而不是專案內。
    /// 保護目的：專案內 `.tfm/trash` 曾累積到數十 GB，導致複製專案時把 Trash 再複製
    /// 一次並耗時數分鐘；此測試避免路徑重構後再次引入相同的遞迴放大問題。
    fn production_trash_paths_are_outside_the_working_directory() {
        let cwd = Path::new("/workspace/project");
        let mac = resolve_trash_root_from_environment(
            cwd,
            None,
            None,
            None,
            None,
            Some(OsString::from("/Users/example")),
            true,
        );
        let windows = resolve_trash_root_from_environment(
            cwd,
            None,
            None,
            Some(OsString::from(r"C:\Users\example\AppData\Local")),
            None,
            None,
            false,
        );

        assert_eq!(
            mac,
            Path::new("/Users/example/Library/Application Support/panefm/trash")
        );
        assert_eq!(
            windows,
            Path::new(r"C:\Users\example\AppData\Local")
                .join("panefm")
                .join("trash")
        );
        assert!(!mac.starts_with(cwd));
        assert!(!windows.starts_with(cwd));
    }

    #[test]
    /// 驗證明確設定的 Trash 位置永遠高於各平台預設值。
    /// 保護目的：公司環境可能要求把敏感刪除資料放到受控磁碟，不能因跨平台路徑調整
    /// 而忽略管理者或使用者既有的 `TFM_TRASH_DIR` 設定。
    fn custom_trash_path_overrides_platform_defaults() {
        let resolved = resolve_trash_root_from_environment(
            Path::new("/workspace/project"),
            Some(OsString::from("/secure/panefm-trash")),
            Some(OsString::from("/state/panefm")),
            None,
            None,
            Some(OsString::from("/Users/example")),
            true,
        );

        assert_eq!(resolved, Path::new("/secure/panefm-trash"));
    }

    #[test]
    /// 驗證升級後仍可讀取既有工作目錄中的 Trash，但明確自訂路徑不會被舊目錄攔截。
    /// 保護目的：修正 Trash 資料位置時不可讓使用者原本可還原的項目突然從面板消失；
    /// 同時公司環境的受控儲存設定仍必須擁有最高優先權。
    fn existing_legacy_trash_remains_visible_until_user_clears_it() {
        let dir = tempdir().expect("tempdir");
        let legacy = dir.path().join(".tfm/trash");
        let platform = dir.path().join("platform/trash");
        fs::create_dir_all(&legacy).expect("legacy trash");

        assert_eq!(
            prefer_existing_legacy_trash(dir.path(), platform.clone(), false),
            legacy
        );
        assert_eq!(
            prefer_existing_legacy_trash(dir.path(), platform.clone(), true),
            platform
        );
    }

    #[test]
    /// 驗證丟進 trash 的檔案可以再還原回原位置。
    /// 保護目的：避免 trash 儲存格式或還原流程重構後，遺失原始路徑、內容或 metadata。
    fn trash_store_can_restore_latest_file() {
        let dir = tempdir().expect("tempdir");
        let trash_dir = dir.path().join(".tfm").join("trash");
        let file_path = dir.path().join("note.txt");
        fs::write(&file_path, "hello").expect("file");

        let store = TrashStore::new(dir.path()).expect("store");
        store
            .trash_path(&file_path, "note.txt")
            .expect("trash file");
        assert!(!file_path.exists());
        assert!(trash_dir.exists());

        let restored = store.restore_latest().expect("restore").expect("item");
        assert_eq!(restored.display_name, "note.txt");
        assert_eq!(restored.restored_path, file_path);
        assert!(restored.restored_path.exists());
    }

    #[test]
    /// 驗證可以依指定 id 清單永久刪除 trash 項目，且之後不再能還原。
    /// 保護目的：避免 trash 儲存格式或還原流程重構後，遺失原始路徑、內容或 metadata。
    fn trash_store_can_delete_entry_permanently() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("delete-me.txt");
        fs::write(&file_path, "hello").expect("file");

        let store = TrashStore::new(dir.path()).expect("store");
        store
            .trash_path(&file_path, "delete-me.txt")
            .expect("trash file");

        let entry = store
            .list_entries()
            .expect("list entries")
            .into_iter()
            .next()
            .expect("trash entry");
        let deleted_names = store
            .delete_many_by_ids(std::slice::from_ref(&entry.id))
            .expect("delete permanently");

        assert_eq!(deleted_names, vec![String::from("delete-me.txt")]);
        assert!(store.list_entries().expect("list after delete").is_empty());
        assert!(store.restore_latest().expect("restore latest").is_none());
    }

    #[test]
    /// 驗證可以一次用批次刪除 API 清空整個 trash，並回傳實際清除的數量。
    /// 保護目的：避免 trash 儲存格式或還原流程重構後，遺失原始路徑、內容或 metadata。
    fn trash_store_can_clear_all_entries() {
        let dir = tempdir().expect("tempdir");
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        fs::write(&first, "one").expect("first");
        fs::write(&second, "two").expect("second");

        let store = TrashStore::new(dir.path()).expect("store");
        store.trash_path(&first, "first.txt").expect("trash first");
        store
            .trash_path(&second, "second.txt")
            .expect("trash second");

        let ids = store
            .list_entries()
            .expect("list entries")
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        let cleared = store.delete_many_by_ids(&ids).expect("clear trash").len();

        assert_eq!(cleared, 2);
        assert!(store.list_entries().expect("list after clear").is_empty());
    }
}
