//! 主題設定與色盤映射整合測試。
//!
//! 依據 `DEVELOPMENT_GUIDELINES.md` 規範，黑箱整合測試獨立放置於 `tests/` 目錄，
//! 驗證：
//! 1. 內建主題色盤彼此獨立。
//! 2. 主題名稱與預設值雙向對應。
//! 3. 舊版主題設定別名向後相容。

use panefm::theme::{Theme, ThemePreset};

#[test]
/// 驗證內建主題的色盤內容彼此不同。
///
/// 參數：無。
/// 回傳：無；若主題內容相同則測試失敗。
/// 保護目的：避免新增或映射主題時，造成設定名稱、色盤與 UI 語意顏色彼此不一致。
fn built_in_themes_are_distinct() {
    assert_ne!(Theme::default_theme(), Theme::forest_theme());
    assert_ne!(Theme::default_theme(), Theme::ocean_theme());
}

#[test]
/// 驗證主題名稱與主題預設值之間可以雙向對應。
///
/// 參數：無。
/// 回傳：無；若名稱無法正確解析則測試失敗。
/// 保護目的：避免新增或映射主題時，造成設定名稱、色盤與 UI 語意顏色彼此不一致。
fn preset_name_round_trip_works() {
    for preset in ThemePreset::ALL {
        assert_eq!(ThemePreset::from_name(preset.name()), Some(preset));
    }
}

#[test]
/// 驗證舊版設定名稱仍能對應到新的成熟主題，避免更新後既有設定失效。
/// 保護目的：避免新增或映射主題時，造成設定名稱、色盤與 UI 語意顏色彼此不一致。
fn legacy_theme_names_remain_compatible() {
    assert_eq!(
        ThemePreset::from_name("default"),
        Some(ThemePreset::CatppuccinMocha)
    );
    assert_eq!(
        ThemePreset::from_name("forest"),
        Some(ThemePreset::Everforest)
    );
    assert_eq!(ThemePreset::from_name("ocean"), Some(ThemePreset::Nord));
}

#[test]
/// 驗證所有主題的焦點與非焦點 Pane 膠囊徽章均具備粗體設定與清晰底色。
/// 保護目的：避免更換或調整主題色盤後，造成視窗編號無法凸顯或文字對比度不足。
fn theme_pane_badge_style_has_high_contrast() {
    use ratatui::style::Modifier;

    for preset in ThemePreset::ALL {
        let theme = Theme::from(preset);
        let focused_badge = theme.pane_badge_style(true);
        let unfocused_badge = theme.pane_badge_style(false);

        assert!(
            focused_badge.add_modifier.contains(Modifier::BOLD),
            "focused badge must be bold for {:?}",
            preset
        );
        assert!(
            unfocused_badge.add_modifier.contains(Modifier::BOLD),
            "unfocused badge must be bold for {:?}",
            preset
        );
        assert!(focused_badge.bg.is_some());
        assert!(unfocused_badge.bg.is_some());
        assert!(focused_badge.fg.is_some());
        assert!(unfocused_badge.fg.is_some());
    }
}
