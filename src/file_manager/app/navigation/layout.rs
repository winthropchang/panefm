//! 視窗分割、重新編號、焦點切換、尺寸調整、關閉與 Pane ID Remapping。

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::mem;

use ratatui::layout::Rect;

use super::super::*;
use crate::file_manager::layout::{LayoutNode, SplitDirection, SplitPlacement, pane_spatial_cmp};

impl App {
    /// 取得目前有焦點的 pane 可變參考。
    pub(crate) fn current_pane_mut(&mut self) -> io::Result<&mut PaneState> {
        self.panes
            .get_mut(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused panel"))
    }

    /// 將目前焦點 pane 依指定方向分割成兩個 pane。
    pub(crate) fn split_current(&mut self, direction: SplitDirection) -> io::Result<()> {
        self.split_current_at(direction, SplitPlacement::After)
    }

    /// 將目前焦點 pane 依指定方向與位置分割成兩個 pane。
    pub(crate) fn split_current_at(
        &mut self,
        direction: SplitDirection,
        placement: SplitPlacement,
    ) -> io::Result<()> {
        let source_pane = self
            .panes
            .get(&self.focused_pane)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing focused panel"))?;
        let cwd = source_pane.cwd.clone();
        let show_hidden = source_pane.show_hidden;
        let sort_mode = source_pane.sort_mode;

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let mut pane = PaneState::new(cwd)?;
        pane.set_show_hidden(show_hidden);
        pane.set_sort_mode(sort_mode);
        if let Some(line_mode) = source_pane.line_mode {
            pane.set_line_mode(line_mode);
        }
        self.panes.insert(new_id, pane);
        self.layout =
            self.layout
                .clone()
                .split_leaf(self.focused_pane, direction, placement, new_id);
        self.focused_pane = new_id;
        self.renumber_panes();
        self.status = match (direction, placement) {
            (SplitDirection::Horizontal, SplitPlacement::Before) => String::from("split up"),
            (SplitDirection::Horizontal, SplitPlacement::After) => String::from("split down"),
            (SplitDirection::Vertical, SplitPlacement::Before) => String::from("split left"),
            (SplitDirection::Vertical, SplitPlacement::After) => String::from("split right"),
        };
        Ok(())
    }

    /// 依畫面幾何位置（先上下、再左右）將所有 panel 動態重新編號為 1..=N。
    pub(crate) fn renumber_panes(&mut self) {
        if self.panes.is_empty() {
            return;
        }

        // 1. 在標準化虛擬畫布上計算每個 pane 的矩形區域
        let normalized_area = Rect {
            x: 0,
            y: 0,
            width: 10000,
            height: 10000,
        };
        let mut pane_rects = BTreeMap::new();
        self.layout.render_rects(normalized_area, &mut pane_rects);

        // 2. 依照「先上下、再左右」幾何順序排序所有目前的 pane id
        let mut sorted_ids: Vec<usize> = self.panes.keys().copied().collect();
        sorted_ids.sort_by(|&id_a, &id_b| {
            let rect_a = pane_rects.get(&id_a);
            let rect_b = pane_rects.get(&id_b);
            match (rect_a, rect_b) {
                (Some(ra), Some(rb)) => pane_spatial_cmp(ra, rb),
                _ => id_a.cmp(&id_b),
            }
        });

        // 3. 建立舊 id -> 新 id (1..=N) 的對應表
        let id_map: HashMap<usize, usize> = sorted_ids
            .iter()
            .enumerate()
            .map(|(index, &old_id)| (old_id, index + 1))
            .collect();

        // 若編號完全未變動（例如原本就是 1..N 且順序相同），重設 next_pane_id 並提早返回
        if id_map.iter().all(|(old_id, new_id)| old_id == new_id) {
            self.next_pane_id = self.panes.len() + 1;
            return;
        }

        // 4. 更新 layout 樹中的所有 leaf id
        self.layout.remap_pane_ids(&id_map);

        // 5. 重建 self.panes
        let old_panes = mem::take(&mut self.panes);
        let mut new_panes = BTreeMap::new();
        for (old_id, pane) in old_panes {
            if let Some(&new_id) = id_map.get(&old_id) {
                new_panes.insert(new_id, pane);
            }
        }
        self.panes = new_panes;

        // 6. 更新 focused_pane
        if let Some(&new_focus) = id_map.get(&self.focused_pane) {
            self.focused_pane = new_focus;
        } else if let Some(&first) = self.panes.keys().next() {
            self.focused_pane = first;
        }

        // 7. 更新 next_pane_id
        self.next_pane_id = self.panes.len() + 1;

        // 8. 同步更新所有相依的狀態
        self.remap_dependent_pane_ids(&id_map);
    }

    /// 將所有依附於 pane_id 的內部狀態、背景工作與暫時面板同步更新至新編號。
    pub(crate) fn remap_dependent_pane_ids(&mut self, map: &HashMap<usize, usize>) {
        // 重映射 directory_size_jobs
        let old_size_jobs = mem::take(&mut self.directory_size_jobs);
        let mut new_size_jobs = BTreeMap::new();
        for (old_id, job) in old_size_jobs {
            let new_id = map.get(&old_id).copied().unwrap_or(old_id);
            new_size_jobs.insert(new_id, job);
        }
        self.directory_size_jobs = new_size_jobs;

        // 重映射 directory_load_jobs
        let old_load_jobs = mem::take(&mut self.directory_load_jobs);
        let mut new_load_jobs = BTreeMap::new();
        for (old_id, job) in old_load_jobs {
            let new_id = map.get(&old_id).copied().unwrap_or(old_id);
            new_load_jobs.insert(new_id, job);
        }
        self.directory_load_jobs = new_load_jobs;

        // 重映射 visual_selection
        if let Some(vs) = &mut self.visual_selection {
            vs.pane_id = map.get(&vs.pane_id).copied().unwrap_or(vs.pane_id);
        }

        // 重映射 filter
        if let Some(f) = &mut self.filter {
            f.pane_id = map.get(&f.pane_id).copied().unwrap_or(f.pane_id);
        }

        // 重映射 preview_search
        if let Some(ps) = &mut self.preview_search {
            ps.pane_id = map.get(&ps.pane_id).copied().unwrap_or(ps.pane_id);
        }

        // 重映射 list_find
        if let Some(lf) = &mut self.list_find {
            lf.pane_id = map.get(&lf.pane_id).copied().unwrap_or(lf.pane_id);
        }

        // 重映射 global_search
        if let Some(gs) = &mut self.global_search {
            gs.pane_id = map.get(&gs.pane_id).copied().unwrap_or(gs.pane_id);
        }

        // 重映射 pending_fzf_jump
        if let Some(req) = &mut self.pending_fzf_jump {
            req.pane_id = map.get(&req.pane_id).copied().unwrap_or(req.pane_id);
        }

        // 重映射 help_return
        if let Some(hr) = &mut self.help_return {
            match hr {
                HelpReturnState::PreviewFocus(pid) => {
                    if let Some(&new_id) = map.get(pid) {
                        *pid = new_id;
                    }
                }
                HelpReturnState::Filter(f) => {
                    if let Some(&new_id) = map.get(&f.pane_id) {
                        f.pane_id = new_id;
                    }
                }
                HelpReturnState::PreviewSearch(ps) => {
                    if let Some(&new_id) = map.get(&ps.pane_id) {
                        ps.pane_id = new_id;
                    }
                }
                HelpReturnState::ListFind(lf) => {
                    if let Some(&new_id) = map.get(&lf.pane_id) {
                        lf.pane_id = new_id;
                    }
                }
                HelpReturnState::GlobalSearch(gs) => {
                    if let Some(&new_id) = map.get(&gs.pane_id) {
                        gs.pane_id = new_id;
                    }
                }
                HelpReturnState::VisualSelection(vs) => {
                    if let Some(&new_id) = map.get(&vs.pane_id) {
                        vs.pane_id = new_id;
                    }
                }
                HelpReturnState::Pending(action) => {
                    remap_pending_action_pane_id(action, map);
                }
                HelpReturnState::CommandMode(_) | HelpReturnState::PendingBookmark(_) => {}
            }
        }

        // 重映射 pending_action
        if let Some(action) = &mut self.pending_action {
            remap_pending_action_pane_id(action, map);
        }

        // 重映射 task_log 中正在執行的任務之 pane_id
        for record in &mut self.task_log {
            if record.state == TaskState::Running {
                record.pane_id = map.get(&record.pane_id).copied().unwrap_or(record.pane_id);
            }
        }
    }

    /// 依照目前布局順序取得所有 pane id。
    pub(crate) fn ordered_pane_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        self.layout.pane_ids(&mut ids);
        ids
    }

