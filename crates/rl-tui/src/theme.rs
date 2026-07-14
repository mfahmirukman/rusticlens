use ratatui::style::Color;
use rl_core::load_settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn from_settings() -> Self {
        match load_settings().theme.as_deref() {
            Some("light") => Self::Light,
            _ => Self::Dark,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemeColors {
    pub accent: Color,
    pub muted: Color,
    pub error: Color,
    pub text: Color,
    pub match_highlight: Color,
    #[allow(dead_code)]
    pub match_active: Color,
    #[allow(dead_code)]
    pub yank_highlight: Color,
    #[allow(dead_code)]
    pub selected_bg: Color,
}

impl ThemeColors {
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self {
                accent: Color::Rgb(0, 191, 165),
                muted: Color::Rgb(140, 147, 158),
                error: Color::Rgb(244, 67, 54),
                text: Color::Rgb(220, 224, 230),
                match_highlight: Color::Rgb(255, 235, 59),
                match_active: Color::Rgb(0, 96, 100),
                yank_highlight: Color::Rgb(45, 55, 72),
                selected_bg: Color::Rgb(28, 52, 58),
            },
            ThemeMode::Light => Self {
                accent: Color::Rgb(0, 140, 122),
                muted: Color::Rgb(95, 105, 118),
                error: Color::Rgb(198, 40, 40),
                text: Color::Rgb(30, 36, 44),
                match_highlight: Color::Rgb(180, 140, 0),
                match_active: Color::Rgb(0, 120, 110),
                yank_highlight: Color::Rgb(220, 230, 240),
                selected_bg: Color::Rgb(200, 235, 230),
            },
        }
    }
}
