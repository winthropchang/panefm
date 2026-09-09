use tempfile::tempdir;

use super::*;

#[test]
/// 驗證不存在的 `bookmark.toml` 會被視為空集合，而不是報錯。
/// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
fn load_missing_bookmark_file_returns_empty_store() {
    let dir = tempdir().expect("tempdir");
    let store = BookmarkStore::load(dir.path().join("bookmark.toml")).expect("load store");

    assert!(store.list().is_empty());
}

#[test]
/// 驗證設定書籤後會立刻寫回 `bookmark.toml`，之後重新載入仍能讀回相同內容。
/// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
fn set_bookmark_persists_to_file() {
    let dir = tempdir().expect("tempdir");
    let file = dir.path().join("bookmark.toml");

    let mut store = BookmarkStore::load(file.clone()).expect("load");
    store
        .set('a', PathBuf::from("/tmp/demo"))
        .expect("save bookmark");

    let reloaded = BookmarkStore::load(file).expect("reload");
    assert_eq!(
        reloaded.get('a'),
        Some(&BookmarkTarget::LocalPath(PathBuf::from("/tmp/demo")))
    );
}

#[test]
/// 驗證 SMB 書籤會以原始 `smb://...` 字串寫回檔案，重新載入後仍能辨識成 SMB 目標。
/// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
fn set_smb_bookmark_persists_to_file() {
    let dir = tempdir().expect("tempdir");
    let file = dir.path().join("bookmark.toml");

    let mut store = BookmarkStore::load(file.clone()).expect("load");
    store
        .set_smb('s', String::from("smb://192.0.2.10/shared/docs"))
        .expect("save smb bookmark");

    let reloaded = BookmarkStore::load(file.clone()).expect("reload");
    assert_eq!(
        reloaded.get('s'),
        Some(&BookmarkTarget::SmbLocation(String::from(
            "smb://192.0.2.10/shared/docs"
        )))
    );
    assert!(
        fs::read_to_string(file)
            .expect("bookmark file")
            .contains("smb://192.0.2.10/shared/docs")
    );
}

#[test]
/// 驗證 SMB 中文路徑在 UI 會顯示解碼後文字，但持久化值仍保持原始 percent-encoded URI。
/// 保護目的：避免為了改善書籤可讀性，意外破壞 `bookmark.toml` 或實際 SMB 跳轉所需的 URI。
fn smb_bookmark_decodes_only_its_display_text() {
    let encoded = "smb://192.0.2.10/shared/%E7%B6%B2%E8%B7%AF%E4%BA%8B%E6%A5%AD%E9%83%A8/otto";
    let target = BookmarkTarget::SmbLocation(encoded.to_string());

    assert_eq!(
        target.display_text(),
        "smb://192.0.2.10/shared/網路事業部/otto"
    );
    assert_eq!(target.as_storage_value(), encoded);
}

#[test]
/// 驗證不合法的 UTF-8 percent encoding 不會在 UI 中被替代字元悄悄改寫。
/// 保護目的：遇到非 UTF-8 SMB 名稱時仍顯示可供除錯的原始 URI，並確保連線資料不受影響。
fn smb_bookmark_display_falls_back_for_invalid_utf8() {
    let encoded = "smb://192.0.2.10/shared/%FF";
    let target = BookmarkTarget::SmbLocation(encoded.to_string());

    assert_eq!(target.display_text(), encoded);
    assert_eq!(target.as_storage_value(), encoded);
}

#[test]
/// 驗證書籤檔會放在 `config.toml` 同一個目錄，若沒有設定檔則退回工作目錄旁邊。
/// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
fn bookmark_file_path_prefers_config_directory() {
    let base = Path::new("/workspace/project");
    let config = Path::new("/workspace/settings/config.toml");

    assert_eq!(
        bookmark_file_path(base, Some(config)),
        PathBuf::from("/workspace/settings/bookmark.toml")
    );
    assert_eq!(
        bookmark_file_path(base, None),
        PathBuf::from("/workspace/project/bookmark.toml")
    );
}

#[test]
/// 驗證自動分配書籤代號時，會挑出目前尚未使用的第一個預設按鍵。
/// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
fn next_available_key_uses_first_free_preferred_key() {
    let dir = tempdir().expect("tempdir");
    let mut store = BookmarkStore::load(dir.path().join("bookmark.toml")).expect("load");
    store
        .set('a', PathBuf::from("/tmp/a"))
        .expect("save a bookmark");
    store
        .set('b', PathBuf::from("/tmp/b"))
        .expect("save b bookmark");

    assert_eq!(store.next_available_key(), Some('c'));
}

#[test]
/// 驗證刪除單一書籤後，記憶體與檔案都會同步更新。
/// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
fn remove_bookmark_persists_to_file() {
    let dir = tempdir().expect("tempdir");
    let file = dir.path().join("bookmark.toml");

    let mut store = BookmarkStore::load(file.clone()).expect("load");
    store
        .set('a', PathBuf::from("/tmp/demo"))
        .expect("save bookmark");

    assert!(store.remove('a').expect("remove bookmark"));
    assert_eq!(store.get('a'), None);
    assert!(
        !fs::read_to_string(file)
            .expect("bookmark file")
            .contains("/tmp/demo")
    );
}

#[test]
/// 驗證清空全部書籤後，列表會變成空集合。
/// 保護目的：避免書籤格式與持久化流程調整後，造成既有 bookmark.toml 資料遺失或無法跳轉。
fn clear_bookmarks_removes_all_entries() {
    let dir = tempdir().expect("tempdir");
    let mut store = BookmarkStore::load(dir.path().join("bookmark.toml")).expect("load");
    store
        .set('a', PathBuf::from("/tmp/demo"))
        .expect("save bookmark");
    store
        .set('b', PathBuf::from("/tmp/demo2"))
        .expect("save bookmark");

    store.clear().expect("clear bookmarks");

    assert!(store.list().is_empty());
    assert_eq!(store.next_available_key(), Some('a'));
}

#[test]
/// 驗證在 bookmark.toml 設定 UNC 格式時，能自動轉化並識別為 SMB 目標。
fn unc_bookmark_parses_to_smb_location() {
    let target = parse_bookmark_target("//192.0.2.10/shared/docs").expect("parse unc");
    assert_eq!(
        target,
        BookmarkTarget::SmbLocation(String::from("smb://192.0.2.10/shared/docs"))
    );

    let backslash_target =
        parse_bookmark_target(r"\\192.0.2.10\shared\docs").expect("parse backslash");
    assert_eq!(
        backslash_target,
        BookmarkTarget::SmbLocation(String::from("smb://192.0.2.10/shared/docs"))
    );
}
