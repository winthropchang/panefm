//! 選取項目的跨目錄移動與複製（依路徑或 pane 編號傳輸）。

use std::io;
use std::path::Path;

use super::super::*;

impl App {
    /// 將目前選取或已標記的項目直接移動到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，目標目錄，可以是絕對路徑或相對於目前 pane 的路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_path(&mut self, target: &str) -> io::Result<()> {
        let Some(target_dir) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: move <target-dir>");
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 將目前選取或已標記的項目移到指定 pane 目前所在的目錄。
    ///
    /// 參數：
    /// - `target: &str`，目標 pane 編號字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn move_selected_to_pane_id(&mut self, target: &str) -> io::Result<()> {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: move-panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        let Some(target_dir) = self.panes.get(&target_pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        self.move_selected_entries_into_dir(&target_dir)
    }

    /// 將目前焦點 pane 的選取項目批次移動到目標目錄。
    pub(crate) fn move_selected_entries_into_dir(&mut self, target_dir: &Path) -> io::Result<()> {
        let Some(source_pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let source_dir = source_pane.cwd.clone();
        let entries = source_pane.selected_or_marked_entries();

        if entries.is_empty() {
            self.status = String::from("nothing selected to move");
            return Ok(());
        }

        if !target_dir.exists() {
            self.status = format!("move target does not exist: {}", target_dir.display());
            return Ok(());
        }
        if !target_dir.is_dir() {
            self.status = format!("move target is not a directory: {}", target_dir.display());
            return Ok(());
        }
        if source_dir == target_dir {
            self.status = String::from("move target is the current directory");
            return Ok(());
        }

        let clipboard = ClipboardState {
            operation: ClipboardOperation::Cut,
            entries: entries
                .iter()
                .map(|entry| ClipboardEntry {
                    source_path: entry.path.clone(),
                    display_name: entry.display_name(),
                })
                .collect(),
        };

        if paste_should_run_in_background(&clipboard, target_dir) {
            return self.start_background_paste(
                self.focused_pane,
                target_dir.to_path_buf(),
                clipboard,
                false,
            );
        }

        let mut moved_count = 0usize;
        let mut history_items = Vec::new();
        for entry in &entries {
            let outcome = match PaneState::move_path_to_dir_with_history(&entry.path, target_dir) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.operation_history.push(FileOperation {
                        kind: FileOperationKind::Move,
                        items: history_items,
                    });
                    self.status = format!("move failed for {}: {error}", entry.display_name());
                    return Ok(());
                }
            };
            moved_count += 1;
            history_items.push(OperationItem {
                source_path: entry.path.clone(),
                destination_path: outcome.target_path,
                replaced_backup: outcome.backup_path,
            });
        }

        self.operation_history.push(FileOperation {
            kind: FileOperationKind::Move,
            items: history_items,
        });
        self.reload_all_panes()?;
        self.status = if moved_count == 1 {
            format!("moved 1 item -> {}", target_dir.display())
        } else {
            format!("moved {moved_count} items -> {}", target_dir.display())
        };
        Ok(())
    }

    /// 將目前選取或已標記的項目複製到指定路徑。
    ///
    /// 參數：
    /// - `target: &str`，目標目錄，可以是絕對路徑或相對於目前 pane 的路徑。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn copy_selected_to_path(&mut self, target: &str) -> io::Result<()> {
        let Some(target_dir) = self.resolve_path_argument(target) else {
            self.status = String::from("usage: copy <target-dir>");
            return Ok(());
        };
        self.copy_selected_entries_into_dir(&target_dir)
    }

    /// 將目前選取或已標記的項目複製到指定 pane 目前所在的目錄。
    ///
    /// 參數：
    /// - `target: &str`，目標 pane 編號字串。
    ///
    /// 回傳：`io::Result<()>`。
    pub(crate) fn copy_selected_to_pane_id(&mut self, target: &str) -> io::Result<()> {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: copy-panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        let Some(target_dir) = self.panes.get(&target_pane_id).map(|pane| pane.cwd.clone()) else {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
            return Ok(());
        };
        self.copy_selected_entries_into_dir(&target_dir)
    }

    /// 將目前焦點 pane 的選取項目批次複製到目標目錄。
    pub(crate) fn copy_selected_entries_into_dir(&mut self, target_dir: &Path) -> io::Result<()> {
        let Some(source_pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let entries = source_pane.selected_or_marked_entries();

        if entries.is_empty() {
            self.status = String::from("nothing selected to copy");
            return Ok(());
        }

        if !target_dir.exists() {
            self.status = format!("copy target does not exist: {}", target_dir.display());
            return Ok(());
        }
        if !target_dir.is_dir() {
            self.status = format!("copy target is not a directory: {}", target_dir.display());
            return Ok(());
        }

        let clipboard = ClipboardState {
            operation: ClipboardOperation::Copy,
            entries: entries
                .iter()
                .map(|entry| ClipboardEntry {
                    source_path: entry.path.clone(),
                    display_name: entry.display_name(),
                })
                .collect(),
        };

        if paste_should_run_in_background(&clipboard, target_dir) {
            return self.start_background_paste(
                self.focused_pane,
                target_dir.to_path_buf(),
                clipboard,
                false,
            );
        }

        let mut copied_count = 0usize;
        let mut history_items = Vec::new();
        for entry in &entries {
            let outcome = match PaneState::copy_path_to_dir_with_history(&entry.path, target_dir) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.operation_history.push(FileOperation {
                        kind: FileOperationKind::Copy,
                        items: history_items,
                    });
                    self.status = format!("copy failed for {}: {error}", entry.display_name());
                    return Ok(());
                }
            };
            copied_count += 1;
            history_items.push(OperationItem {
                source_path: entry.path.clone(),
                destination_path: outcome.target_path,
                replaced_backup: outcome.backup_path,
            });
        }

        self.operation_history.push(FileOperation {
            kind: FileOperationKind::Copy,
            items: history_items,
        });
        self.reload_all_panes()?;
        self.status = if copied_count == 1 {
            format!("copied 1 item -> {}", target_dir.display())
        } else {
            format!("copied {copied_count} items -> {}", target_dir.display())
        };
        Ok(())
    }
}
