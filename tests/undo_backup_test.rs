use panefm::file_manager::undo_backup::{
    clear_undo_backup_dir_in, create_unique_undo_backup_path_in, is_internal_temporary_name,
    sync_delete_from_undo_backup_in,
};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

#[test]
/// 驗證 undoBackup 命名結構與單檔同步刪除。
/// 保護目的：確認覆蓋備份能正確生成獨立唯一的備份路徑，並能在 Trash 永久刪除時同步清除。
fn undo_backup_path_generation_and_sync_delete() {
    let dir = tempdir().expect("tempdir");
    let backup_dir = dir.path().join("undoBackup");
    fs::create_dir_all(&backup_dir).expect("create dir");

    let target_file = PathBuf::from("/some/project/LogoIcon.png");
    let backup_path = create_unique_undo_backup_path_in(&target_file, &backup_dir);

    assert!(backup_path.starts_with(&backup_dir));
    assert!(
        backup_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("LogoIcon.png-")
    );

    fs::write(&backup_path, "old content").expect("write backup");
    assert!(backup_path.exists());

    // 模擬從 Trash 永久刪除 LogoIcon.png，驗證同步刪除
    let deleted = sync_delete_from_undo_backup_in(&["LogoIcon.png".to_string()], &backup_dir)
        .expect("sync delete");
    assert_eq!(deleted, 1);
    assert!(!backup_path.exists());
}

#[test]
/// 驗證清空 undoBackup 目錄下所有項目。
/// 保護目的：確認清空 Trash 時能同步清空所有備份檔案。
fn clear_undo_backup_dir_removes_all() {
    let dir = tempdir().expect("tempdir");
    let backup_dir = dir.path().join("undoBackup");
    fs::create_dir_all(&backup_dir).expect("create dir");

    let file1 = backup_dir.join("test1-1.backup");
    let file2 = backup_dir.join("test2-2.backup");
    fs::write(&file1, "b1").expect("write");
    fs::write(&file2, "b2").expect("write");

    let cleared = clear_undo_backup_dir_in(&backup_dir).expect("clear");
    assert_eq!(cleared, 2);
    assert!(!file1.exists());
    assert!(!file2.exists());
}

#[test]
/// 驗證識別 PaneFM 內部暫存檔名。
/// 保護目的：確保內部傳輸的 .part 等暫存檔不會被當作一般檔案處理。
fn recognizes_internal_temporary_name() {
    assert!(is_internal_temporary_name(".panefm-transfer-123-1.part"));
    assert!(!is_internal_temporary_name(".gitignore"));
    assert!(!is_internal_temporary_name(".DS_Store"));
}
