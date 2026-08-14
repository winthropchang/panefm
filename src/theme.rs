use ratatui::style::{Color, Modifier, Style};

/// 表示可被使用者選擇的主題預設名稱。
///
/// 這個列舉不直接保存顏色，而是作為主題系統的穩定識別值，
/// 讓命令模式、設定檔與主題選擇視窗都能用相同名稱操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePreset {
    Default,
    Forest,
    Ocean,
}

impl ThemePreset {
    /// 內建所有主題預設值的固定清單。
    ///
    /// 參數：無。
    /// 回傳：`[ThemePreset; 3]`，依序列出所有可用主題。
    pub const ALL: [ThemePreset; 3] = [
        ThemePreset::Default,
        ThemePreset::Forest,
        ThemePreset::Ocean,
    ];

    /// 取得主題預設值對應的字串名稱。
    ///
    /// 參數：
    /// - `self: ThemePreset`，目前的主題預設值。
    ///
    /// 回傳：`&'static str`，可用於命令模式或設定檔的名稱。
    pub const fn name(self) -> &'static str {
        match self {
            ThemePreset::Default => "default",
            ThemePreset::Forest => "forest",
            ThemePreset::Ocean => "ocean",
        }
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
        match name {
            "default" => Some(Self::Default),
            "forest" => Some(Self::Forest),
            "ocean" => Some(Self::Ocean),
            _ => None,
        }
    }

    /// 取得下一個主題預設值，供輪替主題時使用。
    ///
    /// 參數：
    /// - `self: ThemePreset`，目前的主題預設值。
    ///
    /// 回傳：`ThemePreset`，依照固定順序切換後的主題。
    pub const fn next(self) -> Self {
        match self {
            ThemePreset::Default => ThemePreset::Forest,
            ThemePreset::Forest => ThemePreset::Ocean,
            ThemePreset::Ocean => ThemePreset::Default,
        }
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
    pub danger: Color,
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
    pub const fn new(
        accent: Color,
        focus_border: Color,
        muted: Color,
        selection_bg: Color,
        selection_fg: Color,
        danger: Color,
    ) -> Self {
        Self {
            accent,
            focus_border,
            muted,
            selection_bg,
            selection_fg,
            danger,
        }
    }

    /// 建立預設主題。
    ///
    /// 參數：無。
    /// 回傳：`Theme`，作為程式啟動時的預設色盤。
    pub const fn default_theme() -> Self {
        Self::new(
            Color::Yellow,
            Color::Green,
            Color::DarkGray,
            Color::Gray,
            Color::Black,
            Color::LightRed,
        )
    }

    /// 建立偏綠色調的森林主題。
    ///
    /// 參數：無。
    /// 回傳：`Theme`，適合較自然風格的配色。
    pub const fn forest_theme() -> Self {
        Self::new(
            Color::LightGreen,
            Color::Green,
            Color::Gray,
            Color::Rgb(90, 90, 90),
            Color::White,
            Color::LightRed,
        )
    }

    /// 建立偏藍色調的海洋主題。
    ///
    /// 參數：無。
    /// 回傳：`Theme`，適合較冷色與清爽風格的配色。
    pub const fn ocean_theme() -> Self {
        Self::new(
            Color::LightCyan,
            Color::Cyan,
            Color::Gray,
            Color::Rgb(70, 80, 95),
            Color::White,
            Color::LightRed,
        )
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
        match value {
            ThemePreset::Default => Theme::default_theme(),
            ThemePreset::Forest => Theme::forest_theme(),
            ThemePreset::Ocean => Theme::ocean_theme(),
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
    fn built_in_themes_are_distinct() {
        assert_ne!(Theme::default_theme(), Theme::forest_theme());
        assert_ne!(Theme::default_theme(), Theme::ocean_theme());
    }

    #[test]
    /// 驗證主題名稱與主題預設值之間可以雙向對應。
    ///
    /// 參數：無。
    /// 回傳：無；若名稱無法正確解析則測試失敗。
    fn preset_name_round_trip_works() {
        for preset in ThemePreset::ALL {
            assert_eq!(ThemePreset::from_name(preset.name()), Some(preset));
        }
    }
}