    /// 將焦點直接切到指定 pane 編號。
    pub(crate) fn focus_pane_by_id(&mut self, target_pane_id: usize) {
        if !self.panes.contains_key(&target_pane_id) {
            self.status = format!(
                "unknown panel {target_pane_id}. available: {}",
                self.available_pane_ids_label()
            );
            return;
        }
        if self.focused_pane == target_pane_id {
            return;
        }
        self.focused_pane = target_pane_id;
        self.status = format!("focused panel {target_pane_id}");
    }

    /// 從 `:panel <id>` 的參數解析目標 panel 編號並切換焦點。
    pub(crate) fn focus_pane_by_id_argument(&mut self, target: &str) {
        let Some(target_pane_id) = parse_pane_id_argument(target) else {
            self.status = format!(
                "usage: panel <panel-id>. available: {}",
                self.available_pane_ids_label()
            );
            return;
        };
        self.focus_pane_by_id(target_pane_id);
    }

    /// 關閉目前有焦點的 pane。
    pub(crate) fn close_current_pane(&mut self) {
        let ids = self.ordered_pane_ids();
        if ids.len() <= 1 {
            self.status = String::from("cannot close the last panel");
            return;
        }

        let old_focus = self.focused_pane;
        if let Some(index) = ids.iter().position(|id| *id == old_focus) {
            let fallback = if index > 0 {
                ids[index - 1]
            } else {
                ids[index + 1]
            };
            if let Some(layout) = self.layout.clone().close_pane(old_focus) {
                self.layout = layout;
                self.cancel_directory_load(old_focus);
                self.cancel_directory_size_scan(old_focus);
                self.panes.remove(&old_focus);
                if self
                    .global_search
                    .as_ref()
                    .is_some_and(|search| search.pane_id == old_focus)
                {
                    self.global_search = None;
                }
                self.focused_pane = fallback;
                self.renumber_panes();
                self.status = format!("closed panel {old_focus}");
            }
        }
    }

