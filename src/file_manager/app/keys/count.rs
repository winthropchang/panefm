use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};

use super::super::*;

impl App {
    /// 嘗試把目前按鍵視為 count prefix 的下一個數字。
    ///
    /// 規則：
    /// - `1..=9` 永遠可以開始或延續 count。
    /// - `0` 只有在已經有 count 時，才會被視為後續位數。
    pub(crate) fn capture_pending_count_digit(&mut self, key: &KeyEvent) -> bool {
        if !key.modifiers.is_empty() {
            return false;
        }

        let KeyCode::Char(ch) = key.code else {
            return false;
        };

        if !ch.is_ascii_digit() {
            return false;
        }
        if ch == '0' && self.pending_count.is_none() {
            return false;
        }

        let digit = ch.to_digit(10).unwrap_or(0) as usize;
        let next = self
            .pending_count
            .unwrap_or(0)
            .saturating_mul(10)
            .saturating_add(digit);
        self.pending_count = Some(next);
        self.status = format!("count: {next}");
        true
    }

    /// 取出目前暫存的 count prefix。
    pub(crate) fn take_pending_count(&mut self) -> Option<usize> {
        self.pending_count.take()
    }

    /// 取出目前暫存的 count；若沒有則回傳 1。
    pub(crate) fn take_count_or_one(&mut self) -> usize {
        self.take_pending_count().unwrap_or(1).max(1)
    }

    /// 取出目前 count，並轉成固定大步長移動的實際步數。
    pub(crate) fn take_large_move_step(&mut self) -> usize {
        self.take_count_or_one()
            .saturating_mul(self.config.navigation.fast_move_step.max(1))
    }

    /// 取出目前 count，並轉成一般彈窗列表使用的 page 步長。
    pub(crate) fn take_panel_page_step(&mut self) -> usize {
        self.take_count_or_one()
            .saturating_mul(self.config.navigation.panel_page_step.max(1))
    }

    /// 清除目前暫存的 count prefix。
    pub(crate) fn clear_pending_count(&mut self) {
        self.pending_count = None;
    }

    /// 取出目前垂直導航步長（支援智能長按加速）。
    /// 若有手動輸入 count prefix（如 `5j`），則以手動 count 為優先並重置加速；
    /// 若為無前綴的連續同方向按鍵（間隔 <= 150ms），則隨連按次數智能遞增步長。
    pub(crate) fn take_vertical_nav_step(&mut self, direction: NavDirection) -> usize {
        if let Some(count) = self.take_pending_count() {
            self.nav_acceleration = None;
            return count.max(1);
        }

        if !self.config.navigation.scroll_acceleration {
            return 1;
        }

        let now = Instant::now();
        // 連發重複間隔閾值：macOS / Linux / Windows 鍵盤連發頻率通常在 15ms ~ 100ms 之間，
        // 設為 150ms 可精準捕獲各平台的長按連發與高頻連按。
        const REPEAT_THRESHOLD: Duration = Duration::from_millis(150);

        let count = match self.nav_acceleration {
            Some(acc)
                if acc.direction == direction
                    && now.duration_since(acc.last_press) <= REPEAT_THRESHOLD =>
            {
                acc.repeat_count.saturating_add(1)
            }
            _ => 1,
        };

        self.nav_acceleration = Some(NavAcceleration {
            direction,
            last_press: now,
            repeat_count: count,
        });

        // 平滑階梯式加速：
        // 第 1 ~ 3 次（約前 250ms）：1 格（精確單格微調）
        // 第 4 ~ 7 次（約 250ms ~ 500ms）：2 格（平滑雙倍速起步）
        // 第 8 次以上（> 500ms）：3 格（巡航極速穿梭）
        if count <= 3 {
            1
        } else if count <= 7 {
            2
        } else {
            3
        }
    }

    /// 清除和一般移動相關的暫存狀態，例如 count、pending g、pending y 與導航加速。
    pub(crate) fn reset_pending_motion_state(&mut self) {
        self.clear_pending_count();
        self.pending_g = false;
        self.pending_y = false;
        self.nav_acceleration = None;
    }
}
