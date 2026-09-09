use std::borrow::Cow;

use super::{fuzzy_matched_indices, fuzzy_matched_indices_by_fields};

#[test]
/// 驗證不連續字元也能命中，並把較貼近查詢的候選排在前面。
/// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
fn fuzzy_filter_matches_non_contiguous_characters_and_ranks_results() {
    let items = ["src/file_manager/app.rs", "archive.zip", "sample.txt"];
    let matched = fuzzy_matched_indices(&items, "sfma", |item| Cow::Borrowed(*item));

    assert_eq!(matched, vec![0]);
}

#[test]
/// 驗證空查詢不會改變候選順序，避免剛打開 filter 時列表突然重排。
/// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
fn fuzzy_filter_keeps_original_order_for_empty_query() {
    let items = ["beta", "alpha", "gamma"];
    let matched = fuzzy_matched_indices(&items, "", |item| Cow::Borrowed(*item));

    assert_eq!(matched, vec![0, 1, 2]);
}

#[test]
/// 驗證 smart-case：小寫查詢可忽略大小寫，大寫查詢則尊重大小寫。
/// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
fn fuzzy_filter_uses_smart_case_matching() {
    let items = ["README.md", "readme-copy.md"];

    assert_eq!(
        fuzzy_matched_indices(&items, "read", |item| Cow::Borrowed(*item)),
        vec![0, 1]
    );
    assert_eq!(
        fuzzy_matched_indices(&items, "READ", |item| Cow::Borrowed(*item)),
        vec![0]
    );
}

#[test]
/// 驗證多欄位資料不會把查詢字元跨欄位串接，避免產生肉眼無法理解的假命中。
/// 保護目的：避免模糊比對演算法調整後，出現跨欄位假命中、排序跳動或 smart-case 退化。
fn fuzzy_filter_does_not_join_characters_across_fields() {
    let items = [("alp", "ha.txt"), ("alpha.txt", "/tmp")];
    let matched = fuzzy_matched_indices_by_fields(&items, "alpha", |item| [item.0, item.1]);

    assert_eq!(matched, vec![1]);
}
