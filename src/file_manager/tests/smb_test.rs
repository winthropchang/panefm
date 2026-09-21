use std::{
    fs,
    path::{Path, PathBuf},
};

use tempfile::tempdir;

use super::{
    ResolvedSmbLocation, build_smb_mount_launch, clean_host, decode_mount_field, decode_share_name,
    find_macos_smb_mount, find_macos_smb_mount_by_share, find_matching_volume, find_smbutil_mount,
    parse_smb_location, parse_smbutil_statshares, resolve_smb_location_with_mount_root,
    smb_share_root_url,
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

#[test]
/// 驗證 `clean_host` 能正確剝除使用者帳號、網域名稱、Port 與 IPv6 中括號。
fn clean_host_handles_credentials_ports_and_ipv6() {
    assert_eq!(clean_host("192.168.0.141"), "192.168.0.141");
    assert_eq!(clean_host("otto@192.168.0.141"), "192.168.0.141");
    assert_eq!(clean_host("domain;otto@192.168.0.141:445"), "192.168.0.141");
    assert_eq!(clean_host("nas.local:445"), "nas.local");
    assert_eq!(clean_host("[fe80::1]:445"), "fe80::1");
    assert_eq!(clean_host("[fe80::1]"), "fe80::1");
}

#[test]
/// 驗證 `parse_smb_location` 能支援雙引號、單引號以及大小寫不敏感的前綴。
fn parse_smb_location_handles_quotes_and_case() {
    let double_quoted =
        parse_smb_location(r#""smb://192.168.0.141/shared/folder""#).expect("double quoted");
    assert_eq!(double_quoted.host, "192.168.0.141");
    assert_eq!(double_quoted.share, "shared");
    assert_eq!(double_quoted.subpath, PathBuf::from("folder"));

    let single_quoted =
        parse_smb_location("'smb://192.168.0.141/shared/folder'").expect("single quoted");
    assert_eq!(single_quoted.host, "192.168.0.141");
    assert_eq!(single_quoted.share, "shared");

    let upper_case = parse_smb_location("SMB://192.168.0.141/shared").expect("upper case");
    assert_eq!(upper_case.host, "192.168.0.141");
    assert_eq!(upper_case.share, "shared");
}

#[test]
/// 驗證 `parse_smb_location` 能將 host 中的使用者憑證剝離，但保持原始 URL 完整。
fn parse_smb_location_cleans_host_credentials() {
    let location =
        parse_smb_location("smb://domain;otto:pass@192.168.0.141/shared/sub").expect("parse");
    assert_eq!(location.host, "192.168.0.141");
    assert_eq!(location.share, "shared");
    assert_eq!(location.subpath, PathBuf::from("sub"));
    assert_eq!(
        location.url,
        "smb://domain;otto:pass@192.168.0.141/shared/sub"
    );
}

#[test]
/// 驗證遠端路徑結尾帶有斜線時，mount parser 仍能精準比對。
fn macos_mount_parser_matches_with_credentials_and_trailing_slash() {
    let output = "//otto@nas-server/shared/ on /Volumes/shared (smbfs, nodev)\n";
    assert_eq!(
        find_macos_smb_mount(output, "nas-server", "shared"),
        Some(PathBuf::from("/Volumes/shared"))
    );
    assert_eq!(
        find_macos_smb_mount(output, "user@nas-server:445", "shared/"),
        Some(PathBuf::from("/Volumes/shared"))
    );
    assert_eq!(
        find_macos_smb_mount_by_share(output, "shared/"),
        Some(PathBuf::from("/Volumes/shared"))
    );
}

#[test]
/// 驗證卷冊名稱包含括號時（如 `/Volumes/Data (Share)`），不會因括號剖析截斷。
fn macos_mount_parser_handles_parentheses_in_volume_name() {
    let output = "//otto@nas-server/shared on /Volumes/Data (Share) (smbfs, nodev)\n";
    assert_eq!(
        find_macos_smb_mount(output, "nas-server", "shared"),
        Some(PathBuf::from("/Volumes/Data (Share)"))
    );
    assert_eq!(
        find_macos_smb_mount_by_share(output, "shared"),
        Some(PathBuf::from("/Volumes/Data (Share)"))
    );
}

#[test]
/// 驗證 `decode_share_name` 同時支援 octal escapes 與 percent encoding。
fn decode_share_name_handles_octal_and_percent() {
    assert_eq!(decode_share_name("My%20Share"), "My Share");
    assert_eq!(decode_share_name("My\\040Share"), "My Share");
    assert_eq!(decode_share_name("normal_share"), "normal_share");
}

#[test]
/// 驗證 `find_matching_volume` 能動態辨識精確名稱與 `-1` 等後綴卷冊。
fn find_matching_volume_detects_exact_and_suffixed_volumes() {
    let dir = tempdir().expect("tempdir");
    let suffixed = dir.path().join("shared-1");
    fs::create_dir(&suffixed).expect("create suffixed");

    assert_eq!(
        find_matching_volume(dir.path(), "shared"),
        Some(suffixed.clone())
    );
    // 大小寫不敏感
    assert_eq!(find_matching_volume(dir.path(), "SHARED"), Some(suffixed));
}

#[test]
/// 驗證當精確目錄存在但為空（macOS 殘留陳舊目錄），而後綴目錄有內容時，優先選擇後綴目錄。
fn find_matching_volume_prefers_active_suffixed_when_exact_is_empty() {
    let dir = tempdir().expect("tempdir");
    let empty_exact = dir.path().join("shared");
    fs::create_dir(&empty_exact).expect("empty exact");

    let active_suffixed = dir.path().join("shared-1");
    fs::create_dir(&active_suffixed).expect("active suffixed");
    fs::write(active_suffixed.join("test.txt"), "content").expect("file");

    assert_eq!(
        find_matching_volume(dir.path(), "shared"),
        Some(active_suffixed)
    );
}

#[test]
/// 驗證 `parse_smbutil_statshares` 與 `find_smbutil_mount` 正確解析活躍 SMB 卷冊。
fn parse_smbutil_statshares_and_find_smbutil_mount_works() {
    let output = "==================================================================================================\n\
                  SHARE                         ATTRIBUTE TYPE                VALUE\n\
                  ==================================================================================================\n\
                  mingfong                      SERVER_NAME                   192.168.0.141\n\
                                                USER_ID                       501\n\
                                                SMB_NEGOTIATE                 SMBV_NEG_SMB2_ENABLE\n\
                  --------------------------------------------------------------------------------------------------\n";

    let shares = parse_smbutil_statshares(output);
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].0, "mingfong");
    assert_eq!(shares[0].1, "192.168.0.141");

    let dir = tempdir().expect("tempdir");
    let volume_path = dir.path().join("mingfong");
    fs::create_dir(&volume_path).expect("create volume");

    // 主機與 share 相符
    assert_eq!(
        find_smbutil_mount(output, "192.168.0.141", "mingfong", dir.path()),
        Some(volume_path.clone())
    );

    // 主機名稱不符（IP vs Hostname），仍可依 share 找到
    assert_eq!(
        find_smbutil_mount(output, "nas-server.local", "mingfong", dir.path()),
        Some(volume_path)
    );
}