    /// 僅保留目前有焦點的 pane，其餘全部關閉。
    pub(crate) fn only_current_pane(&mut self) {
        let focused = self.focused_pane;
        self.panes.retain(|id, _| *id == focused);
        self.layout = LayoutNode::Leaf { pane_id: focused };
        if self
            .global_search
            .as_ref()
            .is_some_and(|search| search.pane_id != focused)
        {
            self.global_search = None;
        }
        self.renumber_panes();
        self.status = String::from("kept only focused panel");
    }

    /// 均等重設所有分割視窗的尺寸。
    pub(crate) fn equalize_layout(&mut self) {
        self.layout.equalize();
        self.status = String::from("equalized all panels");
    }

    /// 調整目前焦點 panel 的寬度（欄數）。
    pub(crate) fn resize_focused_pane_width(&mut self, delta: i32) {
        let area = self.current_pane_area();
        match self
            .layout
            .resize_pane(self.focused_pane, SplitDirection::Vertical, delta, area)
        {
            Ok(()) => {
                self.status = format!(
                    "resized panel {} width ({:+} cols)",
                    self.focused_pane, delta
                );
            }
            Err(reason) => {
                self.status = format!("cannot resize panel {} width: {reason}", self.focused_pane);
            }
        }
    }

    /// 調整目前焦點 panel 的高度（列數）。
    pub(crate) fn resize_focused_pane_height(&mut self, delta: i32) {
        let area = self.current_pane_area();
        match self
            .layout
            .resize_pane(self.focused_pane, SplitDirection::Horizontal, delta, area)
        {
            Ok(()) => {
                self.status = format!(
                    "resized panel {} height ({:+} rows)",
                    self.focused_pane, delta
                );
            }
            Err(reason) => {
                self.status = format!("cannot resize panel {} height: {reason}", self.focused_pane);
            }
        }
    }

    /// 取得目前可用 panel 編號的格式化標籤（例如 1, 2, 3）。
    pub(crate) fn available_pane_ids_label(&self) -> String {
        self.ordered_pane_ids()
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(crate) fn remap_pending_action_pane_id(
    action: &mut PendingAction,
    map: &HashMap<usize, usize>,
) {
    match action {
        PendingAction::ConfirmDelete { pane_id, .. }
        | PendingAction::ConfirmPasteOverwrite { pane_id, .. }
        | PendingAction::SortPicker { pane_id }
        | PendingAction::GoPicker { pane_id }
        | PendingAction::WindowPicker { pane_id }
        | PendingAction::WindowResize { pane_id }
        | PendingAction::LineModePicker { pane_id }
        | PendingAction::YankPicker { pane_id }
        | PendingAction::ThemeCommandPicker { pane_id }
        | PendingAction::TrashPanel { pane_id, .. }
        | PendingAction::HelpPanel { pane_id, .. }
        | PendingAction::TaskPanel { pane_id, .. }
        | PendingAction::BookmarkPicker { pane_id }
        | PendingAction::BookmarkList { pane_id, .. }
        | PendingAction::ZoxideList { pane_id, .. }
        | PendingAction::ToolPanel { pane_id, .. }
        | PendingAction::CopyPicker { pane_id, .. }
        | PendingAction::OpenPicker { pane_id, .. }
        | PendingAction::Rename { pane_id, .. }
        | PendingAction::CreateEntry { pane_id, .. }
        | PendingAction::RegexRename { pane_id, .. }
        | PendingAction::EasyMotion { pane_id, .. } => {
            if let Some(&new_id) = map.get(pane_id) {
                *pane_id = new_id;
            }
        }
        PendingAction::ConfirmTrashAction { action, .. } => match action {
            TrashConfirmAction::RestoreFromPanel { pane_id, .. }
            | TrashConfirmAction::DeleteFromPanel { pane_id, .. } => {
                if let Some(&new_id) = map.get(pane_id) {
                    *pane_id = new_id;
                }
            }
        },
        PendingAction::DiffMatrix(state) => {
            for pid in &mut state.panel_ids {
                if let Some(&new_id) = map.get(pid) {
                    *pid = new_id;
                }
            }
        }
        PendingAction::ThemePicker { .. } => {}
    }
}
