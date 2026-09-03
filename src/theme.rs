//! 主題名稱、成熟色盤轉換與 PaneFM 語意顏色定義。
//!
//! UI 不應直接寫死 RGB 顏色，而要使用 [`Theme`] 提供的 selected、danger、search
//! match 等語意欄位。如此切換線上成熟主題時，列表、狀態訊息與搜尋高亮才能一起
//! 改變；新增 UI 狀態時也應先在這裡定義顏色用途。

use ratatui::style::{Color, Modifier, Style};
use ratatui_themes::ThemeName as PaletteThemeName;

/// 表示可被使用者選擇的主題預設名稱。
///
/// 這個列舉不直接保存顏色，而是作為主題系統的穩定識別值，
/// 讓命令模式、設定檔與主題選擇視窗都能用相同名稱操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePreset {
    Dracula,
    OneDarkPro,
    Nord,
    CatppuccinMocha,
    CatppuccinLatte,
    GruvboxDark,
    GruvboxLight,
    TokyoNight,
    SolarizedDark,
    SolarizedLight,
    MonokaiPro,
    RosePine,
    Kanagawa,
    Everforest,
    Cyberpunk,
    MidnightCommander,
}

impl ThemePreset {
    /// 內建所有主題預設值的固定清單。
    ///
    /// 參數：無。
    /// 回傳：`[ThemePreset; 16]`，依序列出所有可用主題。
    pub const ALL: [ThemePreset; 16] = [
        ThemePreset::Dracula,
        ThemePreset::OneDarkPro,
        ThemePreset::Nord,
        ThemePreset::CatppuccinMocha,
        ThemePreset::CatppuccinLatte,
        ThemePreset::GruvboxDark,
        ThemePreset::GruvboxLight,
        ThemePreset::TokyoNight,
        ThemePreset::SolarizedDark,
        ThemePreset::SolarizedLight,
        ThemePreset::MonokaiPro,
        ThemePreset::RosePine,
        ThemePreset::Kanagawa,
        ThemePreset::Everforest,
        ThemePreset::Cyberpunk,
        ThemePreset::MidnightCommander,
    ];

    /// 取得主題預設值對應的字串名稱。
    ///
    /// 參數：
    /// - `self: ThemePreset`，目前的主題預設值。
    ///
    /// 回傳：`&'static str`，可用於命令模式或設定檔的名稱。
    pub fn name(self) -> &'static str {
        PaletteThemeName::from(self).slug()
    }

    /// 依照字串名稱嘗試解析主題預設值。
    ///
    /// 參數：
    /// - `name: &str`，來自設定檔或命令輸入的主題名稱。
    ///
    /// 回傳：`Option<ThemePreset>`。
    /// - `Some(...)` 代表成功對應到主題。
    /// - `None` 代表輸入名稱不在支援清單內。
    pub fn from_name(name: &str) -> Option<Self> {
        let normalized = name.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "default" => Some(Self::CatppuccinMocha),
            "forest" => Some(Self::Everforest),
            "ocean" => Some(Self::Nord),
            _ => normalized.parse::<PaletteThemeName>().ok().map(Self::from),
        }
    }

    /// 取得下一個主題預設值，供輪替主題時使用。
    ///
    /// 參數：
    /// - `self: ThemePreset`，目前的主題預設值。
    ///
    /// 回傳：`ThemePreset`，依照固定順序切換後的主題。
    pub const fn next(self) -> Self {
        let index = match self {
            ThemePreset::Dracula => 0,
            ThemePreset::OneDarkPro => 1,
            ThemePreset::Nord => 2,
            ThemePreset::CatppuccinMocha => 3,
            ThemePreset::CatppuccinLatte => 4,
            ThemePreset::GruvboxDark => 5,
            ThemePreset::GruvboxLight => 6,
            ThemePreset::TokyoNight => 7,
            ThemePreset::SolarizedDark => 8,
            ThemePreset::SolarizedLight => 9,
            ThemePreset::MonokaiPro => 10,
            ThemePreset::RosePine => 11,
            ThemePreset::Kanagawa => 12,
            ThemePreset::Everforest => 13,
            ThemePreset::Cyberpunk => 14,
            ThemePreset::MidnightCommander => 15,
        };
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

/// 表示真正提供給 UI 渲染層使用的完整色彩主題。
///
/// 每個欄位都使用語意化名稱，避免在畫面程式裡直接散落具體顏色常數，
/// 讓日後調整主題、切換色盤或從設定檔覆蓋時更容易維護。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub focus_border: Color,
    pub muted: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub preview_match_bg: Color,
    pub preview_match_fg: Color,
    pub preview_current_line_bg: Color,
    pub preview_current_line_fg: Color,
    pub danger: Color,
    pub directory: Color,
    pub executable: Color,
    pub image: Color,
    pub archive: Color,
    pub source: Color,
}

