//! 說明項目的關鍵字與模糊搜尋過濾演算法。

use super::super::*;
use super::types::HelpEntry;

/// 依照關鍵字過濾自訂的 HelpEntry 清單（如 Cheatsheet）。
pub(crate) fn filter_custom_help_entries(entries: &[HelpEntry], query: &str) -> Vec<HelpEntry> {
    if query.trim().is_empty() {
        return entries.to_vec();
    }
    fuzzy_matched_indices_by_fields(entries, query, |entry| {
        vec![
            entry.line.command.clone(),
            entry.line.shortcut.clone(),
            entry.line.description.clone(),
        ]
    })
    .into_iter()
    .map(|index| entries[index].clone())
    .collect()
}
