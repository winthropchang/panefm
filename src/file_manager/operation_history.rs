//! 檔案操作的記憶體內 Undo 歷史。
//!
//! 一次批次貼上或移動會保存成一筆 `FileOperation`，因此使用者復原時不必逐項處理。
//! 歷史目前不寫入磁碟，避免重開程式後對已變動的檔案系統套用過期操作；覆蓋前內容則
//! 暫存在目標旁的隱藏備份，直到該筆歷史被復原、淘汰或程式正常關閉。

use std::{fs, io, path::PathBuf};

use super::{pane::remove_undo_backup, trash::TrashStore};

/// PaneFM 預設保留的批次操作數量，避免覆蓋備份無限制占用磁碟。
pub(crate) const DEFAULT_HISTORY_LIMIT: usize = 20;

/// 表示一筆歷史是複製還是移動；兩者的反向操作不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileOperationKind {
    Copy,
    Move,
}

/// 記錄批次操作中的一個成功項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationItem {
    /// 操作前的來源位置；Move 復原時會搬回這裡。
    pub(crate) source_path: PathBuf,
    /// 操作完成後的實際目的位置，包含自動產生的 `copy 2` 名稱。
    pub(crate) destination_path: PathBuf,
    /// 覆蓋操作執行前的舊目標備份；沒有同名覆蓋時為 `None`。
    pub(crate) replaced_backup: Option<PathBuf>,
}

/// 表示一次使用者操作所完成的一批檔案系統變更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileOperation {
    pub(crate) kind: FileOperationKind,
    pub(crate) items: Vec<OperationItem>,
}

/// 描述執行一次 Undo 後的結果，供狀態列顯示成功與失敗數量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UndoResult {
    pub(crate) kind: FileOperationKind,
    pub(crate) restored: usize,
    pub(crate) failed: usize,
}

/// 管理全域檔案操作歷史；歷史屬於 App，不屬於單一 panel。
#[derive(Debug)]
pub(crate) struct OperationHistory {
    undo_stack: Vec<FileOperation>,
    limit: usize,
}

impl OperationHistory {
    /// 建立指定容量的 Undo 歷史。
    ///
    /// 參數：`limit: usize`，最多保留的批次操作數；至少會保留一筆。
    /// 回傳：`OperationHistory`，初始為空的歷史管理器。
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// 將一批已成功的檔案操作加入歷史，並清理超出容量的舊備份。
    ///
    /// 參數：`operation: FileOperation`，同一次使用者操作完成的所有項目。
    /// 回傳：`() `；空操作不會被加入。
    pub(crate) fn push(&mut self, operation: FileOperation) {
        if operation.items.is_empty() {
            return;
        }
        self.undo_stack.push(operation);
        while self.undo_stack.len() > self.limit {
            let expired = self.undo_stack.remove(0);
            cleanup_operation_backups(&expired);
        }
    }

    /// 復原最近一次批次操作。
    ///
    /// Copy 建立物會移到 PaneFM Trash；Move 會回到原來源。若單一項目失敗，只把失敗
    /// 項目放回歷史頂端，已成功部分不會在下一次 Undo 被重複處理。
    ///
    /// 參數：`trash_store: &TrashStore`，Copy Undo 安全移除建立物所使用的 trash。
    /// 回傳：`io::Result<Option<UndoResult>>`；`None` 代表沒有可復原操作。
    pub(crate) fn undo_latest(
        &mut self,
        trash_store: &TrashStore,
    ) -> io::Result<Option<UndoResult>> {
        let Some(operation) = self.undo_stack.pop() else {
            return Ok(None);
        };
        let kind = operation.kind;
        let mut restored = 0usize;
        let mut failed_items = Vec::new();
        let mut first_error = None;

        for item in operation.items.into_iter().rev() {
            let result = match kind {
                FileOperationKind::Copy => undo_copy_item(&item, trash_store),
                FileOperationKind::Move => undo_move_item(&item),
            };
            match result {
                Ok(()) => restored += 1,
                Err(error) => {
                    first_error.get_or_insert(error);
                    failed_items.push(item);
                }
            }
        }

        let failed = failed_items.len();
        if !failed_items.is_empty() {
            failed_items.reverse();
            self.undo_stack.push(FileOperation {
                kind,
                items: failed_items,
            });
        }

        if restored == 0
            && let Some(error) = first_error
        {
            return Err(error);
        }
        Ok(Some(UndoResult {
            kind,
            restored,
            failed,
        }))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.undo_stack.len()
    }
}

impl Drop for OperationHistory {
    /// 程式正常離開時清除仍由歷史持有的覆蓋備份，避免留下內部隱藏檔。
    fn drop(&mut self) {
        for operation in &self.undo_stack {
            cleanup_operation_backups(operation);
        }
    }
}

/// 復原單一 Copy：新建立物送進 Trash；覆蓋操作再把舊目標備份放回原位。
fn undo_copy_item(item: &OperationItem, trash_store: &TrashStore) -> io::Result<()> {
    if item.destination_path.exists() {
        let display_name = item
            .destination_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| item.destination_path.display().to_string());
        trash_store.trash_path(&item.destination_path, &display_name)?;
    }
    if let Some(backup) = &item.replaced_backup
        && backup.exists()
    {
        fs::rename(backup, &item.destination_path)?;
    }
    Ok(())
}

