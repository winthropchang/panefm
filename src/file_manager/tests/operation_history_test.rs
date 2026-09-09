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

#[test]
/// 驗證 move_path_with_fallback 能正確移動檔案與目錄。
fn move_path_with_fallback_moves_files_and_directories() {
    let dir = tempdir().expect("tempdir");
    let source_file = dir.path().join("a.txt");
    let target_file = dir.path().join("b.txt");
    fs::write(&source_file, "content").expect("write");
    super::move_path_with_fallback(&source_file, &target_file).expect("move file");
    assert!(!source_file.exists());
    assert_eq!(fs::read_to_string(&target_file).expect("read"), "content");

    let source_dir = dir.path().join("folder");
    let target_dir = dir.path().join("folder2");
    fs::create_dir(&source_dir).expect("mkdir");
    fs::write(source_dir.join("sub.txt"), "sub").expect("write sub");
    super::move_path_with_fallback(&source_dir, &target_dir).expect("move dir");
    assert!(!source_dir.exists());
    assert_eq!(
        fs::read_to_string(target_dir.join("sub.txt")).expect("read sub"),
        "sub"
    );
}

use std::path::PathBuf;
