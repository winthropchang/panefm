use panefm::file_manager::diff::{
    DiffEntryState, DiffFilterMode, DiffMatrixRow, DiffMatrixState, DiffStatus,
    compute_diff_matrix, launch_content_diff_spec,
};
use panefm::file_manager::open::LaunchMode;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[test]
/// 驗證計算差異矩陣時能準確識別 Identical、Modified、Exclusive 與 Subset 狀態。
/// 保護目的：確保多端目錄比對時的四種基本狀態判斷邏輯完全正確。
fn compute_diff_matrix_identifies_identical_modified_and_exclusive() {
    let dir1 = tempdir().expect("dir1");
    let dir2 = tempdir().expect("dir2");
    let dir3 = tempdir().expect("dir3");

    // 1. 完全一致檔案 (在 1, 2, 3 都有且內容相同)
    fs::write(dir1.path().join("same.txt"), b"hello world").expect("w1");
    fs::write(dir2.path().join("same.txt"), b"hello world").expect("w2");
    fs::write(dir3.path().join("same.txt"), b"hello world").expect("w3");

    // 2. 內容不同檔案 (在 1, 2 都有但內容不同)
    fs::write(dir1.path().join("diff.txt"), b"version A").expect("w1");
    fs::write(dir2.path().join("diff.txt"), b"version B").expect("w2");

    // 3. 獨有檔案 (僅在 3 有)
    fs::write(dir3.path().join("only3.txt"), b"exclusive to 3").expect("w3");

    // 4. 子集一致檔案 (在 1, 2 有且內容相同，但 3 沒有)
    fs::write(dir1.path().join("subset.txt"), b"subset content").expect("w1");
    fs::write(dir2.path().join("subset.txt"), b"subset content").expect("w2");

    let roots = vec![
        dir1.path().to_path_buf(),
        dir2.path().to_path_buf(),
        dir3.path().to_path_buf(),
    ];

    let rows = compute_diff_matrix(&roots).expect("matrix");
    assert_eq!(rows.len(), 4);

    let same_row = rows
        .iter()
        .find(|r| r.relative_path == Path::new("same.txt"))
        .unwrap();
    assert_eq!(same_row.status, DiffStatus::Identical);

    let diff_row = rows
        .iter()
        .find(|r| r.relative_path == Path::new("diff.txt"))
        .unwrap();
    assert_eq!(diff_row.status, DiffStatus::Modified);

    let only3_row = rows
        .iter()
        .find(|r| r.relative_path == Path::new("only3.txt"))
        .unwrap();
    assert!(matches!(
        only3_row.status,
        DiffStatus::Exclusive { panel_index: 2 }
    ));

    let subset_row = rows
        .iter()
        .find(|r| r.relative_path == Path::new("subset.txt"))
        .unwrap();
    assert_eq!(subset_row.status, DiffStatus::Subset);
}

#[test]
/// 驗證 Diff 篩選模式的循環切換與篩選列計算。
/// 保護目的：確保使用者在 UI 按 `f` 切換篩選模式時能正確過濾顯示項目。
fn diff_filter_mode_cycles_and_filters_correctly() {
    let dir1 = tempdir().expect("dir1");
    let dir2 = tempdir().expect("dir2");

    fs::write(dir1.path().join("same.txt"), b"same").expect("w1");
    fs::write(dir2.path().join("same.txt"), b"same").expect("w2");

    fs::write(dir1.path().join("diff.txt"), b"diff1").expect("w1");
    fs::write(dir2.path().join("diff.txt"), b"diff2").expect("w2");

    fs::write(dir1.path().join("only1.txt"), b"only1").expect("w1");

    let roots = vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()];
    let labels = vec![String::from("Dir1"), String::from("Dir2")];
    let mut state = DiffMatrixState::new_sync(vec![1, 2], roots, labels).expect("state");

    // All: 3 rows
    assert_eq!(state.filtered_indices.len(), 3);

    // DiffOnly: 2 rows (diff.txt, only1.txt)
    state.cycle_filter_mode();
    assert_eq!(state.filter_mode, DiffFilterMode::DiffOnly);
    assert_eq!(state.filtered_indices.len(), 2);

    // ExclusiveOnly: 1 row (only1.txt)
    state.cycle_filter_mode();
    assert_eq!(state.filter_mode, DiffFilterMode::ExclusiveOnly);
    assert_eq!(state.filtered_indices.len(), 1);

    // IdenticalOnly: 1 row (same.txt)
    state.cycle_filter_mode();
    assert_eq!(state.filter_mode, DiffFilterMode::IdenticalOnly);
    assert_eq!(state.filtered_indices.len(), 1);
}

