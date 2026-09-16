use std::path::{Path, PathBuf};

use crate::file_manager::ui::RegexRenamePanelLine;

/// 描述 regex 批次改名預覽中每一列的運算結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegexRenamePreview {
    pub(crate) source_path: PathBuf,
    pub(crate) original_name: String,
    pub(crate) new_name: String,
    pub(crate) outcome: RegexRenameOutcome,
}

/// 表示 regex 批次改名預覽中這一列目前是可套用、無變化或有衝突。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegexRenameOutcome {
    Ready,
    Unchanged,
    Conflict,
    Invalid,
}

/// 判斷 regex 批次改名某一列目前屬於可改名、無變化還是無效名稱。
pub(crate) fn classify_regex_rename_preview(
    original_name: &str,
    new_name: &str,
) -> RegexRenameOutcome {
    if new_name == original_name {
        return RegexRenameOutcome::Unchanged;
    }
    if new_name.is_empty()
        || new_name == "."
        || new_name == ".."
        || new_name.contains('/')
        || new_name.contains('\\')
    {
        return RegexRenameOutcome::Invalid;
    }
    RegexRenameOutcome::Ready
}

/// 將 regex 批次改名預覽轉成 pane 可直接顯示的列表內容。
pub(crate) fn regex_rename_panel_lines(
    previews: &[RegexRenamePreview],
) -> Vec<RegexRenamePanelLine> {
    previews
        .iter()
        .map(|preview| RegexRenamePanelLine {
            original_name: preview.original_name.clone(),
            new_name: preview.new_name.clone(),
            status: match preview.outcome {
                RegexRenameOutcome::Ready => String::from("ready"),
                RegexRenameOutcome::Unchanged => String::from("unchanged"),
                RegexRenameOutcome::Conflict => String::from("conflict"),
                RegexRenameOutcome::Invalid => String::from("invalid"),
            },
        })
        .collect()
}

/// 根據目前 preview 內容整理 regex 批次改名面板的狀態列文字。
pub(crate) fn regex_rename_status(
    pattern: &str,
    replacement: &str,
    previews: &[RegexRenamePreview],
) -> String {
    let ready = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Ready))
        .count();
    let unchanged = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Unchanged))
        .count();
    let conflicts = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Conflict))
        .count();
    let invalid = previews
        .iter()
        .filter(|preview| matches!(preview.outcome, RegexRenameOutcome::Invalid))
        .count();
    format!(
        "rename-regex /{pattern}/ -> {replacement}  [ready:{ready} unchanged:{unchanged} conflict:{conflicts} invalid:{invalid}]"
    )
}

/// 產生一個不會和當前批次改名結果衝突的暫存路徑，供兩階段 rename 使用。
pub(crate) fn unique_regex_rename_temp_path(
    cwd: &Path,
    original_name: &str,
    index: usize,
    previews: &[RegexRenamePreview],
) -> PathBuf {
    let mut attempt = 0usize;
    loop {
        let candidate = cwd.join(format!(
            ".tfm-rename-regex-{index}-{attempt}-{original_name}"
        ));
        let used_as_target = previews
            .iter()
            .any(|preview| cwd.join(&preview.new_name) == candidate);
        if !candidate.exists() && !used_as_target {
            return candidate;
        }
        attempt += 1;
    }
}