impl Theme {
    /// 建立一個新的主題色盤。
    ///
    /// 參數：
    /// - `accent: Color`，操作提示與強調資訊的主要顏色。
    /// - `focus_border: Color`，目前焦點 pane 的邊框顏色。
    /// - `muted: Color`，次要資訊文字顏色。
    /// - `selection_bg: Color`，列表目前選取項目的背景色。
    /// - `danger: Color`，危險操作或刪除提示使用的顏色。
    ///
    /// 回傳：`Theme`，可直接套用於 UI 渲染。
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        accent: Color,
        focus_border: Color,
        muted: Color,
        selection_bg: Color,
        selection_fg: Color,
        preview_match_bg: Color,
        preview_match_fg: Color,
        preview_current_line_bg: Color,
        preview_current_line_fg: Color,
        danger: Color,
        directory: Color,
        executable: Color,
        image: Color,
        archive: Color,
        source: Color,
    ) -> Self {
        Self {
            accent,
            focus_border,
            muted,
            selection_bg,
            selection_fg,
            preview_match_bg,
            preview_match_fg,
            preview_current_line_bg,
            preview_current_line_fg,
            danger,
            directory,
            executable,
            image,
            archive,
            source,
        }
    }

    /// 建立預設主題。
    ///
    /// 參數：無。
    /// 回傳：`Theme`，作為程式啟動時的預設色盤。
    pub fn default_theme() -> Self {
        Self::from(ThemePreset::CatppuccinMocha)
    }

    /// 建立偏綠色調的森林主題。
    ///
    /// 參數：無。
    /// 回傳：`Theme`，適合較自然風格的配色。
    pub fn forest_theme() -> Self {
        Self::from(ThemePreset::Everforest)
    }

    /// 建立偏藍色調的海洋主題。
    ///
    /// 參數：無。
    /// 回傳：`Theme`，適合較冷色與清爽風格的配色。
    pub fn ocean_theme() -> Self {
        Self::from(ThemePreset::Nord)
    }

    /// 產生強調色對應的文字樣式。
    ///
    /// 參數：
    /// - `self: Theme`，目前主題。
    ///
    /// 回傳：`Style`，可用於操作提示或高亮文字。
    pub fn accent_style(self) -> Style {
        Style::default().fg(self.accent)
    }

    /// 產生目前焦點 pane 邊框的樣式。
    ///
    /// 參數：
    /// - `self: Theme`，目前主題。
    ///
    /// 回傳：`Style`，包含邊框顏色與粗體設定。
    pub fn focused_border_style(self) -> Style {
        Style::default()
            .fg(self.focus_border)
            .add_modifier(Modifier::BOLD)
    }

    /// 產生次要資訊使用的文字樣式。
    ///
    /// 參數：
    /// - `self: Theme`，目前主題。
    ///
    /// 回傳：`Style`，通常會套用在檔案大小或輔助資訊上。
    pub fn muted_style(self) -> Style {
        Style::default().fg(self.muted)
    }

    /// 產生列表選取項目的背景樣式。
    ///
    /// 參數：
    /// - `self: Theme`，目前主題。
    ///
    /// 回傳：`Style`，用來表示目前游標所在的項目。
    pub fn selected_item_style(self) -> Style {
        Style::default().bg(self.selection_bg).fg(self.selection_fg)
    }

    /// 產生 preview 中一般命中文字的樣式。
    pub fn preview_match_style(self) -> Style {
        Style::default().fg(self.preview_match_fg)
    }

    /// 產生 preview 中目前焦點命中的樣式。
    pub fn preview_current_line_style(self) -> Style {
        Style::default()
            .bg(self.preview_current_line_bg)
            .fg(self.preview_current_line_fg)
    }

    /// 產生危險操作標題的文字樣式。
    ///
    /// 參數：
    /// - `self: Theme`，目前主題。
    ///
    /// 回傳：`Style`，通常套用在刪除確認視窗等高風險互動上。
    pub fn danger_title_style(self) -> Style {
        Style::default()
            .fg(self.danger)
            .add_modifier(Modifier::BOLD)
    }

    /// 產生錯誤或無法執行通知使用的文字樣式。
    ///
    /// 參數：
    /// - `self: Theme`，目前使用中的主題色盤。
    ///
    /// 回傳：`Style`，使用目前主題定義的危險色，讓錯誤在狀態列中容易辨識。
    pub fn danger_style(self) -> Style {
        Style::default().fg(self.danger)
    }

    /// 產生成功或可套用狀態使用的文字樣式。
    ///
    /// 參數：
    /// - `self: Theme`，目前使用中的主題色盤。
    ///
    /// 回傳：`Style`，使用主題提供的成功色，讓可執行項目容易辨識。
    pub fn success_style(self) -> Style {
        Style::default().fg(self.executable)
    }
}

