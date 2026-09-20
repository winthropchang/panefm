use super::*;

#[test]
/// 驗證 pane 重新載入目錄時，資料夾會排在檔案前面。
///
/// 參數：無。
/// 回傳：無；若排序規則錯誤則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_lists_directories_before_files() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("nested")).expect("nested dir");
    fs::write(dir.path().join("alpha.txt"), "hello").expect("file");

    let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let names: Vec<String> = pane.entries.iter().map(FileEntry::display_name).collect();

    assert_eq!(
        names,
        vec![String::from("nested/"), String::from("alpha.txt")]
    );
}

#[test]
/// 驗證 pane 可以正確進入子目錄並返回父目錄。
///
/// 參數：無。
/// 回傳：無；若目錄切換行為錯誤則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_enters_and_leaves_directories() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir(dir.path().join("alpha")).expect("alpha dir");
    let child = dir.path().join("child");
    fs::create_dir(&child).expect("child dir");
    fs::write(child.join("note.txt"), "hello").expect("note");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.move_down_by(1);
    pane.enter_selected().expect("enter child");
    assert_eq!(pane.cwd, child);

    pane.go_parent().expect("back parent");
    assert_eq!(pane.cwd, dir.path());
    assert_eq!(
        pane.selected_entry().map(FileEntry::display_name),
        Some(String::from("child/"))
    );
}

#[test]
/// 驗證 `PaneState` 可以正確刪除目前選取的檔案。
///
/// 參數：無。
/// 回傳：無；若檔案未被刪除或狀態未更新則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_delete_selected_file_removes_it() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let removed = pane.delete_selected().expect("delete");

    assert_eq!(removed, Some(String::from("alpha.txt")));
    assert!(!file_path.exists());
    assert!(pane.entries.is_empty());
}

#[test]
/// 驗證 `PaneState` 可以正確重新命名目前選取的檔案。
///
/// 參數：無。
/// 回傳：無；若檔案未改名或狀態未更新則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_rename_selected_file_updates_entry() {
    let dir = tempdir().expect("tempdir");
    let old_path = dir.path().join("alpha.txt");
    let new_path = dir.path().join("beta.txt");
    fs::write(&old_path, "hello").expect("file");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let renamed = pane.rename_selected("beta.txt").expect("rename");

    assert_eq!(renamed, Some(String::from("beta.txt")));
    assert!(!old_path.exists());
    assert!(new_path.exists());
    assert_eq!(
        pane.selected_entry().map(FileEntry::display_name),
        Some(String::from("beta.txt"))
    );
}

#[test]
/// 驗證同一個目錄內重複複製檔案時，會自動產生不衝突的新檔名。
///
/// 參數：無。
/// 回傳：無；若重複名稱處理錯誤則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_copy_into_same_directory_creates_duplicate_file_name() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let first_copy = pane
        .copy_entry_into_current_dir(&file_path)
        .expect("first copy");
    let second_copy = pane
        .copy_entry_into_current_dir(&file_path)
        .expect("second copy");

    assert_eq!(first_copy, "alpha copy.txt");
    assert_eq!(second_copy, "alpha copy 2.txt");
    assert!(dir.path().join("alpha copy.txt").exists());
    assert!(dir.path().join("alpha copy 2.txt").exists());
}

#[test]
/// 驗證貼上前取得的預計目標路徑與同名複製規則完全一致。
/// 保護目的：避免錯誤訊息顯示原始檔名，但實際失敗位置是 `copy` 名稱而誤導 SMB 除錯。
fn pane_state_planned_paste_target_uses_actual_duplicate_name() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("alpha.txt");
    fs::write(&file_path, "hello").expect("file");
    fs::write(dir.path().join("alpha copy.txt"), "existing").expect("existing copy");
    let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

    let planned = pane
        .planned_paste_target(&file_path, false)
        .expect("planned target");

    assert_eq!(planned, dir.path().join("alpha copy 2.txt"));
    assert!(!planned.exists());
}

#[test]
/// 驗證同一個目錄內重複複製資料夾時，也會自動產生不衝突的新名稱。
///
/// 參數：無。
/// 回傳：無；若資料夾重複名稱處理錯誤則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_copy_into_same_directory_creates_duplicate_directory_name() {
    let dir = tempdir().expect("tempdir");
    let folder_path = dir.path().join("docs");
    fs::create_dir(&folder_path).expect("folder");
    fs::write(folder_path.join("note.txt"), "hello").expect("note");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let first_copy = pane
        .copy_entry_into_current_dir(&folder_path)
        .expect("first copy");
    let second_copy = pane
        .copy_entry_into_current_dir(&folder_path)
        .expect("second copy");

    assert_eq!(first_copy, "docs copy/");
    assert_eq!(second_copy, "docs copy 2/");
    assert!(dir.path().join("docs copy").is_dir());
    assert!(dir.path().join("docs copy 2").is_dir());
    assert!(dir.path().join("docs copy").join("note.txt").exists());
    assert!(dir.path().join("docs copy 2").join("note.txt").exists());
}

