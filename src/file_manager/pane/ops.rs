use std::{
    fs, io,
    path::{Path, PathBuf},
};

use super::{
    PaneState,
    copy::{
        commit_staged_copy, copy_file_and_verify, copy_file_native_with_progress,
        remove_transfer_path, unique_transfer_path,
    },
    create::{target_path_file_name, unique_target_path},
    tree::{copy_dir_recursive, copy_dir_recursive_with_progress},
    types::{PasteOutcome, TransferProgress},
};
use crate::file_manager::undo_backup::create_unique_undo_backup_path;

impl PaneState {
    #[allow(dead_code)]
    pub(crate) fn copy_entry_into_current_dir(&mut self, source_path: &Path) -> io::Result<String> {
        let outcome = copy_path_into_dir(source_path, &self.cwd, false, false)?;
        self.reload()?;

        let pasted_path = outcome.target_path.clone();
        if let Some(index) = self.visible_indices.iter().position(|visible_index| {
            self.entries
                .get(*visible_index)
                .map(|candidate| candidate.path == pasted_path)
                .unwrap_or(false)
        }) {
            self.selected = index;
            self.list_state.select(Some(index));
        }

        Ok(outcome.display_name)
    }

    /// 複製項目並保留建立 Undo 所需的完整結果。
    ///
    /// 參數：`source_path: &Path`，要複製的來源檔案或資料夾。
    /// 回傳：`io::Result<PasteOutcome>`；成功時包含實際目標與可能的覆蓋備份。
    pub(crate) fn copy_entry_with_history(
        &mut self,
        source_path: &Path,
        overwrite: bool,
    ) -> io::Result<PasteOutcome> {
        let outcome = copy_path_into_dir(source_path, &self.cwd, overwrite, true)?;
        self.reload()?;
        self.select_path(&outcome.target_path);
        Ok(outcome)
    }

