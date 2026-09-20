use super::*;

#[test]
/// 驗證自然排序不需要把數字轉成 `u64`，超長數字與前置零仍有穩定順序。
///
/// 保護目的：大型 build 目錄含有數萬個帶 hash 或數字的檔名；自然排序改為零配置
/// 比較器後，必須同時保留 `file2 < file10` 的語意，且不可因數字溢位退回文字排序。
fn natural_compare_handles_large_numbers_without_allocating_numeric_strings() {
    assert!(natural_cmp("file2", "file10").is_lt());
    assert!(natural_cmp("file0002", "file2").is_gt());
    assert!(
        natural_cmp(
            "build999999999999999999999999",
            "build1000000000000000000000000"
        )
        .is_lt()
    );
    assert!(natural_cmp("中文2", "中文10").is_lt());
}

#[test]
/// 驗證超過平行化門檻的大型目錄仍完整讀取每一筆 metadata。
///
/// 保護目的：大型目錄改成多 worker 後，chunk 邊界不能遺漏或重複項目；檔案大小也
/// 必須保持準確，否則 size linemode、排序與後續傳輸估算都會得到錯誤資料。
fn large_directory_metadata_loading_keeps_every_entry_and_size() {
    let directory = tempdir().expect("tempdir");
    for index in 0..520usize {
        fs::write(
            directory.path().join(format!("entry-{index}.bin")),
            [index as u8],
        )
        .expect("write fixture");
    }

    let entries = read_dir_entries(directory.path()).expect("read large directory");

    assert_eq!(entries.len(), 520);
    assert!(entries.iter().all(|entry| entry.size == 1));
}

#[test]
/// 驗證一般目錄載入不會同步遞迴統計大小，避免本機大型目錄或 SMB 首次瀏覽卡住。
///
/// 參數：無。
/// 回傳：無；若預設載入已填入目錄容量，測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_defers_directory_sizes_until_requested() {
    let dir = tempdir().expect("tempdir");
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).expect("nested dir");
    fs::write(nested.join("one.txt"), "one").expect("first child");

    let pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");

    assert_eq!(directory.directory_size, None);
    assert!(!directory.directory_size_complete);
}

#[test]
/// 驗證背景掃描的部分大小與完成狀態可以分階段更新同一個目錄。
///
/// 參數：無。
/// 回傳：無；若部分值、最終值或完成旗標不正確，測試失敗。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_applies_incremental_directory_size_snapshot() {
    let dir = tempdir().expect("tempdir");
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).expect("nested dir");
    fs::write(nested.join("one.txt"), "one").expect("first child");
    fs::write(nested.join("two.txt"), "two").expect("second child");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    assert!(pane.update_directory_size(&nested, 3, false));
    let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");

    assert_eq!(directory.directory_size, Some(3));
    assert!(!directory.directory_size_complete);

    assert!(pane.update_directory_size(&nested, 6, true));
    let directory = pane.entries.iter().find(|entry| entry.is_dir).expect("dir");
    assert_eq!(directory.directory_size, Some(6));
    assert!(directory.directory_size_complete);
}

#[test]
/// 驗證 watcher 或背景貼上觸發列表 reload 時，不會把正在顯示的目錄大小清空。
///
/// 保護目的：`ms` 掃描可能持續數秒；若每次外部檔案事件都重設快取，畫面會反覆
/// 跳回 `…` 或 `~0B`，大型目錄甚至永遠無法顯示完成值。
fn pane_reload_preserves_directory_size_cache_for_existing_paths() {
    let dir = tempdir().expect("tempdir");
    let nested = dir.path().join("nested");
    fs::create_dir(&nested).expect("nested dir");
    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    assert!(pane.update_directory_size(&nested, 42_000, true));

    fs::write(dir.path().join("new.txt"), "new").expect("external file");
    pane.reload().expect("reload");

    let directory = pane
        .entries
        .iter()
        .find(|entry| entry.path == nested)
        .expect("dir");
    assert_eq!(directory.directory_size, Some(42_000));
    assert!(directory.directory_size_complete);
}

#[test]
/// 驗證排序模式會提供對應的人類可讀標籤。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn sort_mode_labels_match_expected_names() {
    assert_eq!(
        SortMode::Alphabetical { reverse: false }.label(),
        "alphabetical"
    );
    assert_eq!(SortMode::Size { reverse: true }.label(), "size (reverse)");
    assert_eq!(SortMode::Modified { reverse: false }.label(), "modified");
}

#[test]
/// 驗證切換到大小排序後，較大的檔案會排在前面。
/// 保護目的：避免目錄載入、排序、預覽或檔案操作重構後，破壞單一 panel 的資料一致性。
fn pane_state_sort_by_size_reorders_files() {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("small.txt"), "a").expect("small");
    fs::write(dir.path().join("large.txt"), "abcdef").expect("large");

    let mut pane = PaneState::new(dir.path().to_path_buf()).expect("pane");
    pane.set_sort_mode(SortMode::Size { reverse: true });

    let names: Vec<String> = pane
        .visible_entries()
        .into_iter()
        .map(FileEntry::display_name)
        .collect();
    assert_eq!(
        names,
        vec![String::from("large.txt"), String::from("small.txt")]
    );
    assert_eq!(pane.sort_mode, SortMode::Size { reverse: true });
}
