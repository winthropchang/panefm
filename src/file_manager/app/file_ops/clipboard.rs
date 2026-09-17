//! 內部剪貼簿存取、複製/剪下狀態管理與系統剪貼簿匯出。

use std::io;

use super::super::*;

impl App {
    /// 打開 `Copy` 面板，讓使用者把不同格式的文字直接複製到系統剪貼簿。
    pub(crate) fn open_copy_picker(&mut self) -> io::Result<()> {
        let Some(target) = self.selected_open_target() else {
            self.status = String::from("nothing selected to copy");
            return Ok(());
        };

        self.pending_action = Some(PendingAction::CopyPicker {
            pane_id: self.focused_pane,
            target: target.clone(),
            selected: 0,
        });
        self.status = format!("copy to clipboard: {}", target.display_name);
        Ok(())
    }

    /// 根據選擇的複製動作，把文字寫進系統剪貼簿。
    pub(crate) fn copy_target_to_system_clipboard(
        &mut self,
        target: OpenTarget,
        action: CopyAction,
    ) -> io::Result<()> {
        let text = build_copy_text(&target, action)
            .map_err(|error| io::Error::other(error.to_string()))?;
        write_text_to_system_clipboard(&text)?;
        self.status = format!(
            "{}: {}",
            copy_action_status_label(action),
            target.display_name
        );
        Ok(())
    }

    /// 將目前選取項目放進內部剪貼簿，模式為複製。
    pub(crate) fn copy_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Copy);
    }

    /// 將目前選取項目放進內部剪貼簿，模式為剪下。
    pub(crate) fn cut_selected(&mut self) {
        self.store_selected_in_clipboard(ClipboardOperation::Cut);
    }

    /// 清除目前內部剪貼簿中指定類型的 yank 狀態。
    ///
    /// 規則：
    /// - 若目前剪貼簿剛好就是指定操作類型，就清掉它。
    /// - 若目前不是該類型，則只更新狀態列，不動既有內容。
    pub(crate) fn clear_clipboard(&mut self, operation: ClipboardOperation) {
        match self.clipboard.as_ref() {
            Some(clipboard) if clipboard.operation == operation => {
                self.clipboard = None;
                self.status = match operation {
                    ClipboardOperation::Copy => String::from("cleared copied items"),
                    ClipboardOperation::Cut => String::from("cleared cut items"),
                };
            }
            _ => {
                self.status = match operation {
                    ClipboardOperation::Copy => String::from("no copied items to clear"),
                    ClipboardOperation::Cut => String::from("no cut items to clear"),
                };
            }
        }
    }

    /// 把目前焦點 pane 的選取項目寫入剪貼簿。
    pub(crate) fn store_selected_in_clipboard(&mut self, operation: ClipboardOperation) {
        let Some(pane) = self.panes.get(&self.focused_pane) else {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        };

        let entries: Vec<ClipboardEntry> = pane
            .selected_or_marked_entries()
            .into_iter()
            .map(|entry| {
                let display_name = entry.display_name();
                ClipboardEntry {
                    source_path: entry.path,
                    display_name,
                }
            })
            .collect();

        if entries.is_empty() {
            self.status = match operation {
                ClipboardOperation::Copy => String::from("nothing selected to copy"),
                ClipboardOperation::Cut => String::from("nothing selected to cut"),
            };
            return;
        }

        let count = entries.len();
        self.clipboard = Some(ClipboardState { entries, operation });

        self.status = match operation {
            ClipboardOperation::Copy if count == 1 => String::from("copied 1 item"),
            ClipboardOperation::Copy => format!("copied {count} items"),
            ClipboardOperation::Cut if count == 1 => String::from("cut 1 item"),
            ClipboardOperation::Cut => format!("cut {count} items"),
        };
    }
}
