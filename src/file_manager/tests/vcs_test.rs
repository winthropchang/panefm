use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

use super::{
    VcsFileStatus, VcsRepoInfo, VcsType, parse_git_status_output, parse_svn_info_output,
    parse_svn_status_output, read_git_branch_from_head, rollup_status_to_ancestors,
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