#[test]
/// 驗證 `PaneState` 可以依照一般名稱建立新檔案並將焦點移到新檔案。
///
/// 參數：無。
/// 回傳：無；若檔案未建立或選取狀態錯誤則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_create_plain_file_adds_new_entry() {
    let dir = tempdir().expect("tempdir");
    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

    let created = pane.create_entry("note.txt").expect("create file");

    assert_eq!(created, "note.txt");
    assert!(dir.path().join("note.txt").exists());
    assert_eq!(
        pane.selected_entry().map(FileEntry::display_name),
        Some(String::from("note.txt"))
    );
}

#[test]
/// 驗證 `PaneState` 可以依照結尾 `/` 建立新資料夾並將焦點移到新資料夾。
///
/// 參數：無。
/// 回傳：無；若資料夾未建立或選取狀態錯誤則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_create_directory_from_trailing_slash_adds_new_entry() {
    let dir = tempdir().expect("tempdir");
    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

    let created = pane.create_entry("workspace/").expect("create directory");

    assert_eq!(created, "workspace/");
    assert!(dir.path().join("workspace").is_dir());
    assert_eq!(
        pane.selected_entry().map(FileEntry::display_name),
        Some(String::from("workspace/"))
    );
}

#[test]
/// 驗證巢狀建立會自動補齊父目錄，並在最後建立檔案。
///
/// 參數：無。
/// 回傳：無；若父目錄未建立或檔案未建立則測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_create_nested_file_builds_parent_directories() {
    let dir = tempdir().expect("tempdir");
    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");

    let created = pane
        .create_entry("test/gg.txt")
        .expect("create nested file");

    assert_eq!(created, "test/gg.txt");
    assert!(dir.path().join("test").is_dir());
    assert!(dir.path().join("test").join("gg.txt").exists());
    assert_eq!(
        pane.selected_entry().map(FileEntry::display_name),
        Some(String::from("test/"))
    );
}

#[test]
/// 驗證預設不顯示隱藏檔，切換後才會出現在列表中。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_toggle_hidden_changes_visible_entries() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join(".secret"), "s").expect("hidden");
    fs::write(dir.path().join("alpha.txt"), "a").expect("normal");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let initial_names: Vec<String> = pane
        .visible_entries()
        .into_iter()
        .map(FileEntry::display_name)
        .collect();
    assert_eq!(initial_names, vec![String::from("alpha.txt")]);

    pane.toggle_hidden();
    let toggled_names: Vec<String> = pane
        .visible_entries()
        .into_iter()
        .map(FileEntry::display_name)
        .collect();
    assert_eq!(
        toggled_names,
        vec![String::from(".secret"), String::from("alpha.txt")]
    );
}

#[test]
/// 驗證當 cancelled 旗標被設定時，read_dir_entries_with_cancellation 會立即中斷並回傳 Interrupted。
/// 保護目的：確保使用者在大型目錄快速切換時，舊的 worker 會提早退出，不會持續佔用磁碟 I/O。
fn read_dir_entries_with_cancellation_aborts_promptly() {
    let dir = tempdir().expect("tempdir");
    for i in 0..10 {
        fs::write(dir.path().join(format!("file_{i}.txt")), b"data").expect("write");
    }

    let cancelled = AtomicBool::new(true);
    let result = read_dir_entries_with_cancellation(dir.path(), &cancelled);
    assert!(result.is_err());
    assert_eq!(
        result.expect_err("must error").kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
/// 驗證 stream_dir_entries_with_cancellation 會回傳正確排序且 metadata 完整的清單。
///
/// 保護目的：背景導航採 natural 排序時仍須補齊檔案大小。舊實作為了縮短載入時間，
/// 在 natural 排序的 Complete 事件也把 size 固定為 0，導致已啟用 `ms` 的舊 panel
/// 永久顯示 0B；同一路徑另開的新 panel 卻正常。此測試同時保護排序與非零大小。
fn stream_dir_entries_with_cancellation_loads_and_completes_sorted() {
    let dir = tempdir().expect("tempdir");
    for i in 0..300 {
        fs::write(dir.path().join(format!("file_{i:03}.txt")), b"sample").expect("write");
    }

    let cancelled = AtomicBool::new(false);
    let mut got_complete = false;

    let result = stream_dir_entries_with_cancellation(
        dir.path(),
        SortMode::Natural { reverse: false },
        0,
        &cancelled,
        |progress| {
            match progress {
                DirectoryLoadProgress::Batch { .. } => {}
                DirectoryLoadProgress::Complete(entries) => {
                    assert_eq!(entries.len(), 300);
                    assert_eq!(entries[0].name, "file_000.txt");
                    assert_eq!(entries[299].name, "file_299.txt");
                    assert!(
                        entries.iter().all(|entry| entry.size == 6),
                        "natural 排序的背景完整結果也必須包含真實檔案大小"
                    );
                    got_complete = true;
                }
            }
            true
        },
    );

    assert!(result.is_ok());
    assert!(got_complete, "必須完成最終 Complete 步驟");
}
