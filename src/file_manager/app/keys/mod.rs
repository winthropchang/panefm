#![allow(unused_imports)]

use super::*;

mod bookmarks;
mod command;
mod count;
mod create;
mod dialogs;
mod diff;
mod easymotion;
mod help;
mod normal_nav;
mod normal_ops;
mod paste;
mod pending;
mod pickers;
mod preview;
mod rename;
mod search;
mod tasks;
mod trash;
mod visual;

pub(crate) use bookmarks::*;
pub(crate) use command::*;
pub(crate) use count::*;
pub(crate) use create::*;
pub(crate) use dialogs::*;
pub(crate) use diff::*;
pub(crate) use easymotion::*;
pub(crate) use help::*;
pub(crate) use normal_nav::*;
pub(crate) use normal_ops::*;
pub(crate) use paste::*;
pub(crate) use pending::*;
pub(crate) use pickers::*;
pub(crate) use preview::*;
pub(crate) use rename::*;
pub(crate) use search::*;
pub(crate) use tasks::*;
pub(crate) use trash::*;
pub(crate) use visual::*;

impl App {
    /// 依照目前互動狀態，把一個鍵盤事件分派給唯一的處理流程。
    ///
    /// 參數：`key: KeyEvent`，已由終端事件迴圈過濾成 Press/Repeat 的按鍵。
    /// 回傳：`Result<bool>`；`true` 代表繼續執行，`false` 只由 quit 流程回傳，錯誤
    /// 則向上交給事件迴圈統一還原 terminal。
    ///
    /// 分派順序不可隨意調換：暫時面板與文字輸入必須先攔截按鍵，否則使用者在
    /// command 輸入 `d` 時可能同時觸發刪除；一般列表快捷鍵永遠是最後一層。
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // Help 被允許從任何上下文打開（F1 永遠支援；~ 僅在非文字輸入狀態下生效，
        // 避免在 filter、goto 或 rename 中輸入 ~ 被誤當成開啟說明）；
        // `help_return` 會保存原上下文，關閉說明後才能回到原 panel/輸入流程。
        if key.code == KeyCode::F(1) || (key_matches_tilde(&key) && !self.is_text_input_active()) {
            if matches!(self.pending_action, Some(PendingAction::HelpPanel { .. })) {
                return self.handle_pending_action_key(key);
            }
            self.open_help_from_current();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if key_matches_question_mark(&key) && !self.is_text_input_active() {
            if matches!(self.pending_action, Some(PendingAction::HelpPanel { .. })) {
                return self.handle_pending_action_key(key);
            }
            self.open_cheatsheet_from_current();
            self.pending_g = false;
            self.pending_y = false;
            return Ok(true);
        }
        if self.pending_action.is_some() {
            return self.handle_pending_action_key(key);
        }
        // 下列輸入狀態互斥，依 UI 層級交給各自 handler；handler 內再共用
        // `edit_text_buffer`，以維持 Insert/Normal、Unicode 游標與 Esc 行為一致。
        if self.filter.as_ref().is_some_and(|filter| filter.editing) {
            return self.handle_filter_input_key(key);
        }
        if self
            .preview_search
            .as_ref()
            .is_some_and(|search| search.editing)
        {
            return self.handle_preview_search_input_key(key);
        }
        if self.list_find.is_some() {
            return self.handle_list_find_input_key(key);
        }
        if self.global_search.is_some() {
            return self.handle_global_search_key(key);
        }
        if self.visual_selection.is_some() {
            return self.handle_visual_selection_key(key);
        }
        if self.command_mode {
            return self.handle_command_key(key);
        }
        if self.pending_bookmark.is_some() {
            return self.handle_bookmark_key(key);
        }
        // UNC 目錄可能被 Windows 網路層長時間阻塞。背景跳轉期間允許 Esc 立即
        // 捨棄接收端並回到一般操作，不必等待作業系統的檔案系統呼叫結束。
        if key.code == KeyCode::Esc && self.active_network_goto_task_id.is_some() {
            self.cancel_network_goto("cancelled by user");
            return Ok(true);
        }
        // 不在輸入或暫時 UI 時，數字與跨 panel 快捷鍵才有意義。這段必須放在
        // count prefix 之前，否則多 panel 下按 2 會被誤解成 `2j` 的前綴。
        if self.panes.len() > 1
            && let Some(target_pane_id) = plain_digit_target_pane_id(&key)
        {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.focus_pane_by_id(target_pane_id);
            return Ok(true);
        }
        if let Some(target_pane_id) = ctrl_digit_target_pane_id(&key) {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.focus_pane_by_id(target_pane_id);
            return Ok(true);
        }
        if key_matches_ctrl_letter(&key, 's') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.split_current(SplitDirection::Horizontal)?;
            return Ok(true);
        }
        if key_matches_ctrl_letter(&key, 'v') {
            self.clear_pending_count();
            self.pending_g = false;
            self.pending_y = false;
            self.split_current(SplitDirection::Vertical)?;
            return Ok(true);
        }
        if self
            .panes
            .get(&self.focused_pane)
            .is_some_and(PaneState::is_preview_active)
        {
            return self.handle_preview_key(key);
        }
        if self.capture_pending_count_digit(&key) {
            return Ok(true);
        }

        if let Some(res) = self.handle_normal_nav_key(&key)? {
            return Ok(res);
        }

        if let Some(res) = self.handle_normal_ops_key(&key)? {
            return Ok(res);
        }

        self.reset_pending_motion_state();
        self.pending_bookmark = None;
        Ok(true)
    }
}
