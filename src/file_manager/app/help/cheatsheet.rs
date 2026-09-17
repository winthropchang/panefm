//! 情境速查表（Context Cheatsheet）統一調度模組。

use super::cheatsheet_modes::cheatsheet_for_mode;
use super::cheatsheet_panels::cheatsheet_for_panel;
use super::cheatsheet_pickers::cheatsheet_for_picker;
use super::types::{ContextHelpKind, HelpEntry};

/// 依情境種類產出對應的 Cheatsheet 標題與專屬功能快捷鍵清單。
pub(crate) fn context_cheatsheet_entries(kind: ContextHelpKind) -> (String, Vec<HelpEntry>) {
    if let Some(entries) = cheatsheet_for_mode(kind) {
        return entries;
    }
    if let Some(entries) = cheatsheet_for_panel(kind) {
        return entries;
    }
    if let Some(entries) = cheatsheet_for_picker(kind) {
        return entries;
    }
    // 若未能命中特定情境，預設退回 Normal 模式的速查表
    cheatsheet_for_mode(ContextHelpKind::Normal)
        .unwrap_or_else(|| (String::from("Cheatsheet: Normal Mode"), Vec::new()))
}