#[test]
/// 驗證 `resolve_smb_location_with_mount_root` 能直接定位至 `-1` 等後綴的現存掛載點。
fn resolve_smb_location_with_mount_root_finds_suffixed_mount() {
    let dir = tempdir().expect("tempdir");
    let suffixed_root = dir.path().join("shared-1");
    fs::create_dir(&suffixed_root).expect("share-1");
    fs::create_dir(suffixed_root.join("docs")).expect("docs");

    let location = parse_smb_location("smb://server/shared/docs").expect("parse");
    let resolved = resolve_smb_location_with_mount_root(&location, dir.path());

    assert_eq!(
        resolved,
        ResolvedSmbLocation::Ready(suffixed_root.join("docs"))
    );
}

#[test]
/// 驗證 `build_smb_mount_launch` 與 `smb_share_root_url` 僅針對 share 根目錄發起掛載請求，
/// 不將深層子路徑傳給系統 open 命令，以保留完整目錄樹架構。
fn smb_mount_launch_targets_share_root_without_subpath() {
    let location = parse_smb_location("smb://192.168.0.141/mingfong/網路事業部/otto").expect("parse");
    let root_url = smb_share_root_url(&location);
    assert_eq!(root_url, "smb://192.168.0.141/mingfong");

    let launch = build_smb_mount_launch(&location);
    #[cfg(target_os = "windows")]
    {
        assert_eq!(launch.program, "explorer.exe");
        assert_eq!(launch.args, vec![r"\\192.168.0.141\mingfong"]);
    }
    #[cfg(target_os = "macos")]
    {
        assert_eq!(launch.program, "open");
        assert_eq!(launch.args, vec!["smb://192.168.0.141/mingfong"]);
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        assert_eq!(launch.program, "xdg-open");
        assert_eq!(launch.args, vec!["smb://192.168.0.141/mingfong"]);
    }
}

