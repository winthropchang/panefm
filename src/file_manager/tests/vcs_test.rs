use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

use super::{
    VcsFileStatus, VcsRepoInfo, VcsType, find_vcs_candidates, find_vcs_root, format_diff_lines,
    is_valid_git_dir, parse_git_status_output, parse_svn_info_output, parse_svn_status_output,
    read_git_branch_from_head, rollup_status_to_ancestors,
};
use crate::theme::{Theme, ThemePreset};

#[test]
/// 驗證 Git porcelain v1 輸出能正確解析為對應的 VcsFileStatus。
fn parse_git_status_porcelain_various_codes() {
    let repo_root = PathBuf::from("/workspace/project");
    let output = concat!(
        " M modified_worktree.rs\n",
        "M  modified_index.rs\n",
        "MM modified_both.rs\n",
        "A  added_staged.rs\n",
        "AM added_modified.rs\n",
        " D deleted_worktree.rs\n",
        "D  deleted_index.rs\n",
        "?? untracked_file.txt\n",
        "UU conflict_file.rs\n",
        "AA conflict_both_added.rs\n",
        "R  old_name.rs -> new_name.rs\n",
        "R  \"old space.rs\" -> \"new space.rs\"\n",
        "T  type_changed.rs\n",
        "!! ignored.txt\n",
    );
    let statuses = parse_git_status_output(output, &repo_root);

    assert_eq!(
        statuses.get(&repo_root.join("modified_worktree.rs")),
        Some(&VcsFileStatus::Modified)
    );
    assert_eq!(
        statuses.get(&repo_root.join("modified_index.rs")),
        Some(&VcsFileStatus::Modified)
    );
    assert_eq!(
        statuses.get(&repo_root.join("modified_both.rs")),
        Some(&VcsFileStatus::Modified)
    );
    assert_eq!(
        statuses.get(&repo_root.join("added_staged.rs")),
        Some(&VcsFileStatus::Added)
    );
    assert_eq!(
        statuses.get(&repo_root.join("added_modified.rs")),
        Some(&VcsFileStatus::Modified)
    );
    assert_eq!(
        statuses.get(&repo_root.join("deleted_worktree.rs")),
        Some(&VcsFileStatus::Deleted)
    );
    assert_eq!(
        statuses.get(&repo_root.join("deleted_index.rs")),
        Some(&VcsFileStatus::Deleted)
    );
    assert_eq!(
        statuses.get(&repo_root.join("untracked_file.txt")),
        Some(&VcsFileStatus::Untracked)
    );
    assert_eq!(
        statuses.get(&repo_root.join("conflict_file.rs")),
        Some(&VcsFileStatus::Conflict)
    );
    assert_eq!(
        statuses.get(&repo_root.join("conflict_both_added.rs")),
        Some(&VcsFileStatus::Conflict)
    );
    assert_eq!(
        statuses.get(&repo_root.join("new_name.rs")),
        Some(&VcsFileStatus::Renamed)
    );
    assert_eq!(
        statuses.get(&repo_root.join("new space.rs")),
        Some(&VcsFileStatus::Renamed)
    );
    assert_eq!(
        statuses.get(&repo_root.join("type_changed.rs")),
        Some(&VcsFileStatus::TypeChanged)
    );
    assert_eq!(
        statuses.get(&repo_root.join("ignored.txt")),
        Some(&VcsFileStatus::Ignored)
    );
}