    /// 計算貼上操作實際會使用的完整目標路徑，但不建立或修改任何檔案。
    ///
    /// 這個方法讓上層在貼上失敗時能顯示真正的 destination，而不是只顯示目標目錄。
    /// 一般貼上若遇到同名項目，結果會包含 `copy` / `copy 2` 等實際名稱；覆蓋貼上則
    /// 回傳原始檔名。計算規則與真正執行 copy/move 的底層共用，避免錯誤訊息誤導使用者。
    ///
    /// 參數：
    /// - `self: &PaneState`，提供目前 pane 的目標目錄。
    /// - `source_path: &Path`，準備貼上的來源檔案或資料夾。
    /// - `overwrite: bool`，是否使用無條件覆蓋規則。
    ///
    /// 回傳：`io::Result<PathBuf>`。
    /// - 成功時回傳本次操作預計使用的完整目標路徑。
    /// - 失敗時代表來源路徑沒有可用檔名，無法建立目標路徑。
    pub(crate) fn planned_paste_target(
        &self,
        source_path: &Path,
        overwrite: bool,
    ) -> io::Result<PathBuf> {
        let file_name = source_path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
        })?;
        target_path_for_paste(source_path, &self.cwd, file_name, overwrite)
    }

    /// 在沒有借用 panel 狀態時，計算來源貼到指定目錄後的預定完整路徑。
    ///
    /// 參數：`source_path: &Path` 為來源；`target_dir: &Path` 為目的目錄；
    /// `overwrite: bool` 表示是否沿用原檔名覆蓋。
    /// 回傳：`io::Result<PathBuf>`，可供背景 paste 工作建立錯誤訊息與實際傳輸。
    pub(crate) fn planned_paste_target_in_dir(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
    ) -> io::Result<PathBuf> {
        let file_name = source_path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
        })?;
        target_path_for_paste(source_path, target_dir, file_name, overwrite)
    }

    /// 在背景工作中複製路徑，並於每次寫入資料後回報新增完成的 byte 數。
    ///
    /// 參數：`source_path: &Path` 為來源；`target_dir: &Path` 為目的目錄；
    /// `overwrite: bool` 表示是否覆蓋；`progress: &mut F` 接收目標可見、發現大小與完成量。
    /// 回傳：`io::Result<PasteOutcome>`；不重新載入 panel，適合 worker thread 使用。
    pub(crate) fn copy_path_to_dir_with_history_progress<F>(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
        progress: &mut F,
    ) -> io::Result<PasteOutcome>
    where
        F: FnMut(TransferProgress),
    {
        copy_path_into_dir_with_progress(source_path, target_dir, overwrite, true, progress)
    }

    /// 在背景工作中移動路徑；跨磁碟或 SMB 無法 rename 時改採 copy 後刪除來源。
    ///
    /// 參數：`source_path: &Path` 為來源；`target_dir: &Path` 為目的目錄；
    /// `overwrite: bool` 表示是否覆蓋；`progress: &mut F` 接收傳輸排程與 byte 進度。
    /// 回傳：`io::Result<PasteOutcome>`；同磁碟 rename 會立即完成，跨裝置則可持續回報。
    pub(crate) fn move_path_to_dir_with_history_progress<F>(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
        progress: &mut F,
    ) -> io::Result<PasteOutcome>
    where
        F: FnMut(TransferProgress),
    {
        Self::move_path_to_dir_with_history_progress_using_rename(
            source_path,
            target_dir,
            overwrite,
            progress,
            |source, target| fs::rename(source, target),
        )
    }

    /// 以可替換的原生 rename 執行背景 move，讓 fallback 規則可被單元測試完整覆蓋。
    ///
    /// 參數：`source_path`、`target_dir`、`overwrite` 與 `progress` 和公開入口相同；
    /// `rename_source` 型別為 `FnOnce(&Path, &Path) -> io::Result<()>`，只負責把來源移到
    /// 最終目標。回傳：`io::Result<PasteOutcome>`；rename 失敗時會改用
    /// 原生 copy，驗證成功後才刪除來源。
    pub(crate) fn move_path_to_dir_with_history_progress_using_rename<F, R>(
        source_path: &Path,
        target_dir: &Path,
        overwrite: bool,
        progress: &mut F,
        rename_source: R,
    ) -> io::Result<PasteOutcome>
    where
        F: FnMut(TransferProgress),
        R: FnOnce(&Path, &Path) -> io::Result<()>,
    {
        // 一般檔案可用單次 metadata 提供完成量；目錄不可在 rename 前遞迴掃描，否則
        // 原本應瞬間完成的同磁碟 move 會因數萬個子項目而先卡住數秒。
        let source_file_size = fs::metadata(source_path)
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len());
        match move_path_into_dir_with_source_rename(
            source_path,
            target_dir,
            overwrite,
            true,
            rename_source,
        ) {
            Ok(outcome) => {
                if let Some(source_file_size) = source_file_size {
                    progress(TransferProgress::BytesDiscovered(source_file_size));
                    progress(TransferProgress::BytesCopied(source_file_size));
                }
                Ok(outcome)
            }
            Err(_rename_error) => {
                let outcome = copy_path_into_dir_with_progress(
                    source_path,
                    target_dir,
                    overwrite,
                    true,
                    progress,
                )?;
                remove_existing_target(source_path)?;
                Ok(outcome)
            }
        }
    }

    /// 移動項目並保留建立 Undo 所需的完整結果。
    ///
    /// 參數：`source_path: &Path`，要移動的來源檔案或資料夾；`overwrite: bool`，是否覆蓋。
    /// 回傳：`io::Result<PasteOutcome>`；成功時包含目的路徑與覆蓋前備份。
    pub(crate) fn move_entry_with_history(
        &mut self,
        source_path: &Path,
        overwrite: bool,
    ) -> io::Result<PasteOutcome> {
        let outcome = move_path_into_dir(source_path, &self.cwd, overwrite, true)?;
        self.reload()?;
        self.select_path(&outcome.target_path);
        Ok(outcome)
    }

    /// 將來源複製到指定目錄並回傳 Undo 所需結果，不要求目標是目前 panel。
    ///
    /// 參數：`source_path: &Path`，來源路徑；`target_dir: &Path`，目的目錄。
    /// 回傳：`io::Result<PasteOutcome>`，成功時可直接寫入操作歷史。
    pub(crate) fn copy_path_to_dir_with_history(
        source_path: &Path,
        target_dir: &Path,
    ) -> io::Result<PasteOutcome> {
        copy_path_into_dir(source_path, target_dir, false, true)
    }

    /// 將來源移到指定目錄並回傳 Undo 所需結果，不要求目標是目前 panel。
    ///
    /// 參數：`source_path: &Path`，來源路徑；`target_dir: &Path`，目的目錄。
    /// 回傳：`io::Result<PasteOutcome>`，成功時可直接寫入操作歷史。
    pub(crate) fn move_path_to_dir_with_history(
        source_path: &Path,
        target_dir: &Path,
    ) -> io::Result<PasteOutcome> {
        move_path_into_dir(source_path, target_dir, false, true)
    }

    #[cfg(test)]
    /// 以可替換的 rename 函數測試同步 move 在跨裝置時的 copy 降級。
    pub(crate) fn move_path_to_dir_with_history_using_rename<R>(
        source_path: &Path,
        target_dir: &Path,
        rename_source: R,
    ) -> io::Result<PasteOutcome>
    where
        R: FnOnce(&Path, &Path) -> io::Result<()>,
    {
        move_path_into_dir_using_rename(source_path, target_dir, false, true, rename_source)
    }
}

