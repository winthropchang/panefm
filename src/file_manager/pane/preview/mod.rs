pub(crate) mod content;
pub(crate) mod preheat;
pub(crate) mod scroll;
pub(crate) mod search;
pub(crate) mod title;

use super::PaneState;

impl PaneState {
    pub(crate) fn is_preview_open(&self) -> bool {
        self.preview_open
    }

    /// 明確設定目前 panel 的雙欄即時預覽開啟狀態。
    pub(crate) fn set_preview_open(&mut self, open: bool) {
        self.preview_open = open;
        if !open {
            self.preview_focused = false;
        }
        self.preview_active = self.preview_open && self.preview_focused;
    }

    /// 切換目前 panel 的雙欄即時預覽開啟狀態。
    pub(crate) fn toggle_preview_open(&mut self) -> bool {
        self.preview_open = !self.preview_open;
        if self.preview_open {
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_diff_mode = false;
            self.preview_search_query = None;
            self.preview_current_match = None;
            self.preview_focused = true;
        } else {
            self.preview_focused = false;
            self.preview_diff_mode = false;
        }
        self.preview_active = self.preview_open && self.preview_focused;
        self.preview_open
    }

    /// 切換預覽的 VCS Diff 差異模式。
    /// 若預覽未開啟，則同時開啟預覽並切換至 diff 模式。
    /// 若預覽已開啟且處於 diff 模式，則切回全文模式。
    /// 若預覽已開啟但處於全文模式，則切換至 diff 模式。
    /// 回傳切換後是否處於 diff 模式。
    pub(crate) fn toggle_preview_diff_mode(&mut self) -> bool {
        if !self.preview_open {
            self.preview_open = true;
            self.preview_diff_mode = true;
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_search_query = None;
            self.preview_current_match = None;
            self.preview_focused = true;
        } else {
            self.preview_diff_mode = !self.preview_diff_mode;
            self.preview_scroll = 0;
            self.preview_cursor = 0;
            self.preview_search_query = None;
            self.preview_current_match = None;
            self.preview_focused = true;
        }
        self.preview_active = self.preview_open && self.preview_focused;
        self.preview_diff_mode
    }

    /// 判斷當前焦點是否正處於右側預覽（可進行滾動、跳頁與預覽搜尋）。
    pub(crate) fn is_preview_focused(&self) -> bool {
        self.preview_open && self.preview_focused
    }

    /// 設定焦點是否切入右側預覽視窗。
    pub(crate) fn set_preview_focused(&mut self, focused: bool) {
        if self.preview_open {
            self.preview_focused = focused;
        } else {
            self.preview_focused = false;
        }
        self.preview_active = self.preview_open && self.preview_focused;
    }

    /// 判斷目前 panel 是否正在接收預覽操作按鍵（等同於 is_preview_focused）。
    pub(crate) fn is_preview_active(&self) -> bool {
        self.preview_open && self.preview_focused
    }

    /// 明確設定目前 panel 的 preview 操作狀態。
    pub(crate) fn set_preview_active(&mut self, active: bool) {
        self.preview_open = active;
        self.preview_focused = active;
        self.preview_active = active;
    }

    /// 切換目前 panel 的 preview 顯示狀態，並回傳切換後的結果。
    #[allow(dead_code)]
    pub(crate) fn toggle_preview_active(&mut self) -> bool {
        self.toggle_preview_open()
    }
}
