#![allow(unused_imports)]

use super::*;

impl App {
    /// 進入 visual selection 模式，準備用移動游標的方式框選一段範圍。
    pub(crate) fn open_visual_selection(&mut self) -> io::Result<()> {
        let pane_id = self.focused_pane;
        let selected = self.current_pane_mut()?.selected;
        self.visual_selection = Some(VisualSelectionState {
            pane_id,
            anchor: selected,
            current: selected,
        });
        self.pending_g = false;
        self.pending_y = false;
        self.status = String::from("visual: range selection");
        Ok(())
    }

    /// 將目前 visual selection 範圍加入已標記清單，並回到一般模式。
    pub(crate) fn commit_visual_selection(&mut self) -> io::Result<()> {
        let Some(selection) = self.visual_selection.take() else {
            return Ok(());
        };
        let Some(pane) = self.panes.get_mut(&selection.pane_id) else {
            self.status = String::from("panel no longer exists");
            return Ok(());
        };

        let added = pane.mark_range(selection.anchor, selection.current);
        self.status = format!("marked {added} items");
        self.pending_g = false;
        Ok(())
    }

    /// 當 visual selection 模式中游標移動後，更新目前選取範圍的尾端。
    pub(crate) fn sync_visual_selection_cursor(&mut self) {
        if let Some(selection) = &mut self.visual_selection
            && let Some(pane) = self.panes.get(&selection.pane_id)
        {
            selection.current = pane.selected;
        }
    }

    /// 回傳 visual selection 狀態列文字，包含目前暫時框選的項目數量。
    pub(crate) fn visual_status_label(&self) -> String {
        match &self.visual_selection {
            Some(selection) => {
                let count = selection.anchor.abs_diff(selection.current) + 1;
                format!("visual: selecting {count} items")
            }
            None => String::from("normal mode"),
        }
    }

    /// 將目前 trash 視覺選取範圍加入已標記清單。
    pub(crate) fn commit_trash_visual_selection(
        &self,
        entries: &[TrashListEntry],
        marked_ids: &mut Vec<String>,
        anchor: usize,
        current: usize,
    ) -> usize {
        let start = anchor.min(current);
        let end = anchor.max(current);
        let mut added = 0usize;

        for entry in entries
            .iter()
            .skip(start)
            .take(end.saturating_sub(start) + 1)
        {
            if !marked_ids.iter().any(|id| id == &entry.id) {
                marked_ids.push(entry.id.clone());
                added += 1;
            }
        }

        added
    }

    /// 將目前 task 視覺選取範圍加入已標記清單。
    pub(crate) fn commit_task_visual_selection(
        &self,
        tasks: &[TaskRecord],
        marked_ids: &mut Vec<usize>,
        anchor: usize,
        current: usize,
    ) -> usize {
        let start = anchor.min(current);
        let end = anchor.max(current);
        let mut added = 0usize;

        for task in tasks.iter().skip(start).take(end.saturating_sub(start) + 1) {
            if !marked_ids.contains(&task.id) {
                marked_ids.push(task.id);
                added += 1;
            }
        }

        added
    }

    /// 回傳 task 面板 visual mode 的狀態列文字。
    pub(crate) fn task_visual_status_label(
        &self,
        anchor: usize,
        current: usize,
        marked_count: usize,
    ) -> String {
        let selecting = anchor.abs_diff(current) + 1;
        if marked_count > 0 {
            format!("task visual: selecting {selecting} tasks ({marked_count} marked)")
        } else {
            format!("task visual: selecting {selecting} tasks")
        }
    }

    /// 清除目前焦點 pane 中所有標記。
    pub(crate) fn clear_marks_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let count = pane.marked_count();
        pane.clear_marks();
        self.status = if count == 0 {
            String::from("no marks to clear")
        } else {
            format!("cleared {count} marks")
        };
        Ok(())
    }

    /// 將目前焦點 pane 中所有可見項目全部標記，方便後續做批次操作。
    pub(crate) fn mark_all_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let added = pane.mark_all_visible();
        let total = pane.marked_count();
        self.status = if total == 0 {
            String::from("nothing to mark")
        } else {
            format!("marked all visible items (+{added}, total {total})")
        };
        Ok(())
    }

    /// 切換目前焦點項目的標記狀態，讓單項多選操作更直接。
    pub(crate) fn toggle_mark_selected_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let selected_name = pane.selected_entry().map(|entry| entry.display_name());
        match pane.toggle_mark_selected() {
            Some(true) => {
                let name = selected_name.unwrap_or_else(|| String::from("item"));
                self.status = format!("marked {name}");
            }
            Some(false) => {
                let name = selected_name.unwrap_or_else(|| String::from("item"));
                self.status = format!("unmarked {name}");
            }
            None => {
                self.status = String::from("nothing selected to mark");
            }
        }
        Ok(())
    }

    /// 反轉目前焦點 pane 所有可見項目的標記狀態。
    pub(crate) fn invert_marks_in_focused_pane(&mut self) -> io::Result<()> {
        let pane = self.current_pane_mut()?;
        let (added, removed) = pane.invert_visible_marks();
        let total = pane.marked_count();
        self.status = if added == 0 && removed == 0 {
            String::from("nothing to invert")
        } else {
            format!("inverted visible marks (+{added}, -{removed}, total {total})")
        };
        Ok(())
    }

    /// 判斷目前是否仍有任何 pane 保留已提交的標記。
    pub(crate) fn has_any_marks(&self) -> bool {
        self.panes.values().any(|pane| pane.marked_count() > 0)
    }

    /// 清掉所有 pane 的已提交標記，讓整個畫面回到一般模式。
    pub(crate) fn clear_all_marks(&mut self) {
        let mut cleared = 0usize;
        for pane in self.panes.values_mut() {
            cleared += pane.marked_count();
            pane.clear_marks();
        }

        self.pending_g = false;
        self.pending_y = false;
        self.status = if cleared == 0 {
            String::from("normal mode")
        } else {
            format!("cleared {cleared} marks")
        };
    }
}