fn copy_path_into_dir(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<PasteOutcome> {
    let file_name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
    })?;
    let target_path = target_path_for_paste(source_path, target_dir, file_name, overwrite)?;
    let backup_path = if overwrite {
        copy_path_transactional_with_backup(source_path, &target_path, true, retain_backup)?
    } else {
        copy_path_direct_with_cleanup(source_path, &target_path, |source, target| {
            if source.is_dir() {
                copy_dir_recursive(source, target)
            } else {
                copy_file_and_verify(source, target)
            }
        })?;
        None
    };

    let display_name = if source_path.is_dir() {
        format!("{}/", target_path_file_name(&target_path))
    } else {
        target_path_file_name(&target_path)
    };
    Ok(PasteOutcome {
        display_name,
        target_path,
        backup_path,
    })
}

/// 將單一路徑複製到目標資料夾，並持續回報實際讀寫完成量。
///
/// 參數與 [`copy_path_into_dir`] 相同；`progress` 接收走訪與實際寫入進度事件。
/// 回傳：`io::Result<PasteOutcome>`；失敗時沿用 partial target 清理與 Undo 備份規則。
fn copy_path_into_dir_with_progress<F>(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
    progress: &mut F,
) -> io::Result<PasteOutcome>
where
    F: FnMut(TransferProgress),
{
    let file_name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
    })?;
    let target_path = target_path_for_paste(source_path, target_dir, file_name, overwrite)?;
    let backup_path = if overwrite {
        let staged_path = unique_transfer_path(&target_path, "part");
        let copy_result = copy_path_with_progress(source_path, &staged_path, progress);
        if let Err(error) = copy_result {
            let _ = remove_transfer_path(&staged_path);
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "copy {} to {} failed: {error}",
                    source_path.display(),
                    target_path.display()
                ),
            ));
        }
        commit_staged_copy(&staged_path, &target_path, true, retain_backup)?
    } else {
        copy_path_direct_with_cleanup(source_path, &target_path, |source, target| {
            copy_path_with_progress(source, target, progress)
        })?;
        None
    };

    let display_name = if source_path.is_dir() {
        format!("{}/", target_path_file_name(&target_path))
    } else {
        target_path_file_name(&target_path)
    };
    Ok(PasteOutcome {
        display_name,
        target_path,
        backup_path,
    })
}