#[test]
/// 驗證外部 Diff 命令規格的產生。
/// 保護目的：確保按下 `d` 啟動外部比對程式時回傳合法的命令規格。
fn launch_content_diff_spec_builds_valid_spec() {
    let roots = vec![
        PathBuf::from("/a"),
        PathBuf::from("/b"),
        PathBuf::from("/c"),
    ];
    let row = DiffMatrixRow {
        relative_path: PathBuf::from("src/main.rs"),
        is_dir: false,
        panel_states: vec![
            DiffEntryState::Present {
                size: 10,
                hash: 1,
                is_dir: false,
            },
            DiffEntryState::Present {
                size: 12,
                hash: 2,
                is_dir: false,
            },
            DiffEntryState::Present {
                size: 10,
                hash: 1,
                is_dir: false,
            },
        ],
        status: DiffStatus::Modified,
        display_size: 12,
    };

    let spec = launch_content_diff_spec(&roots, &row);
    assert!(spec.is_some());
    let spec = spec.unwrap();
    assert_eq!(spec.mode, LaunchMode::TerminalBlocking);
}

#[test]
/// 驗證 Diff 矩陣排序：所有非 Identical（不同的）檔案自動排在最前，Identical 排在後方，且統計數正確。
/// 保護目的：避免大量檔案比對時，差異項目混雜在相同項目中難以檢視。
fn diff_sorts_differences_before_identical_items_and_computes_counts() {
    let dir1 = tempdir().expect("dir1");
    let dir2 = tempdir().expect("dir2");

    // 相同的檔案
    fs::write(dir1.path().join("a_same.txt"), b"identical content").expect("w1");
    fs::write(dir2.path().join("a_same.txt"), b"identical content").expect("w2");
    fs::write(dir1.path().join("z_same.txt"), b"identical content").expect("w1");
    fs::write(dir2.path().join("z_same.txt"), b"identical content").expect("w2");

    // 內容不同的檔案
    fs::write(dir1.path().join("m_diff.txt"), b"v1").expect("w1");
    fs::write(dir2.path().join("m_diff.txt"), b"v2").expect("w2");

    // 單邊獨有的檔案
    fs::write(dir1.path().join("b_only1.txt"), b"only1").expect("w1");
    fs::write(dir2.path().join("y_only2.txt"), b"only2").expect("w2");

    let roots = vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()];
    let labels = vec![String::from("Dir1"), String::from("Dir2")];
    let state = DiffMatrixState::new_sync(vec![1, 2], roots, labels).expect("state");

    // 總共 5 個項目：3 個不同 (m_diff, b_only1, y_only2)；2 個相同 (a_same, z_same)
    assert_eq!(state.rows.len(), 5);
    assert_eq!(state.different_count(), 3);
    assert_eq!(state.identical_count(), 2);

    // 驗證排序：前 3 個項目全部是非 Identical（不同的排在前面）
    assert_ne!(state.rows[0].status, DiffStatus::Identical);
    assert_ne!(state.rows[1].status, DiffStatus::Identical);
    assert_ne!(state.rows[2].status, DiffStatus::Identical);

    // 驗證排序：後 2 個項目全部是 Identical（相同的排在後面）
    assert_eq!(state.rows[3].status, DiffStatus::Identical);
    assert_eq!(state.rows[4].status, DiffStatus::Identical);

    let names: Vec<_> = state
        .rows
        .iter()
        .map(|r| r.relative_path.to_string_lossy().into_owned())
        .collect();
    // 前半部不同組按路徑排序：b_only1.txt, m_diff.txt, y_only2.txt
    assert_eq!(&names[0..3], &["b_only1.txt", "m_diff.txt", "y_only2.txt"]);
    // 後半部相同組按路徑排序：a_same.txt, z_same.txt
    assert_eq!(&names[3..5], &["a_same.txt", "z_same.txt"]);
}
