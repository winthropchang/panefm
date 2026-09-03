//! 管理集中式 `undoBackup` 目錄、覆蓋備份命名與 Trash 聯動清理。
//!
//! 在執行檔同一層目錄維護 `undoBackup` 目錄，所有因覆蓋或搬移產生供 Undo 使用的
//! 備份皆統一放置於此，避免污染專案目錄。當 Trash 永久刪除檔案或清空時，
//! 會同步在此目錄中清除對應或全部的備份資料。

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering as AtomicOrdering},
};

static UNDO_BACKUP_SEQUENCE: AtomicUsize = AtomicUsize::new(1);

/// 解析放置 Undo 備份的專屬目錄。
///
/// 依據規格，優先放置在執行檔同一層的 `undoBackup` 目錄；
/// 若執行檔目錄唯讀或環境特殊，平滑退回 platform state 或暫存目錄。
pub fn resolve_undo_backup_dir() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(custom) = std::env::var_os("PANEFM_UNDO_BACKUP_DIR") {
            let path = PathBuf::from(custom);
            let _ = fs::create_dir_all(&path);
            return path;
        }
        let fallback =
            std::env::temp_dir().join(format!("panefm-test-undobackup-{}", std::process::id()));
        let _ = fs::create_dir_all(&fallback);
        fallback
    }

    #[cfg(not(test))]
    {
        if let Some(custom) = std::env::var_os("PANEFM_UNDO_BACKUP_DIR") {
            let path = PathBuf::from(custom);
            if fs::create_dir_all(&path).is_ok() {
                return path;
            }
        }

        if let Ok(exe_path) = std::env::current_exe()
            && let Some(exe_dir) = exe_path.parent()
        {
            let candidate = exe_dir.join("undoBackup");
            if fs::create_dir_all(&candidate).is_ok() {
                return candidate;
            }
        }

        if let Some(data_dir) = std::env::var_os("PANEFM_STATE_DIR") {
            let candidate = PathBuf::from(data_dir).join("undoBackup");
            if fs::create_dir_all(&candidate).is_ok() {
                return candidate;
            }
        }

        let fallback = std::env::temp_dir().join("undoBackup");
        let _ = fs::create_dir_all(&fallback);
        fallback
    }
}

/// 在 `undoBackup` 目錄中建立具原始名稱與序號識別的唯一備份路徑。
pub fn create_unique_undo_backup_path(target_path: &Path) -> PathBuf {
    create_unique_undo_backup_path_in(target_path, &resolve_undo_backup_dir())
}

/// 在指定備份目錄中建立具原始名稱與序號識別的唯一備份路徑。
pub fn create_unique_undo_backup_path_in(target_path: &Path, backup_dir: &Path) -> PathBuf {
    let original_name = target_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("target"));
    loop {
        let sequence = UNDO_BACKUP_SEQUENCE.fetch_add(1, AtomicOrdering::Relaxed);
        let candidate = backup_dir.join(format!(
            "{original_name}-{}-{sequence}.backup",
            std::process::id()
        ));
        if !candidate.exists() {
            return candidate;
        }
    }
}

/// 依據 Trash 永久刪除的檔案名稱，同步刪除 `undoBackup` 目錄下對應的備份檔案或資料夾。
pub fn sync_delete_from_undo_backup(target_names: &[String]) -> io::Result<usize> {
    sync_delete_from_undo_backup_in(target_names, &resolve_undo_backup_dir())
}

/// 在指定目錄中依據檔案名稱同步刪除對應的備份檔案或資料夾。
pub fn sync_delete_from_undo_backup_in(
    target_names: &[String],
    backup_dir: &Path,
) -> io::Result<usize> {
    if !backup_dir.exists() {
        return Ok(0);
    }
    let mut removed_count = 0;
    for entry in fs::read_dir(backup_dir)? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        for target in target_names {
            let prefix = format!("{target}-");
            if file_name.starts_with(&prefix) || &file_name == target {
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
                removed_count += 1;
                break;
            }
        }
    }
    Ok(removed_count)
}

/// 清空 `undoBackup` 目錄下的所有備份檔案與資料夾。
pub fn clear_undo_backup_dir() -> io::Result<usize> {
    clear_undo_backup_dir_in(&resolve_undo_backup_dir())
}

/// 清空指定備份目錄下的所有備份檔案與資料夾。
pub fn clear_undo_backup_dir_in(backup_dir: &Path) -> io::Result<usize> {
    if !backup_dir.exists() {
        return Ok(0);
    }
    let mut removed_count = 0;
    for entry in fs::read_dir(backup_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
        removed_count += 1;
    }
    Ok(removed_count)
}

/// 判斷檔名是否屬於 PaneFM 內部的暫存檔案（如寫入時的 `.part` 暫存檔）。
pub fn is_internal_temporary_name(name: &str) -> bool {
    name.starts_with(".panefm-transfer-")
}