/// 依來源型別選擇可回報進度的單檔或遞迴資料夾複製。
///
/// 參數：`source_path`、`target_path` 為來源與目標；`progress` 接收傳輸進度事件。
/// 回傳：`io::Result<()>`；任一子項目失敗會交由外層清理整批目標。
fn copy_path_with_progress<F>(
    source_path: &Path,
    target_path: &Path,
    progress: &mut F,
) -> io::Result<()>
where
    F: FnMut(TransferProgress),
{
    if source_path.is_dir() {
        copy_dir_recursive_with_progress(source_path, target_path, progress)
    } else {
        let size = fs::metadata(source_path)?.len();
        progress(TransferProgress::BytesDiscovered(size));
        copy_file_native_with_progress(source_path, target_path, &mut |increment| {
            progress(TransferProgress::BytesCopied(increment));
        })
    }
}

/// 直接複製到正式目標，失敗時清除本次建立的部分內容。
///
/// 一般貼上採用這條跨平台原生檔案引擎路徑，
/// 不額外要求 SMB 伺服器允許 rename；目標名稱由上層保證原本不存在。
///
/// 參數：
/// - `source_path: &Path`，來源檔案或資料夾。
/// - `target_path: &Path`，本次新建立的正式目標路徑。
/// - `copy_to_target: F`，實際寫入函數，型別為 `FnOnce(&Path, &Path) -> io::Result<()>`。
///
/// 回傳：`io::Result<()>`；失敗時會盡力移除已建立的部分目標，避免佔用檔名。
pub(crate) fn copy_path_direct_with_cleanup<F>(
    source_path: &Path,
    target_path: &Path,
    copy_to_target: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    if target_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("paste target already exists: {}", target_path.display()),
        ));
    }
    if let Err(error) = copy_to_target(source_path, target_path) {
        let cleanup_result = remove_transfer_path(target_path);
        let cleanup_detail = cleanup_result
            .err()
            .map_or_else(String::new, |cleanup_error| {
                format!(
                    "; WARNING: partial target could not be removed: {} ({cleanup_error})",
                    target_path.display()
                )
            });
        return Err(io::Error::new(
            error.kind(),
            format!(
                "copy {} to {} failed: {error}{cleanup_detail}",
                source_path.display(),
                target_path.display()
            ),
        ));
    }
    Ok(())
}

/// 將單一路徑移動到目標資料夾。
///
/// 參數：
/// - `source_path: &Path`，來源檔案或資料夾。
/// - `target_dir: &Path`，貼上目標資料夾。
///
/// 將單一路徑移動到目標資料夾，優先嘗試快速的原生 rename；若失敗（例如跨磁碟／跨裝置 EXDEV）
/// 則自動降級為 copy_path_into_dir 加上刪除來源項目。
fn move_path_into_dir(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<PasteOutcome> {
    move_path_into_dir_using_rename(
        source_path,
        target_dir,
        overwrite,
        retain_backup,
        |source, target| fs::rename(source, target),
    )
}

/// 使用可替換的 rename 函數實作 move，支援失敗時降級為 copy + delete。
fn move_path_into_dir_using_rename<R>(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
    rename_source: R,
) -> io::Result<PasteOutcome>
where
    R: FnOnce(&Path, &Path) -> io::Result<()>,
{
    match move_path_into_dir_with_source_rename(
        source_path,
        target_dir,
        overwrite,
        retain_backup,
        rename_source,
    ) {
        Ok(outcome) => Ok(outcome),
        Err(_rename_error) => {
            let outcome = copy_path_into_dir(source_path, target_dir, overwrite, retain_backup)?;
            remove_existing_target(source_path)?;
            Ok(outcome)
        }
    }
}

/// 使用可替換的來源 rename 實作 move 的原生快速路徑。
///
/// 覆蓋既有目標時，舊內容仍由本函數先移到 Undo backup；`rename_source` 只處理來源
/// 到正式目標的那一步。這種切分讓測試能穩定模擬 Windows／SMB 回傳 unsupported，
/// 不需要依賴測試機器真的掛載另一個檔案系統。
///
/// 參數：前四項與 [`move_path_into_dir`] 相同；`rename_source` 是來源 rename 函數。
/// 回傳：`io::Result<PasteOutcome>`；rename 失敗時會先恢復覆蓋前目標，再回傳原始錯誤。
fn move_path_into_dir_with_source_rename<R>(
    source_path: &Path,
    target_dir: &Path,
    overwrite: bool,
    retain_backup: bool,
    rename_source: R,
) -> io::Result<PasteOutcome>
where
    R: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let file_name = source_path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "source path has no file name")
    })?;
    let target_path = target_path_for_paste(source_path, target_dir, file_name, overwrite)?;

    let backup_path =
        (overwrite && target_path.exists()).then(|| create_unique_undo_backup_path(&target_path));
    if let Some(backup_path) = &backup_path {
        move_path_with_fallback(&target_path, backup_path)?;
    }
    if let Err(error) = rename_source(source_path, &target_path) {
        if let Some(backup_path) = &backup_path {
            let _ = move_path_with_fallback(backup_path, &target_path);
        }
        return Err(error);
    }

    let display_name = if target_path.is_dir() {
        format!("{}/", target_path_file_name(&target_path))
    } else {
        target_path_file_name(&target_path)
    };
    let backup_path = if retain_backup {
        backup_path
    } else {
        if let Some(backup_path) = &backup_path {
            remove_transfer_path(backup_path)?;
        }
        None
    };
    Ok(PasteOutcome {
        display_name,
        target_path,
        backup_path,
    })
}