#[test]
/// 驗證 SVN status 輸出能正確解析第一欄狀態字元。
fn parse_svn_status_various_codes() {
    let repo_root = PathBuf::from("/workspace/svn_repo");
    let output = "\
M       src/main.rs
A       src/new_component.rs
D       src/legacy.rs
!       src/missing.rs
?       notes.txt
C       config.xml
R       replaced.rs
I       target/
~       submodule_link
M       folder with spaces/file.txt
";
    let statuses = parse_svn_status_output(output, &repo_root);

    assert_eq!(
        statuses.get(&repo_root.join("src/main.rs")),
        Some(&VcsFileStatus::Modified)
    );
    assert_eq!(
        statuses.get(&repo_root.join("src/new_component.rs")),
        Some(&VcsFileStatus::Added)
    );
    assert_eq!(
        statuses.get(&repo_root.join("src/legacy.rs")),
        Some(&VcsFileStatus::Deleted)
    );
    assert_eq!(
        statuses.get(&repo_root.join("src/missing.rs")),
        Some(&VcsFileStatus::Deleted)
    );
    assert_eq!(
        statuses.get(&repo_root.join("notes.txt")),
        Some(&VcsFileStatus::Untracked)
    );
    assert_eq!(
        statuses.get(&repo_root.join("config.xml")),
        Some(&VcsFileStatus::Conflict)
    );
    assert_eq!(
        statuses.get(&repo_root.join("replaced.rs")),
        Some(&VcsFileStatus::Renamed)
    );
    assert_eq!(
        statuses.get(&repo_root.join("target")),
        Some(&VcsFileStatus::Ignored)
    );
    assert_eq!(
        statuses.get(&repo_root.join("submodule_link")),
        Some(&VcsFileStatus::TypeChanged)
    );
    assert_eq!(
        statuses.get(&repo_root.join("folder with spaces/file.txt")),
        Some(&VcsFileStatus::Modified)
    );
}

#[test]
/// 驗證 SVN info 輸出能正確解析出分支名稱與修訂版本號。
fn parse_svn_info_trunk_and_branches() {
    let trunk_output = "\
Path: .
Working Copy Root Path: /workspace/svn_repo
URL: https://svn.example.com/svn/project/trunk
Relative URL: ^/trunk
Repository Root: https://svn.example.com/svn/project
Repository UUID: 12345678-abcd-1234-abcd-1234567890ab
Revision: 1024
Node Kind: directory
Schedule: normal
Last Changed Author: dev
Last Changed Rev: 1020
";
    let branch_or_rev = parse_svn_info_output(trunk_output);
    assert_eq!(branch_or_rev, "trunk:r1024");

    let branch_output = "\
Path: .
Working Copy Root Path: /workspace/svn_repo
URL: https://svn.example.com/svn/project/branches/feature-vcs
Relative URL: ^/branches/feature-vcs
Revision: 42
";
    let branch_or_rev = parse_svn_info_output(branch_output);
    assert_eq!(branch_or_rev, "feature-vcs:r42");

    let rev_only_output = "\
Path: .
Revision: 256
";
    let branch_or_rev = parse_svn_info_output(rev_only_output);
    assert_eq!(branch_or_rev, "r256");
}

#[test]
/// 驗證子檔案的 VCS 變更狀態會依優先級向上匯總至上層目錄。
fn directory_ancestor_status_rollup() {
    let repo_root = PathBuf::from("/workspace/repo");
    let mut statuses = std::collections::HashMap::new();

    let button_path = repo_root.join("src/components/button.rs");
    let test_path = repo_root.join("tests/test_ui.rs");
    let header_path = repo_root.join("src/components/header.rs");

    statuses.insert(button_path.clone(), VcsFileStatus::Modified);
    rollup_status_to_ancestors(
        &mut statuses,
        &button_path,
        &repo_root,
        VcsFileStatus::Modified,
    );

    statuses.insert(test_path.clone(), VcsFileStatus::Added);
    rollup_status_to_ancestors(&mut statuses, &test_path, &repo_root, VcsFileStatus::Added);

    // Conflict 具備更高優先權，應覆蓋 Modified
    statuses.insert(header_path.clone(), VcsFileStatus::Conflict);
    rollup_status_to_ancestors(
        &mut statuses,
        &header_path,
        &repo_root,
        VcsFileStatus::Conflict,
    );

    // /workspace/repo/src/components 包含 Modified 與 Conflict，應取 Conflict
    assert_eq!(
        statuses.get(&repo_root.join("src/components")),
        Some(&VcsFileStatus::Conflict)
    );
    // /workspace/repo/src 應繼承 Conflict
    assert_eq!(
        statuses.get(&repo_root.join("src")),
        Some(&VcsFileStatus::Conflict)
    );
    // /workspace/repo/tests 應繼承 Added
    assert_eq!(
        statuses.get(&repo_root.join("tests")),
        Some(&VcsFileStatus::Added)
    );
}

