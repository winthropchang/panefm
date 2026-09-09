//! 所有 panel filter 共用的模糊匹配與穩定排序策略。
//!
//! 搜尋工具決定「取得哪些候選」，本模組只在既有候選中做互動式縮小範圍。欄位
//! 彼此分開匹配，避免查詢字元跨過名稱與路徑邊界形成令人困惑的假命中。

use std::borrow::Cow;

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// 依照模糊查詢過濾並排序候選項目的索引，供所有 `f` filter 介面共用。
///
/// 參數：
/// - `items: &[T]`，尚未過濾的原始候選資料。
/// - `query: &str`，使用者輸入的模糊查詢；空字串會保留原始順序與全部項目。
/// - `text: F`，把單一候選 `&T` 轉成可搜尋文字的函數。
///
/// 回傳：`Vec<usize>`。
/// - 每個值都是候選項目在 `items` 中的原始索引。
/// - 有查詢時依匹配分數由高到低排列；同分時維持原始順序，避免游標畫面跳動。
pub(crate) fn fuzzy_matched_indices<T, F>(items: &[T], query: &str, text: F) -> Vec<usize>
where
    F: for<'a> Fn(&'a T) -> Cow<'a, str>,
{
    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).collect();
    }

    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut utf32_buffer = Vec::new();
    let mut matches: Vec<(usize, u32)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let field = text(item);
            pattern
                .score(
                    Utf32Str::new(field.as_ref(), &mut utf32_buffer),
                    &mut matcher,
                )
                .map(|score| (index, score))
        })
        .collect();
    matches.sort_by(|(left_index, left_score), (right_index, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_index.cmp(right_index))
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

/// 依多個獨立欄位做模糊過濾，避免查詢字元跨越欄位邊界形成不合理命中。
///
/// 參數：
/// - `items: &[T]`，尚未過濾的原始候選資料。
/// - `query: &str`，使用者輸入的模糊查詢。
/// - `fields: F`，回傳單一候選可搜尋欄位的函數，例如檔名與原始路徑可分成兩欄。
///
/// 回傳：`Vec<usize>`，依候選各欄位中的最高分排序；同分時維持原始順序。
pub(crate) fn fuzzy_matched_indices_by_fields<T, I, F>(
    items: &[T],
    query: &str,
    fields: F,
) -> Vec<usize>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
    F: Fn(&T) -> I,
{
    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).collect();
    }

    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut utf32_buffer = Vec::new();
    let mut matches: Vec<(usize, u32)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            fields(item)
                .into_iter()
                .filter_map(|field| {
                    pattern.score(
                        Utf32Str::new(field.as_ref(), &mut utf32_buffer),
                        &mut matcher,
                    )
                })
                .max()
                .map(|score| (index, score))
        })
        .collect();
    matches.sort_by(|(left_index, left_score), (right_index, right_score)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_index.cmp(right_index))
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
#[path = "tests/fuzzy_test.rs"]
mod tests;