/// 執行交易式複製，並依需求把覆蓋前備份交給 Undo 歷史保存。
///
/// 參數：
/// - `source_path: &Path`，來源路徑。
/// - `target_path: &Path`，正式目標路徑。
/// - `overwrite: bool`，是否允許覆蓋。
/// - `retain_backup: bool`，成功後是否保留舊目標備份。
///
/// 回傳：`io::Result<Option<PathBuf>>`；有覆蓋且要求保留時回傳備份路徑。
fn copy_path_transactional_with_backup(
    source_path: &Path,
    target_path: &Path,
    overwrite: bool,
    retain_backup: bool,
) -> io::Result<Option<PathBuf>> {
    let staged_path = unique_transfer_path(target_path, "part");
    let copy_result: io::Result<()> = if source_path.is_dir() {
        copy_dir_recursive(source_path, &staged_path)
    } else {
        copy_file_and_verify(source_path, &staged_path)
    };
    if let Err(ref error) = copy_result {
        let _ = remove_transfer_path(&staged_path);
        return Err(io::Error::new(
            error.kind(),
            format!(
                "copy {} to {} failed: {error}",
                source_path.display(),
                target_path.display()
            ),
        ));
    }

    commit_staged_copy(&staged_path, target_path, overwrite, retain_backup)
}

/// 依照是否允許覆蓋，決定貼上時真正要使用的目標路徑。
///
/// 規則：
/// - `overwrite = false` 時，沿用原本的 `copy`, `copy 2` 命名策略。
/// - `overwrite = true` 且來源與目標不是同一個實體時，回傳原始名稱；
///   既有目標要由呼叫端依照複製或移動操作的安全規則處理。
/// - 若來源本來就在同一個目錄，為了避免覆蓋自己，會退回不覆蓋策略。
fn target_path_for_paste(
    source_path: &Path,
    target_dir: &Path,
    original_name: &std::ffi::OsStr,
    overwrite: bool,
) -> io::Result<PathBuf> {
    let original_name = original_name.to_string_lossy();
    let direct_target = target_dir.join(original_name.as_ref());

    let same_location = source_path.parent() == Some(target_dir) && direct_target == source_path;
    if !overwrite || same_location {
        return Ok(unique_target_path(
            target_dir,
            std::ffi::OsStr::new(original_name.as_ref()),
        ));
    }

    Ok(direct_target)
}