#[test]
/// 驗證快速讀取 .git/HEAD 分支與 detached commit 雜湊。
fn git_head_branch_reading() {
    let temp = tempdir().expect("tempdir");
    let git_dir = temp.path().join(".git");
    fs::create_dir(&git_dir).expect("create .git");

    // 1. 標準分支 ref: refs/heads/main
    let head_path = git_dir.join("HEAD");
    fs::write(&head_path, "ref: refs/heads/main\n").expect("write HEAD");
    let branch = read_git_branch_from_head(temp.path());
    assert_eq!(branch, Some("main".to_string()));

    // 2. 特徵分支 ref: refs/heads/feature/vcs-support
    fs::write(&head_path, "ref: refs/heads/feature/vcs-support\r\n").expect("write HEAD");
    let branch = read_git_branch_from_head(temp.path());
    assert_eq!(branch, Some("feature/vcs-support".to_string()));

    // 3. Detached HEAD (40-char commit SHA)
    fs::write(&head_path, "4b825dc642cb6eb9a060e54bf8d69288fbee4904\n").expect("write HEAD");
    let branch = read_git_branch_from_head(temp.path());
    assert_eq!(branch, Some("4b825dc".to_string()));
}

#[test]
/// 驗證 VcsFileStatus 的標籤字元、顏色與優先權定義符合規格。
fn vcs_file_status_properties() {
    let theme = Theme::from(ThemePreset::Dracula);

    assert_eq!(VcsFileStatus::Modified.badge_char(), 'M');
    assert_eq!(VcsFileStatus::Added.badge_char(), 'A');
    assert_eq!(VcsFileStatus::Deleted.badge_char(), 'D');
    assert_eq!(VcsFileStatus::Untracked.badge_char(), '?');
    assert_eq!(VcsFileStatus::Conflict.badge_char(), 'C');
    assert_eq!(VcsFileStatus::Renamed.badge_char(), 'R');
    assert_eq!(VcsFileStatus::TypeChanged.badge_char(), '~');
    assert_eq!(VcsFileStatus::Ignored.badge_char(), '!');

    assert_eq!(
        VcsFileStatus::Modified.color(&theme),
        ratatui::style::Color::Yellow
    );
    assert_eq!(
        VcsFileStatus::Added.color(&theme),
        ratatui::style::Color::Green
    );
    assert_eq!(
        VcsFileStatus::Deleted.color(&theme),
        ratatui::style::Color::Red
    );
    assert_eq!(
        VcsFileStatus::Untracked.color(&theme),
        ratatui::style::Color::Cyan
    );
    assert_eq!(
        VcsFileStatus::Conflict.color(&theme),
        ratatui::style::Color::LightRed
    );

    // 優先權順序
    assert!(VcsFileStatus::Conflict.priority() > VcsFileStatus::Modified.priority());
    assert!(VcsFileStatus::Modified.priority() > VcsFileStatus::Added.priority());
    assert!(VcsFileStatus::Added.priority() > VcsFileStatus::Deleted.priority());
    assert!(VcsFileStatus::Deleted.priority() > VcsFileStatus::Untracked.priority());
}

