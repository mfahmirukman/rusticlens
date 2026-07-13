use egui::{Color32, Context, Stroke, Visuals};

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

pub struct Theme;

impl Theme {
    pub const BG: Color32 = Color32::from_rgb(26, 29, 33);
    pub const PANEL: Color32 = Color32::from_rgb(30, 34, 39);
    pub const PANEL_ELEVATED: Color32 = Color32::from_rgb(37, 42, 48);
    pub const BORDER: Color32 = Color32::from_rgb(55, 60, 68);
    pub const TEXT: Color32 = Color32::from_rgb(220, 224, 230);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(140, 147, 158);
    pub const ACCENT: Color32 = Color32::from_rgb(0, 191, 165);
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(0, 140, 122);
    pub const LINK: Color32 = Color32::from_rgb(93, 173, 226);
    pub const SUCCESS: Color32 = Color32::from_rgb(76, 175, 80);
    pub const WARNING: Color32 = Color32::from_rgb(255, 193, 7);
    pub const ERROR: Color32 = Color32::from_rgb(244, 67, 54);
    pub const ROW_SELECTED: Color32 = Color32::from_rgba_premultiplied(0, 191, 165, 40);
    pub const ACCENT_ACTIVE_BG: Color32 = Color32::from_rgb(28, 52, 58);
    pub const LOG_BG: Color32 = Color32::from_rgb(12, 14, 16);
    pub const SEARCH_HIGHLIGHT: Color32 = Color32::from_rgb(80, 70, 20);
    pub const SEARCH_CURRENT: Color32 = Color32::from_rgb(120, 90, 20);

    pub const LIGHT_BG: Color32 = Color32::from_rgb(245, 247, 250);
    pub const LIGHT_PANEL: Color32 = Color32::from_rgb(255, 255, 255);
    pub const LIGHT_PANEL_ELEVATED: Color32 = Color32::from_rgb(236, 240, 244);
    pub const LIGHT_BORDER: Color32 = Color32::from_rgb(200, 208, 218);
    pub const LIGHT_TEXT: Color32 = Color32::from_rgb(30, 36, 44);
    #[allow(dead_code)]
    pub const LIGHT_TEXT_MUTED: Color32 = Color32::from_rgb(95, 105, 118);
    pub const LIGHT_ROW_SELECTED: Color32 = Color32::from_rgba_premultiplied(0, 191, 165, 55);
    pub const LIGHT_LOG_BG: Color32 = Color32::from_rgb(250, 252, 255);

    pub fn apply(ctx: &Context) {
        Self::apply_mode(ctx, ThemeMode::from_settings());
    }

    pub fn apply_mode(ctx: &Context, mode: ThemeMode) {
        let mut visuals = match mode {
            ThemeMode::Dark => Visuals::dark(),
            ThemeMode::Light => Visuals::light(),
        };

        let (bg, panel, elevated, border, text, row_selected, log_bg) = match mode {
            ThemeMode::Dark => (
                Self::BG,
                Self::PANEL,
                Self::PANEL_ELEVATED,
                Self::BORDER,
                Self::TEXT,
                Self::ROW_SELECTED,
                Self::LOG_BG,
            ),
            ThemeMode::Light => (
                Self::LIGHT_BG,
                Self::LIGHT_PANEL,
                Self::LIGHT_PANEL_ELEVATED,
                Self::LIGHT_BORDER,
                Self::LIGHT_TEXT,
                Self::LIGHT_ROW_SELECTED,
                Self::LIGHT_LOG_BG,
            ),
        };

        visuals.panel_fill = panel;
        visuals.window_fill = elevated;
        visuals.extreme_bg_color = bg;
        visuals.faint_bg_color = elevated;
        visuals.widgets.noninteractive.bg_fill = panel;
        visuals.widgets.inactive.bg_fill = elevated;
        visuals.widgets.hovered.bg_fill = match mode {
            ThemeMode::Dark => Color32::from_rgb(48, 54, 62),
            ThemeMode::Light => Color32::from_rgb(220, 226, 234),
        };
        visuals.widgets.active.bg_fill = Self::ACCENT_DIM;
        visuals.selection.bg_fill = row_selected;
        visuals.selection.stroke = Stroke::new(1.0, text);
        visuals.hyperlink_color = Self::LINK;
        visuals.warn_fg_color = Self::WARNING;
        visuals.error_fg_color = Self::ERROR;
        visuals.override_text_color = Some(text);
        visuals.window_stroke = Stroke::new(1.0, border);
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        ctx.set_style(style);

        let _ = log_bg;
    }

    pub fn status_color(status: &str) -> Color32 {
        let lower = status.to_lowercase();
        if lower.contains("running")
            || lower.contains("available")
            || lower.contains("complete")
            || lower.contains("succeeded")
            || lower.contains("ready")
            || lower.contains("active")
            || lower == "true"
        {
            Self::SUCCESS
        } else if lower.contains("fail") || lower.contains("error") {
            Self::ERROR
        } else if lower.contains("progress") || lower.contains("pending") || lower.contains("idle")
        {
            Self::WARNING
        } else {
            Self::TEXT_MUTED
        }
    }
}
