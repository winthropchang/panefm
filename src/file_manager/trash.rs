use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

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

    /// 依照指定的 trash 記錄 id 還原對應項目。
    ///
    /// 參數：
    /// - `id: &str`，要還原的 trash 項目 id。
    ///
    /// 回傳：`io::Result<Option<RestoreResult>>`。
    pub(crate) fn restore_by_id(&self, id: &str) -> io::Result<Option<RestoreResult>> {
        let Some(record) = self
            .load_records()?
            .into_iter()
            .find(|record| record.id == id)
        else {
            return Ok(None);
        };

        self.restore_record(record).map(Some)
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

    /// 永久刪除指定 id 的 trash 項目。
    ///
    /// 參數：
    /// - `id: &str`，要永久刪除的 trash 紀錄 id。
    ///
    /// 回傳：`io::Result<Option<String>>`。
    /// - `Ok(Some(name))` 代表成功刪除，並回傳顯示名稱。
    /// - `Ok(None)` 代表找不到該項目。
    pub(crate) fn delete_by_id(&self, id: &str) -> io::Result<Option<String>> {
        let Some(record) = self
            .load_records()?
            .into_iter()
            .find(|record| record.id == id)
        else {
            return Ok(None);
        };

        self.delete_record(record).map(Some)
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

    /// 永久清空整個 trash。
    ///
    /// 參數：無。
    ///
    /// 回傳：`io::Result<usize>`，代表實際刪除的項目數。
    pub(crate) fn clear(&self) -> io::Result<usize> {
        let ids = self
            .list_entries()?
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        Ok(self.delete_many_by_ids(&ids)?.len())
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
    if let Some(custom) = env::var_os("TFM_TRASH_DIR") {
        return PathBuf::from(custom);
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

/// 依路徑型別刪除檔案或整個資料夾樹。
fn remove_path(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
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
    use std::fs;

    use tempfile::tempdir;

    use super::TrashStore;

    #[test]
    /// 驗證丟進 trash 的檔案可以再還原回原位置。
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
    /// 驗證可以依指定 id 永久刪除 trash 項目，且之後不再能還原。
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
        let deleted_name = store
            .delete_by_id(&entry.id)
            .expect("delete permanently")
            .expect("deleted item");

        assert_eq!(deleted_name, "delete-me.txt");
        assert!(store.list_entries().expect("list after delete").is_empty());
        assert!(store.restore_latest().expect("restore latest").is_none());
    }

    #[test]
    /// 驗證可以一次清空整個 trash，並回傳實際清除的數量。
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

        let cleared = store.clear().expect("clear trash");

        assert_eq!(cleared, 2);
        assert!(store.list_entries().expect("list after clear").is_empty());
    }
}