/// 先把來源完整複製到目標目錄內的暫存路徑，成功後才切換成正式名稱。
///
/// 參數：
/// - `source_path: &Path`，要複製的來源檔案或資料夾。
/// - `target_path: &Path`，使用者最後應看見的正式目標路徑。
/// - `overwrite: bool`，`true` 代表允許在傳輸完成後替換既有目標。
///
/// 回傳：`io::Result<()>`；成功時正式路徑已完整可用，失敗時不會留下
/// 只有部分內容的正式檔名，並會盡力清除內部暫存路徑。
#[allow(dead_code)]
pub(crate) fn copy_path_transactional(
    source_path: &Path,
    target_path: &Path,
    overwrite: bool,
) -> io::Result<()> {
    copy_path_transactional_with(source_path, target_path, overwrite, |source, staged| {
        if source.is_dir() {
            copy_dir_recursive(source, staged)
        } else {
            copy_file_and_verify(source, staged)
        }
    })
}

/// 執行可注入複製器的交易式複製核心，讓失敗清理規則可以被單元測試完整驗證。
///
/// 參數：
/// - `source_path: &Path`，來源路徑。
/// - `target_path: &Path`，正式目標路徑。
/// - `overwrite: bool`，是否允許替換既有目標。
/// - `copy_to_staged: F`，把來源寫入暫存路徑的函數，型別為
///   `FnOnce(&Path, &Path) -> io::Result<()>`。
///
/// 回傳：`io::Result<()>`，成功時已提交正式名稱；失敗時保留原目標並清理暫存資料。
#[allow(dead_code)]
pub(crate) fn copy_path_transactional_with<F>(
    source_path: &Path,
    target_path: &Path,
    overwrite: bool,
    copy_to_staged: F,
) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let staged_path = unique_transfer_path(target_path, "part");
    if let Err(error) = copy_to_staged(source_path, &staged_path) {
        let _ = remove_transfer_path(&staged_path);
        return Err(io::Error::new(
            error.kind(),
            format!(
                "copy {} to {} failed: {error}",
                source_path.display(),
                target_path.display()
            ),
        ));
    }

    if let Err(error) = commit_staged_copy(&staged_path, target_path, overwrite, false) {
        let _ = remove_transfer_path(&staged_path);
        return Err(error);
    }

    Ok(())
}

/// 使用平台原生 copy 複製單一檔案，並在完成後確認來源、回報值與目標大小一致。
///
/// 清除 Undo 歷史不再需要的覆蓋備份。
///
/// 參數：`path: &Path`，由貼上結果交回的隱藏備份路徑。
/// 回傳：`io::Result<()>`；路徑不存在時視為已清理完成。
pub(crate) fn remove_undo_backup(path: &Path) -> io::Result<()> {
    remove_transfer_path(path)
}

/// 將來源路徑移動到目標路徑，若跨裝置導致 rename 失敗，則退回 copy+delete。
pub(crate) fn move_path_with_fallback(source: &Path, destination: &Path) -> io::Result<()> {
    if fs::rename(source, destination).is_ok() {
        return Ok(());
    }
    if source.is_dir() {
        copy_dir_recursive(source, destination)?;
        remove_existing_target(source)?;
    } else {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        copy_file_and_verify(source, destination)?;
        remove_existing_target(source)?;
    }
    Ok(())
}

/// 移除即將被覆蓋的既有目標檔案或資料夾。
///
/// 參數：
/// - `target_path: &Path`，即將被覆蓋的既有檔案或資料夾。
///
/// 回傳：`io::Result<()>`。
/// - 成功時代表既有目標已被安全移除。
/// - 失敗時代表沒有權限、目標正被使用或刪除過程出錯。
pub(crate) fn remove_existing_target(target_path: &Path) -> io::Result<()> {
    if !target_path.exists() {
        return Ok(());
    }
    let mut last_err = None;
    for _ in 0..10 {
        let res = if target_path.is_dir() {
            fs::remove_dir_all(target_path)
        } else {
            fs::remove_file(target_path)
        };
        match res {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
    if let Some(err) = last_err {
        Err(err)
    } else {
        Ok(())
    }
}
