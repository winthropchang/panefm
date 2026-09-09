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
