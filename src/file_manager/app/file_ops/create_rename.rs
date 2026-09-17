//! 建立新檔案/目錄、單一重新命名與正則表達式批次改名。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;

use regex::Regex;

use super::super::*;

impl App {
    /// 開始重新命名流程，建立一個待輸入的新名稱互動。
    pub(crate) fn start_rename(&mut self) {
        let Some(entry) = self
            .panes
            .get(&self.focused_pane)
            .and_then(PaneState::selected_entry)
            .cloned()
        else {
            self.status = String::from("nothing selected to rename");
            return;
        };

        self.pending_action = Some(PendingAction::Rename {
            pane_id: self.focused_pane,
            original_name: entry.display_name(),
            cursor: rename_basename_cursor(&entry.name),
            buffer: entry.name,
            mode: RenameMode::Insert,
        });
        self.status = String::from("rename: insert");
    }

    /// 真正執行重新命名目前待確認項目的檔案系統操作。
    pub(crate) fn confirm_rename(
        &mut self,
        pane_id: usize,
        original_name: &str,
        new_name: &str,
    ) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let rename_result = {
            let pane = self
                .panes
                .get_mut(&pane_id)
                .expect("checked pane existence before rename");
            pane.rename_selected(new_name)
        };

        match rename_result {
            Ok(Some(renamed_name)) => {
                self.reload_all_panes()?;
                self.status = format!("renamed {original_name} -> {renamed_name}");
            }
            Ok(None) => {
                self.status = String::from("nothing selected to rename");
            }
            Err(error) => {
                self.status = format!("failed to rename {original_name}: {error}");
            }
        }

        Ok(())
    }

    /// 開始建立新檔案流程，打開一個可直接輸入名稱的 inline 編輯器。
    pub(crate) fn start_create_entry(&mut self) {
        self.pending_action = Some(PendingAction::CreateEntry {
            pane_id: self.focused_pane,
            buffer: String::new(),
            cursor: 0,
            mode: RenameMode::Insert,
        });
        self.status = create_status_label("insert");
    }

    /// 真正執行建立新項目的檔案系統操作。
    pub(crate) fn confirm_create_entry(&mut self, pane_id: usize, path: &str) -> io::Result<()> {
        let Some(_) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let create_result = {
            let pane = self
                .panes
                .get_mut(&pane_id)
                .expect("checked pane existence before create");
            pane.create_entry(path)
        };

        match create_result {
            Ok(created_name) => {
                self.reload_all_panes()?;
                let item_type = if created_name.ends_with('/') {
                    "directory"
                } else {
                    "file"
                };
                self.status = format!("created {item_type}: {created_name}");
            }
            Err(error) => {
                self.status = format!("failed to create entry: {error}");
            }
        }

        Ok(())
    }

    /// 讓命令模式可以直接建立新項目，而不必再開啟 inline 輸入框。
    pub(crate) fn create_entry_from_command(&mut self, path: &str) -> io::Result<()> {
        self.confirm_create_entry(self.focused_pane, path)
    }

    /// 解析 `:rename-regex` 指令參數，並建立批次改名預覽。
    pub(crate) fn start_regex_rename_from_command(&mut self, args: &str) -> io::Result<()> {
        let parsed = shlex::split(args).unwrap_or_default();
        if parsed.len() != 2 {
            self.status = String::from("usage: rename-regex <pattern> <replace>");
            return Ok(());
        }
        self.open_regex_rename_preview(&parsed[0], &parsed[1])
    }

    /// 依照 regex 規則建立目前選取項目的批次改名預覽。
    pub(crate) fn open_regex_rename_preview(
        &mut self,
        pattern: &str,
        replacement: &str,
    ) -> io::Result<()> {
        let regex = match Regex::new(pattern) {
            Ok(regex) => regex,
            Err(error) => {
                self.status = format!("invalid regex: {error}");
                return Ok(());
            }
        };
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let entries = pane.selected_or_marked_entries();
        if entries.is_empty() {
            self.status = String::from("nothing selected to rename");
            return Ok(());
        }

        let selected_paths = entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>();
        let existing_names = pane
            .entries
            .iter()
            .filter(|entry| !selected_paths.contains(&entry.path))
            .map(|entry| entry.name.clone())
            .collect::<BTreeSet<_>>();

        let mut previews = entries
            .into_iter()
            .map(|entry| {
                let new_name = regex.replace_all(&entry.name, replacement).into_owned();
                let outcome = classify_regex_rename_preview(&entry.name, &new_name);
                RegexRenamePreview {
                    source_path: entry.path,
                    original_name: entry.name,
                    new_name,
                    outcome,
                }
            })
            .collect::<Vec<_>>();

        let mut target_counts = BTreeMap::new();
        for preview in previews
            .iter()
            .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
        {
            *target_counts
                .entry(preview.new_name.clone())
                .or_insert(0usize) += 1;
        }

        for preview in &mut previews {
            if !matches!(preview.outcome, RegexRenameOutcome::Ready) {
                continue;
            }
            if target_counts.get(&preview.new_name).copied().unwrap_or(0) > 1
                || existing_names.contains(&preview.new_name)
            {
                preview.outcome = RegexRenameOutcome::Conflict;
            }
        }

        self.pending_action = Some(PendingAction::RegexRename {
            pane_id: self.focused_pane,
            pattern: pattern.to_string(),
            replacement: replacement.to_string(),
            selected: 0,
            previews,
        });
        if let Some(action) = self.pending_action.as_ref() {
            self.status = self.status_for_pending_action(action)?;
        }
        Ok(())
    }

    /// 真正執行 regex 批次改名預覽中所有可套用的項目。
    pub(crate) fn apply_regex_rename_preview(
        &mut self,
        pane_id: usize,
        previews: &[RegexRenamePreview],
    ) -> io::Result<()> {
        let Some(pane) = self.panes.get(&pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };
        let cwd = pane.cwd.clone();
        let ready = previews
            .iter()
            .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
            .cloned()
            .collect::<Vec<_>>();

        if previews.iter().any(|preview| {
            matches!(
                preview.outcome,
                RegexRenameOutcome::Conflict | RegexRenameOutcome::Invalid
            )
        }) {
            self.status = String::from("rename-regex: resolve conflicts before apply");
            return Ok(());
        }
        if ready.is_empty() {
            self.status = String::from("rename-regex: nothing to apply");
            return Ok(());
        }

        let staged = ready
            .iter()
            .enumerate()
            .map(|(index, preview)| {
                let temp_path =
                    unique_regex_rename_temp_path(&cwd, &preview.original_name, index, &ready);
                (
                    preview.source_path.clone(),
                    temp_path,
                    cwd.join(&preview.new_name),
                )
            })
            .collect::<Vec<_>>();

        for (source_path, temp_path, _) in &staged {
            fs::rename(source_path, temp_path)?;
        }

        for (_, temp_path, final_path) in &staged {
            if let Err(error) = fs::rename(temp_path, final_path) {
                self.status = format!("rename-regex failed: {error}");
                return Ok(());
            }
        }

        self.reload_all_panes()?;
        self.status = if ready.len() == 1 {
            String::from("rename-regex: renamed 1 item")
        } else {
            format!("rename-regex: renamed {} items", ready.len())
        };
        Ok(())
    }
}
