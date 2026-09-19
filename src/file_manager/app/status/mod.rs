//! 狀態列（Status Bar）顯示、文字折疊、錯誤判定與情境快捷鍵提示模組。

pub(crate) mod action_hints;
pub(crate) mod formatters;
pub(crate) mod hints;
pub(crate) mod picker_hints;

/// 描述底部快捷鍵列中的一組高頻操作提示。
///
/// 欄位：
/// - `key: &'static str`，實際要按下的快捷鍵。
/// - `label: &'static str`，簡短的英文功能名稱。
///
/// 清單本身的順序就是顯示優先度；畫面不足時只會捨棄尾端低優先項目，避免把
/// `~/F1 help` 等重要入口裁掉，或顯示只剩一半的快捷鍵說明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusShortcutHint {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
}

pub(crate) use formatters::*;
pub(crate) use hints::*;
