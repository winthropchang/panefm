use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePreset {
    Default,
    Forest,
    Ocean,
}

impl ThemePreset {
    pub const ALL: [ThemePreset; 3] = [
        ThemePreset::Default,
        ThemePreset::Forest,
        ThemePreset::Ocean,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            ThemePreset::Default => "default",
            ThemePreset::Forest => "forest",
            ThemePreset::Ocean => "ocean",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "default" => Some(Self::Default),
            "forest" => Some(Self::Forest),
            "ocean" => Some(Self::Ocean),
            _ => None,
        }
    }

    pub const fn next(self) -> Self {
        match self {
            ThemePreset::Default => ThemePreset::Forest,
            ThemePreset::Forest => ThemePreset::Ocean,
            ThemePreset::Ocean => ThemePreset::Default,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub focus_border: Color,
    pub muted: Color,
    pub selection_bg: Color,
    pub danger: Color,
}

impl Theme {
    pub const fn new(
        accent: Color,
        focus_border: Color,
        muted: Color,
        selection_bg: Color,
        danger: Color,
    ) -> Self {
        Self {
            accent,
            focus_border,
            muted,
            selection_bg,
            danger,
        }
    }

    pub const fn default_theme() -> Self {
        Self::new(
            Color::Yellow,
            Color::Green,
            Color::DarkGray,
            Color::Blue,
            Color::LightRed,
        )
    }

    pub const fn forest_theme() -> Self {
        Self::new(
            Color::LightGreen,
            Color::Green,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
        )
    }

    pub const fn ocean_theme() -> Self {
        Self::new(
            Color::LightCyan,
            Color::Cyan,
            Color::Gray,
            Color::Blue,
            Color::LightRed,
        )
    }

    pub fn accent_style(self) -> Style {
        Style::default().fg(self.accent)
    }

    pub fn focused_border_style(self) -> Style {
        Style::default()
            .fg(self.focus_border)
            .add_modifier(Modifier::BOLD)
    }

    pub fn muted_style(self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn selected_item_style(self) -> Style {
        Style::default().bg(self.selection_bg)
    }

    pub fn danger_title_style(self) -> Style {
        Style::default()
            .fg(self.danger)
            .add_modifier(Modifier::BOLD)
    }
}

impl From<ThemePreset> for Theme {
    fn from(value: ThemePreset) -> Self {
        match value {
            ThemePreset::Default => Theme::default_theme(),
            ThemePreset::Forest => Theme::forest_theme(),
            ThemePreset::Ocean => Theme::ocean_theme(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_theme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_themes_are_distinct() {
        assert_ne!(Theme::default_theme(), Theme::forest_theme());
        assert_ne!(Theme::default_theme(), Theme::ocean_theme());
    }

    #[test]
    fn preset_name_round_trip_works() {
        for preset in ThemePreset::ALL {
            assert_eq!(ThemePreset::from_name(preset.name()), Some(preset));
        }
    }
}