/// 將主題預設值轉成對應的實際色盤。
///
/// 參數：
/// - `value: ThemePreset`，要轉換的主題識別值。
///
/// 回傳：`Theme`，可直接用於畫面渲染。
impl From<ThemePreset> for Theme {
    /// 執行主題預設值到主題色盤的轉換。
    ///
    /// 參數：
    /// - `value: ThemePreset`，來源主題預設值。
    ///
    /// 回傳：`Theme`，對應的色盤資料。
    fn from(value: ThemePreset) -> Self {
        let palette = PaletteThemeName::from(value).palette();
        Self::new(
            palette.accent,
            palette.info,
            palette.muted,
            palette.selection,
            palette.fg,
            palette.warning,
            palette.error,
            palette.selection,
            palette.fg,
            palette.error,
            palette.info,
            palette.success,
            palette.secondary,
            palette.warning,
            palette.accent,
        )
    }
}

/// 將本專案的主題識別值轉成外部色盤套件使用的主題名稱。
///
/// 參數：
/// - `value: ThemePreset`，本專案目前選取的主題。
///
/// 回傳：`PaletteThemeName`，由 `ratatui-themes` 提供的成熟色盤識別值。
impl From<ThemePreset> for PaletteThemeName {
    /// 將 PaneFM 穩定的設定值逐一映射到 ratatui-themes 色盤名稱。
    fn from(value: ThemePreset) -> Self {
        match value {
            ThemePreset::Dracula => Self::Dracula,
            ThemePreset::OneDarkPro => Self::OneDarkPro,
            ThemePreset::Nord => Self::Nord,
            ThemePreset::CatppuccinMocha => Self::CatppuccinMocha,
            ThemePreset::CatppuccinLatte => Self::CatppuccinLatte,
            ThemePreset::GruvboxDark => Self::GruvboxDark,
            ThemePreset::GruvboxLight => Self::GruvboxLight,
            ThemePreset::TokyoNight => Self::TokyoNight,
            ThemePreset::SolarizedDark => Self::SolarizedDark,
            ThemePreset::SolarizedLight => Self::SolarizedLight,
            ThemePreset::MonokaiPro => Self::MonokaiPro,
            ThemePreset::RosePine => Self::RosePine,
            ThemePreset::Kanagawa => Self::Kanagawa,
            ThemePreset::Everforest => Self::Everforest,
            ThemePreset::Cyberpunk => Self::Cyberpunk,
            ThemePreset::MidnightCommander => Self::MidnightCommander,
        }
    }
}

/// 將外部色盤套件的主題名稱轉回本專案使用的識別值。
///
/// 參數：
/// - `value: PaletteThemeName`，由 `ratatui-themes` 解析出的主題名稱。
///
/// 回傳：`ThemePreset`，本專案可儲存、切換與渲染的主題識別值。
impl From<PaletteThemeName> for ThemePreset {
    /// 將外部色盤名稱映射回可寫入 config.toml 的 PaneFM 主題值。
    fn from(value: PaletteThemeName) -> Self {
        match value {
            PaletteThemeName::Dracula => Self::Dracula,
            PaletteThemeName::OneDarkPro => Self::OneDarkPro,
            PaletteThemeName::Nord => Self::Nord,
            PaletteThemeName::CatppuccinMocha => Self::CatppuccinMocha,
            PaletteThemeName::CatppuccinLatte => Self::CatppuccinLatte,
            PaletteThemeName::GruvboxDark => Self::GruvboxDark,
            PaletteThemeName::GruvboxLight => Self::GruvboxLight,
            PaletteThemeName::TokyoNight => Self::TokyoNight,
            PaletteThemeName::SolarizedDark => Self::SolarizedDark,
            PaletteThemeName::SolarizedLight => Self::SolarizedLight,
            PaletteThemeName::MonokaiPro => Self::MonokaiPro,
            PaletteThemeName::RosePine => Self::RosePine,
            PaletteThemeName::Kanagawa => Self::Kanagawa,
            PaletteThemeName::Everforest => Self::Everforest,
            PaletteThemeName::Cyberpunk => Self::Cyberpunk,
            PaletteThemeName::MidnightCommander => Self::MidnightCommander,
            _ => Self::Dracula,
        }
    }
}

impl Default for Theme {
    /// 取得 `Theme` 的預設實作。
    ///
    /// 參數：無。
    /// 回傳：`Theme`，目前等同於 `Theme::default_theme()`。
    fn default() -> Self {
        Self::default_theme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
