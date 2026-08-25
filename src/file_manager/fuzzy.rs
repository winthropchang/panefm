//! 所有 panel filter 共用的模糊匹配與穩定排序策略。
//!
//! 搜尋工具決定「取得哪些候選」，本模組只在既有候選中做互動式縮小範圍。欄位
//! 彼此分開匹配，避免查詢字元跨過名稱與路徑邊界形成令人困惑的假命中。

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
    F: Fn(&T) -> String,
{
    fuzzy_matched_indices_by_fields(items, query, |item| vec![text(item)])
}

/// 依多個獨立欄位做模糊過濾，避免查詢字元跨越欄位邊界形成不合理命中。
///
/// 參數：
/// - `items: &[T]`，尚未過濾的原始候選資料。
/// - `query: &str`，使用者輸入的模糊查詢。
/// - `fields: F`，回傳單一候選可搜尋欄位的函數，例如檔名與原始路徑可分成兩欄。
///
/// 回傳：`Vec<usize>`，依候選各欄位中的最高分排序；同分時維持原始順序。
pub(crate) fn fuzzy_matched_indices_by_fields<T, F>(
    items: &[T],
    query: &str,
    fields: F,
) -> Vec<usize>
where
    F: Fn(&T) -> Vec<String>,
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
                    pattern.score(Utf32Str::new(&field, &mut utf32_buffer), &mut matcher)
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
mod tests {
    use super::{fuzzy_matched_indices, fuzzy_matched_indices_by_fields};

    #[test]
    /// 驗證不連續字元也能命中，並把較貼近查詢的候選排在前面。
    /// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
    fn fuzzy_filter_matches_non_contiguous_characters_and_ranks_results() {
        let items = ["src/file_manager/app.rs", "archive.zip", "sample.txt"];
        let matched = fuzzy_matched_indices(&items, "sfma", |item| (*item).to_string());

        assert_eq!(matched, vec![0]);
    }

    #[test]
    /// 驗證空查詢不會改變候選順序，避免剛打開 filter 時列表突然重排。
    /// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
    fn fuzzy_filter_keeps_original_order_for_empty_query() {
        let items = ["beta", "alpha", "gamma"];
        let matched = fuzzy_matched_indices(&items, "", |item| (*item).to_string());

        assert_eq!(matched, vec![0, 1, 2]);
    }

    #[test]
    /// 驗證 smart-case：小寫查詢可忽略大小寫，大寫查詢則尊重大小寫。
    /// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
    fn fuzzy_filter_uses_smart_case_matching() {
        let items = ["README.md", "readme-copy.md"];

        assert_eq!(
            fuzzy_matched_indices(&items, "read", |item| (*item).to_string()),
            vec![0, 1]
        );
        assert_eq!(
            fuzzy_matched_indices(&items, "READ", |item| (*item).to_string()),
            vec![0]
        );
    }

    #[test]
    /// 驗證多欄位資料不會把查詢字元跨欄位串接，避免產生肉眼無法理解的假命中。
    /// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
    fn fuzzy_filter_does_not_join_characters_across_fields() {
        let items = [("alp", "ha.txt"), ("alpha.txt", "/tmp")];
        let matched = fuzzy_matched_indices_by_fields(&items, "alpha", |item| {
            vec![item.0.to_string(), item.1.to_string()]
        });

        assert_eq!(matched, vec![1]);
    }
}
