use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::tempdir;

use super::{
    ResolvedSmbLocation, decode_mount_field, find_macos_smb_mount, find_macos_smb_mount_by_share,
    parse_smb_location, resolve_smb_location_with_mount_root,
};

#[test]
/// 驗證 SMB URL 會拆成 host、share、subpath 並解碼百分比字元。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn parse_smb_location_extracts_share_and_subpath() {
    let location = parse_smb_location("smb://192.0.2.10/shared/docs/report%20v1").expect("parse");

    assert_eq!(location.host, "192.0.2.10");
    assert_eq!(location.share, "shared");
    assert_eq!(location.subpath, Path::new("docs").join("report v1"));
}

#[test]
/// 驗證已存在掛載根目錄時直接回傳可瀏覽路徑，不再要求系統掛載。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn resolve_smb_location_with_mount_root_reports_ready_when_share_exists() {
    let dir = tempdir().expect("tempdir");
    let share_root = dir.path().join("shared");
    fs::create_dir(&share_root).expect("share");
    fs::create_dir(share_root.join("docs")).expect("docs");

    let location = parse_smb_location("smb://server/shared/docs").expect("parse");
    let resolved = resolve_smb_location_with_mount_root(&location, dir.path());

    assert_eq!(
        resolved,
        ResolvedSmbLocation::Ready(share_root.join("docs"))
    );
}

#[test]
/// 驗證 share 尚未掛載時回傳 NeedsMount，讓 App 顯示明確連線流程。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn resolve_smb_location_with_mount_root_reports_needs_mount_when_missing() {
    let dir = tempdir().expect("tempdir");
    let location = parse_smb_location("smb://server/shared/docs").expect("parse");
    let resolved = resolve_smb_location_with_mount_root(&location, dir.path());

    assert_eq!(
        resolved,
        ResolvedSmbLocation::NeedsMount {
            local_path: dir.path().join("shared").join("docs")
        }
    );
}

#[test]
/// 驗證缺少 share 名稱的 SMB URL 會被拒絕，避免跳到不明確的 host 根目錄。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn parse_smb_location_requires_share_name() {
    let error = parse_smb_location("smb://192.0.2.10").expect_err("missing share");

    assert_eq!(
        error.to_string(),
        "SMB 位址格式錯誤：請使用 goto smb://host/share[/path]，不能只有 IP 或主機名稱"
    );
}

#[test]
/// 驗證 macOS 有同名 share 時，會依 host 選擇真正對應的 `-1` 掛載點。
///
/// 參數：無。
/// 回傳：無；若只依 share 名稱誤選其他伺服器的掛載點則測試失敗。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn macos_mount_parser_matches_host_and_share() {
    let output = "//otto@old-server/shared on /Volumes/shared (smbfs, nodev)\n\
                      //domain;otto@192.0.2.10/shared on /Volumes/shared-1 (smbfs, nodev)\n";

    assert_eq!(
        find_macos_smb_mount(output, "192.0.2.10", "shared"),
        Some(PathBuf::from("/Volumes/shared-1"))
    );
}

#[test]
/// 驗證 mount table 的空白跳脫可以還原，避免含空白的掛載目錄無法進入。
///
/// 參數：無。
/// 回傳：無；若 `\040` 沒有還原成空白則測試失敗。
/// 保護目的：避免跨平台命令與路徑處理調整後，只在 macOS 或 Windows 其中一端失效。
fn mount_field_decoder_restores_octal_escapes() {
    assert_eq!(
        decode_mount_field("/Volumes/Company\\040Share"),
        "/Volumes/Company Share"
    );
}

#[test]
/// 驗證 UNC 正斜線格式 `//host/share[/path]` 能正確解析並轉為標準 `smb://` URL。
fn parse_smb_location_extracts_unc_forward_slashes() {
    let root = parse_smb_location("//192.168.0.141/mingfong/").expect("parse root");
    assert_eq!(root.host, "192.168.0.141");
    assert_eq!(root.share, "mingfong");
    assert_eq!(root.subpath, PathBuf::new());
    assert_eq!(root.url, "smb://192.168.0.141/mingfong");

    let nested =
        parse_smb_location("//192.168.0.141/mingfong/docs/report.txt").expect("parse nested");
    assert_eq!(nested.host, "192.168.0.141");
    assert_eq!(nested.share, "mingfong");
    assert_eq!(nested.subpath, Path::new("docs").join("report.txt"));
    assert_eq!(nested.url, "smb://192.168.0.141/mingfong/docs/report.txt");
}

#[test]
/// 驗證 Windows UNC 反斜線格式 `\\host\share\path` 能正確解析並轉為標準 `smb://` URL。
fn parse_smb_location_extracts_unc_backslashes() {
    let location = parse_smb_location(r"\\192.168.0.141\mingfong\docs\report.txt").expect("parse");
    assert_eq!(location.host, "192.168.0.141");
    assert_eq!(location.share, "mingfong");
    assert_eq!(location.subpath, Path::new("docs").join("report.txt"));
    assert_eq!(location.url, "smb://192.168.0.141/mingfong/docs/report.txt");
}

#[test]
/// 驗證缺少 share 名稱的 UNC 路徑會被拒絕。
fn parse_smb_location_requires_share_name_for_unc() {
    let error = parse_smb_location("//192.168.0.141").expect_err("missing share");
    assert_eq!(
        error.to_string(),
        "SMB 位址格式錯誤：請使用 goto smb://host/share[/path]，不能只有 IP 或主機名稱"
    );
}

#[test]
/// 驗證 macOS mount table 中 host 帶有 port (如 :445) 時依然能正確比對。
fn macos_mount_parser_matches_host_with_port() {
    let output = "//otto@192.168.0.141:445/mingfong on /Volumes/mingfong (smbfs, nodev)\n";
    assert_eq!(
        find_macos_smb_mount(output, "192.168.0.141", "mingfong"),
        Some(PathBuf::from("/Volumes/mingfong"))
    );
}

#[test]
/// 驗證當 host 名稱不同 (如 hostname vs IP) 時，能 fallback 依 share 名稱找出 smbfs 掛載點。
fn macos_mount_parser_matches_by_share_fallback() {
    let output = "//otto@mingfong-nas.local/mingfong on /Volumes/mingfong (smbfs, nodev)\n";
    assert_eq!(
        find_macos_smb_mount_by_share(output, "mingfong"),
        Some(PathBuf::from("/Volumes/mingfong"))
    );
}