/// 復原單一 Move：把目的項目搬回原來源，再還原可能存在的覆蓋前目標。
fn undo_move_item(item: &OperationItem) -> io::Result<()> {
    if item.source_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("undo source already exists: {}", item.source_path.display()),
        ));
    }
    if !item.destination_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "undo destination is missing: {}",
                item.destination_path.display()
            ),
        ));
    }

    fs::rename(&item.destination_path, &item.source_path)?;
    if let Some(backup) = &item.replaced_backup
        && backup.exists()
        && let Err(error) = fs::rename(backup, &item.destination_path)
    {
        let _ = fs::rename(&item.source_path, &item.destination_path);
        return Err(error);
    }
    Ok(())
}

/// 清理由已淘汰歷史持有的覆蓋備份；清理失敗不應阻止主程式關閉。
fn cleanup_operation_backups(operation: &FileOperation) {
    for backup in operation
        .items
        .iter()
        .filter_map(|item| item.replaced_backup.as_deref())
    {
        let _ = remove_undo_backup(backup);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        DEFAULT_HISTORY_LIMIT, FileOperation, FileOperationKind, OperationHistory, OperationItem,
    };
    use crate::file_manager::trash::TrashStore;

    /// 驗證一次批次 Copy 只占一筆歷史，而且 Undo 會把整批建立物一起移到 Trash。
    ///
    /// 保護目的：避免使用者貼錯大量檔案後仍必須逐一刪除，這是本功能的核心情境。
    #[test]
    fn undo_copy_removes_entire_batch_to_trash() {
        let dir = tempdir().expect("tempdir");
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        fs::write(&first, "one").expect("first");
        fs::write(&second, "two").expect("second");
        let trash = TrashStore::new(dir.path()).expect("trash");
        let mut history = OperationHistory::new(DEFAULT_HISTORY_LIMIT);
        history.push(FileOperation {
            kind: FileOperationKind::Copy,
            items: vec![item(&first), item(&second)],
        });

        let result = history
            .undo_latest(&trash)
            .expect("undo")
            .expect("history entry");

        assert_eq!(result.restored, 2);
        assert!(!first.exists());
        assert!(!second.exists());
        assert_eq!(trash.list_entries().expect("trash entries").len(), 2);
        assert_eq!(history.len(), 0);
    }

    /// 驗證 Move Undo 會把目的檔移回原始位置，而不是複製後留下兩份。
    ///
    /// 保護目的：確保 cut/paste 與 move 命令都能精確反向還原原路徑。
    #[test]
    fn undo_move_restores_original_path() {
        let dir = tempdir().expect("tempdir");
        let source = dir.path().join("source.txt");
        let destination = dir.path().join("moved.txt");
        fs::write(&destination, "body").expect("destination");
        let trash = TrashStore::new(dir.path()).expect("trash");
        let mut history = OperationHistory::new(DEFAULT_HISTORY_LIMIT);
        history.push(FileOperation {
            kind: FileOperationKind::Move,
            items: vec![OperationItem {
                source_path: source.clone(),
                destination_path: destination.clone(),
                replaced_backup: None,
            }],
        });

        history.undo_latest(&trash).expect("undo move");

        assert_eq!(
            fs::read_to_string(&source).expect("source restored"),
            "body"
        );
        assert!(!destination.exists());
    }

    /// 驗證連續 Undo 依照後進先出順序處理多筆歷史。
    ///
    /// 保護目的：架構必須從第一版就能支援多次 Undo，避免未來從單一紀錄重寫。
    #[test]
    fn repeated_undo_uses_latest_operation_first() {
        let dir = tempdir().expect("tempdir");
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        fs::write(&first, "one").expect("first");
        fs::write(&second, "two").expect("second");
        let trash = TrashStore::new(dir.path()).expect("trash");
        let mut history = OperationHistory::new(DEFAULT_HISTORY_LIMIT);
        history.push(FileOperation {
            kind: FileOperationKind::Copy,
            items: vec![item(&first)],
        });
        history.push(FileOperation {
            kind: FileOperationKind::Copy,
            items: vec![item(&second)],
        });

        history.undo_latest(&trash).expect("undo second");
        assert!(first.exists());
        assert!(!second.exists());
        history.undo_latest(&trash).expect("undo first");
        assert!(!first.exists());
    }

    /// 驗證覆蓋 Copy Undo 會移除新內容並把覆蓋前備份精確還原。
    ///
    /// 保護目的：避免 `P` 覆蓋後 Undo 只刪除新檔，導致原本資料永久遺失。
    #[test]
    fn undo_overwrite_copy_restores_backup() {
        let dir = tempdir().expect("tempdir");
        let destination = dir.path().join("target.txt");
        let backup = dir.path().join(".backup");
        fs::write(&destination, "new").expect("new target");
        fs::write(&backup, "old").expect("old backup");
        let trash = TrashStore::new(dir.path()).expect("trash");
        let mut history = OperationHistory::new(DEFAULT_HISTORY_LIMIT);
        history.push(FileOperation {
            kind: FileOperationKind::Copy,
            items: vec![OperationItem {
                source_path: dir.path().join("source.txt"),
                destination_path: destination.clone(),
                replaced_backup: Some(backup.clone()),
            }],
        });

        history.undo_latest(&trash).expect("undo overwrite");

        assert_eq!(
            fs::read_to_string(destination).expect("restored old"),
            "old"
        );
        assert!(!backup.exists());
    }

    fn item(destination: &std::path::Path) -> OperationItem {
        OperationItem {
            source_path: PathBuf::new(),
            destination_path: destination.to_path_buf(),
            replaced_backup: None,
        }
    }

    use std::path::PathBuf;
}