#[test]
/// 驗證 VcsRepoInfo 的 format_header_label 輸出正確的 pane 標題後綴。
fn vcs_repo_info_format_header_label() {
    let git_info = VcsRepoInfo {
        vcs_type: VcsType::Git,
        repo_root: PathBuf::from("/workspace"),
        branch_or_rev: "main".to_string(),
        statuses: std::collections::HashMap::new(),
    };
    assert_eq!(git_info.format_header_label(), "git:main");

    let svn_info = VcsRepoInfo {
        vcs_type: VcsType::Svn,
        repo_root: PathBuf::from("/workspace/svn"),
        branch_or_rev: "trunk:r142".to_string(),
        statuses: std::collections::HashMap::new(),
    };
    assert_eq!(svn_info.format_header_label(), "svn:trunk:r142");
}

#[test]
/// 驗證 VcsManager 背景 Worker 能夠發送與接收非同步查詢結果。
fn vcs_manager_query_and_response() {
    let manager = super::VcsManager::new();
    let temp = tempdir().expect("tempdir");

    // 建立臨時 Git 儲存庫
    let git_dir = temp.path().join(".git");
    fs::create_dir(&git_dir).expect("create .git");
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/develop\n").expect("write HEAD");

    // 發送查詢
    manager.request_query(1, temp.path().to_path_buf());

    // 輪詢等待結果（通常數毫秒完成）
    let start = std::time::Instant::now();
    let mut received = None;
    while start.elapsed() < std::time::Duration::from_secs(3) {
        if let Some(resp) = manager.try_recv_response() {
            received = Some(resp);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let resp = received.expect("必須收到 VcsQueryResponse");
    assert_eq!(resp.pane_id, 1);
    assert_eq!(resp.directory, temp.path());
    let info = resp.info.expect("必須成功偵測到 Git repo");
    assert_eq!(info.vcs_type, VcsType::Git);
    assert_eq!(info.branch_or_rev, "develop");
}

#[test]
/// 驗證 `is_valid_git_dir` 能精準識別空 .git 目錄與真正的 Git 儲存庫。
/// 保護目的：避免子目錄中殘留空 .git 資料夾時誤將其判定為 Git，遮蔽上層有效的 SVN 儲存庫。
fn is_valid_git_dir_differentiates_real_and_empty_dirs() {
    let temp = tempdir().expect("tempdir");

    // 1. 空目錄名為 .git（例如中斷的 git init 或殘留檔案夾） -> 必須為 false
    let empty_git = temp.path().join("empty_repo");
    fs::create_dir_all(empty_git.join(".git")).expect("create empty .git");
    assert!(!is_valid_git_dir(&empty_git));

    // 2. 包含 HEAD 的合法 .git 目錄 -> 必須為 true
    let valid_git = temp.path().join("valid_repo");
    let valid_git_dir = valid_git.join(".git");
    fs::create_dir_all(&valid_git_dir).expect("create valid .git");
    fs::write(valid_git_dir.join("HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
    assert!(is_valid_git_dir(&valid_git));

    // 3. 包含 config 的合法 .git 目錄 -> 必須為 true
    let config_git = temp.path().join("config_repo");
    let config_git_dir = config_git.join(".git");
    fs::create_dir_all(&config_git_dir).expect("create config .git");
    fs::write(config_git_dir.join("config"), "[core]\n").expect("write config");
    assert!(is_valid_git_dir(&config_git));

    // 4. Submodule .git 檔案 (gitdir: ...) -> 必須為 true
    let sub_git = temp.path().join("submodule_repo");
    fs::create_dir_all(&sub_git).expect("create sub dir");
    fs::write(sub_git.join(".git"), "gitdir: ../.git/modules/sub\n").expect("write gitdir");
    assert!(is_valid_git_dir(&sub_git));

    // 5. 非 gitdir 的無效 .git 檔案 -> 必須為 false
    let invalid_sub = temp.path().join("invalid_sub");
    fs::create_dir_all(&invalid_sub).expect("create invalid sub");
    fs::write(invalid_sub.join(".git"), "random garbage\n").expect("write garbage");
    assert!(!is_valid_git_dir(&invalid_sub));
}

#[test]
/// 驗證當子目錄包含空 .git 時，`find_vcs_candidates` 能略過它並成功識別上層的 SVN 儲存庫。
/// 這是本次修復的核心場景（OnlineCasino monorepo 下 PlatformClient 殘留空 .git）。
fn find_vcs_skips_empty_git_and_finds_parent_svn() {
    let temp = tempdir().expect("tempdir");

    // 頂層目錄為 SVN
    let svn_root = temp.path().join("MonorepoRoot");
    fs::create_dir_all(svn_root.join(".svn")).expect("create .svn");

    // 多層子目錄，其中一層有殘留的空 .git
    let client_dir = svn_root
        .join("hot168")
        .join("Client")
        .join("PlatformClient");
    fs::create_dir_all(client_dir.join(".git")).expect("create empty .git");

    // 最深層的資產目錄
    let target_dir = client_dir.join("src").join("assets").join("animations");
    fs::create_dir_all(&target_dir).expect("create target dir");

    // 查詢候選清單
    let candidates = find_vcs_candidates(&target_dir);
    assert_eq!(candidates.len(), 1, "空 .git 必須被忽略，只保留頂層 SVN");
    assert_eq!(candidates[0].0, VcsType::Svn);
    assert_eq!(candidates[0].1, svn_root);

    // find_vcs_root 必須直接回傳頂層 SVN
    let root = find_vcs_root(&target_dir);
    assert_eq!(root, Some((VcsType::Svn, svn_root)));
}

#[test]
/// 驗證 Windows 路徑分隔符標準化，確保即使 Git 輸出 `/` 而 Windows 列表使用 `\` 也能正確查得狀態。
fn path_normalization_and_status_lookup_consistency() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().to_path_buf();

    #[cfg(windows)]
    assert_eq!(
        normalize_path(std::path::Path::new("a/b/c")),
        PathBuf::from("a\\b\\c")
    );

    // 模擬 Git status 輸出使用 forward slash `/`
    let git_output = " M src/assets/icon.png\nA  src/utils/math.rs\n";
    let statuses = parse_git_status_output(git_output, &repo_root);

    let info = VcsRepoInfo {
        vcs_type: VcsType::Git,
        repo_root: repo_root.clone(),
        branch_or_rev: "main".to_string(),
        statuses,
    };

    // 使用系統原生路徑進行查詢
    let native_icon_path = repo_root.join("src").join("assets").join("icon.png");
    assert_eq!(
        info.status_for_path(&native_icon_path),
        Some(VcsFileStatus::Modified),
        "原生路徑查詢必須能查到由 Git porcelain 輸出的狀態"
    );

    let native_math_path = repo_root.join("src").join("utils").join("math.rs");
    assert_eq!(
        info.status_for_path(&native_math_path),
        Some(VcsFileStatus::Added)
    );

    // 父資料夾的 rollup 狀態也必須能以原生路徑查得
    let parent_dir = repo_root.join("src").join("assets");
    assert_eq!(
        info.status_for_path(&parent_dir),
        Some(VcsFileStatus::Modified)
    );
}

#[test]
/// 驗證 Unified Diff 輸出能被正確解析並套用語法顏色高亮（+ 綠色、- 紅色、@@ 青色粗體、+++/--- 黃色粗體）。
fn format_diff_lines_highlighting_colors() {
    use ratatui::style::Color;
    let theme = Theme::default_theme();
    let sample_diff = concat!(
        "--- a/src/main.rs\n",
        "+++ b/src/main.rs\n",
        "@@ -10,4 +10,5 @@\n",
        " let old_var = 1;\n",
        "-let removed = 2;\n",
        "+let added = 3;\n",
    );
    let lines = format_diff_lines(sample_diff, &theme);
    assert_eq!(lines.len(), 6);

    // --- Yellow
    assert_eq!(lines[0].spans[0].style.fg, Some(Color::Yellow));
    // +++ Yellow
    assert_eq!(lines[1].spans[0].style.fg, Some(Color::Yellow));
    // @@ Cyan
    assert_eq!(lines[2].spans[0].style.fg, Some(Color::Cyan));
    // Context muted
    assert_eq!(lines[3].spans[0].style.fg, Some(theme.muted));
    // - Red
    assert_eq!(lines[4].spans[0].style.fg, Some(Color::Red));
    // + Green
    assert_eq!(lines[5].spans[0].style.fg, Some(Color::Green));
}

#[test]
/// 驗證當工作目錄同時存在有效的 SVN 與 Git 時，PaneFM 嚴格優先使用 SVN。
/// 保護目的：避免使用者在主要由 SVN 管理之團隊專案中，被本地暫存之 Git 倉庫覆蓋狀態。
fn find_vcs_prioritizes_svn_over_git_when_coexisting() {
    let temp = tempdir().expect("tempdir");
    let project_dir = temp.path().join("PlatformClient");

    // 建立有效的 SVN 工作副本標記
    fs::create_dir_all(project_dir.join(".svn")).expect("create .svn");

    // 同時建立有效的 Git 倉庫標記（包含 HEAD）
    let git_dir = project_dir.join(".git");
    fs::create_dir_all(&git_dir).expect("create .git");
    fs::write(git_dir.join("HEAD"), "ref: refs/heads/master\n").expect("write git HEAD");

    let candidates = find_vcs_candidates(&project_dir);
    assert!(candidates.len() >= 2, "必須同時偵測到 SVN 與 Git 候選");
    assert_eq!(candidates[0].0, VcsType::Svn, "第 0 順位必須優先採用 SVN");
    assert_eq!(candidates[1].0, VcsType::Git, "第 1 順位作為次要 Git 回退");
    assert_eq!(
        find_vcs_root(&project_dir),
        Some((VcsType::Svn, project_dir.clone()))
    );
}

#[test]
/// 驗證 SVN status 能將深層子目錄之檔案變更向上 rollup 至所有父資料夾（如 BusinessCard/Referral.ts 使 BusinessCard/ 獲得 M 標記）。
/// 保護目的：確保使用者在外部檢視父目錄清單時，能清晰掌握內部子目錄有尚未提交之修改。
fn parse_svn_status_rolls_up_nested_subdirectories() {
    let repo_root = PathBuf::from("R:\\PlatformClient\\src\\MainProgram\\Scene\\Lobby\\Module");
    let svn_output = "\
M       BusinessCard/Referral.ts
A       Assets/Sounds/bgm.mp3
";
    let statuses = parse_svn_status_output(svn_output, &repo_root);

    let info = VcsRepoInfo {
        vcs_type: VcsType::Svn,
        repo_root: repo_root.clone(),
        branch_or_rev: "svn".to_string(),
        statuses,
    };

    // 1. 本身修改的檔案必須標記為 Modified
    let referral_file = repo_root.join("BusinessCard").join("Referral.ts");
    assert_eq!(
        info.status_for_path(&referral_file),
        Some(VcsFileStatus::Modified)
    );

    // 2. 其父目錄 BusinessCard/ 必須被 rollup 標記為 Modified（出現 M 符號）
    let business_card_dir = repo_root.join("BusinessCard");
    assert_eq!(
        info.status_for_path(&business_card_dir),
        Some(VcsFileStatus::Modified),
        "父目錄 BusinessCard/ 必須被自動標記為 Modified (M)"
    );

    // 3. 多層新增的檔案與中間所有父目錄階層皆必須被 rollup
    let bgm_file = repo_root.join("Assets").join("Sounds").join("bgm.mp3");
    let sounds_dir = repo_root.join("Assets").join("Sounds");
    let assets_dir = repo_root.join("Assets");
    assert_eq!(info.status_for_path(&bgm_file), Some(VcsFileStatus::Added));
    assert_eq!(
        info.status_for_path(&sounds_dir),
        Some(VcsFileStatus::Added)
    );
    assert_eq!(
        info.status_for_path(&assets_dir),
        Some(VcsFileStatus::Added),
        "最外層父目錄 Assets/ 也必須被 rollup 標記為 Added (A)"
    );
}
