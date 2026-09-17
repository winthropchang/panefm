//! 書籤建立、單鍵跳轉、清單瀏覽、刪除、清空與 Zoxide 跳轉。

use std::io;
use std::path::{Path, PathBuf};

use super::super::*;

impl App {
    /// 將指定字元代號綁定為書籤，並儲存目前焦點 pane 的位置。
    pub(crate) fn set_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let target = pane.bookmark_target.clone();
        match &target {
            BookmarkTarget::LocalPath(path) => {
                self.bookmark_store
                    .set(key, path.clone())
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.status = format!("bookmark [{key}] = {}", path.display());
            }
            BookmarkTarget::SmbLocation(location) => {
                self.bookmark_store
                    .set_smb(key, location.clone())
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.status = format!("bookmark [{key}] = {}", target.display_text());
            }
        }
        Ok(())
    }

    /// 自動挑選下一個可用書籤代號，並把指定 pane 目前位置存成書籤。
    ///
    /// 參數：
    /// - `pane_id: usize`，要儲存位置的 pane 編號。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn add_bookmark_with_auto_key(&mut self, pane_id: usize) -> io::Result<()> {
        let Some(key) = self.bookmark_store.next_available_key() else {
            self.status = String::from("bookmark: no available auto key");
            return Ok(());
        };

        self.focused_pane = pane_id;
        self.set_bookmark(key)
    }

    /// 跳到指定書籤對應的路徑。
    pub(crate) fn jump_to_bookmark(&mut self, key: char) -> io::Result<()> {
        let Some(target) = self.bookmark_store.get(key).cloned() else {
            self.status = format!("bookmark [{key}] not found");
            return Ok(());
        };

        self.jump_to_bookmark_target(self.focused_pane, key, &target)
    }

    /// 讓 `:bookmark jump <key>` 可以直接跳到指定書籤。
    pub(crate) fn jump_to_bookmark_from_command(&mut self, args: &str) -> io::Result<()> {
        let Some(key) = parse_bookmark_argument(args) else {
            self.status = String::from("usage: bookmark jump <key>");
            return Ok(());
        };
        self.jump_to_bookmark(key)
    }

    /// 刪除指定代號的單一書籤，並同步更新狀態列。
    ///
    /// 參數：
    /// - `key: char`，要刪除的書籤代號。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn delete_bookmark(&mut self, key: char) -> io::Result<()> {
        let removed = self
            .bookmark_store
            .remove(key)
            .map_err(|error| io::Error::other(error.to_string()))?;

        if removed {
            self.status = format!("bookmark [{key}] deleted");
        } else {
            self.status = format!("bookmark [{key}] not found");
        }
        Ok(())
    }

    /// 根據目前書籤列表選取位置刪除單一書籤。
    ///
    /// 參數：
    /// - `entries: &[BookmarkEntry]`，目前列表中可見的書籤資料。
    /// - `selected: usize`，目前游標指到的列索引。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn delete_bookmark_from_list(
        &mut self,
        entries: &[BookmarkEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("bookmark delete: empty");
            return Ok(());
        };

        self.delete_bookmark(entry.key)
    }

    /// 清空全部書籤，並同步寫回 `bookmark.toml`。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn delete_all_bookmarks(&mut self) -> io::Result<()> {
        self.bookmark_store
            .clear()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.status = String::from("all bookmarks deleted");
        Ok(())
    }

    /// 依書籤目標型別跳到本機目錄，或自動發起 SMB 掛載／進入流程。
    pub(crate) fn jump_to_bookmark_target(
        &mut self,
        pane_id: usize,
        key: char,
        target: &BookmarkTarget,
    ) -> io::Result<()> {
        self.jump_to_bookmark_target_with_mount_root(pane_id, key, target, Path::new("/Volumes"))
    }

    /// 依書籤目標型別跳到本機目錄，或用指定掛載根目錄自動發起 SMB 掛載／進入流程。
    pub(crate) fn jump_to_bookmark_target_with_mount_root(
        &mut self,
        pane_id: usize,
        key: char,
        target: &BookmarkTarget,
        mount_root: &Path,
    ) -> io::Result<()> {
        self.focused_pane = pane_id;

        match target {
            BookmarkTarget::LocalPath(path) => {
                if !self.panes.contains_key(&pane_id) {
                    self.status = String::from("panel no longer exists");
                    return Ok(());
                }
                if !path.exists() {
                    self.status = format!("bookmark [{key}] missing: {}", path.display());
                    return Ok(());
                }
                self.go_to_path_and_track(pane_id, path)?;
                self.status = format!("jumped to bookmark [{key}]");
            }
            BookmarkTarget::SmbLocation(location) => {
                self.goto_smb_location_with_mount_root(location, mount_root)?;
                if self.status.starts_with("jumped to smb:") {
                    self.status = format!("jumped to bookmark [{key}]");
                } else if self.status.starts_with("已請求系統掛載 SMB：") {
                    self.status = format!("bookmark [{key}] 正在連線：{location}");
                }
            }
        }

        Ok(())
    }

    /// 從書籤列表彈窗中打開目前選取的書籤。
    pub(crate) fn open_bookmark_from_list(
        &mut self,
        pane_id: usize,
        entries: &[BookmarkEntry],
        selected: usize,
    ) -> io::Result<()> {
        let Some(entry) = entries.get(selected) else {
            self.status = String::from("bookmark jump: empty");
            return Ok(());
        };
        let Some(target) = self.bookmark_store.get(entry.key).cloned() else {
            self.status = format!("bookmark [{}] not found", entry.key);
            return Ok(());
        };
        self.jump_to_bookmark_target(pane_id, entry.key, &target)
    }

    /// 從 zoxide 列表中打開目前選取的目錄。
    pub(crate) fn open_zoxide_from_list(
        &mut self,
        pane_id: usize,
        entries: &[PathBuf],
        selected: usize,
    ) -> io::Result<()> {
        let Some(target_path) = entries.get(selected).cloned() else {
            self.status = String::from("zoxide: empty");
            return Ok(());
        };

        self.go_to_path_and_track(pane_id, &target_path)?;
        self.focused_pane = pane_id;
        self.status = format!("jumped via zoxide: {}", target_path.display());
        Ok(())
    }
}
