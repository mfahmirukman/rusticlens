use egui::{Color32, Context, Stroke, Visuals};

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
    /// Dark teal tint for active rail items — keeps white text readable.
    pub const ACCENT_ACTIVE_BG: Color32 = Color32::from_rgb(28, 52, 58);
    pub const LOG_BG: Color32 = Color32::from_rgb(12, 14, 16);

    pub fn apply(ctx: &Context) {
        let mut visuals = Visuals::dark();
        visuals.panel_fill = Self::PANEL;
        visuals.window_fill = Self::PANEL_ELEVATED;
        visuals.extreme_bg_color = Self::BG;
        visuals.faint_bg_color = Self::PANEL_ELEVATED;
        visuals.widgets.noninteractive.bg_fill = Self::PANEL;
        visuals.widgets.inactive.bg_fill = Self::PANEL_ELEVATED;
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 54, 62);
        visuals.widgets.active.bg_fill = Self::ACCENT_DIM;
        visuals.selection.bg_fill = Self::ROW_SELECTED;
        // Table cells use selection.stroke.color as selected text (egui_extras).
        visuals.selection.stroke = Stroke::new(1.0, Self::TEXT);
        visuals.hyperlink_color = Self::LINK;
        visuals.warn_fg_color = Self::WARNING;
        visuals.error_fg_color = Self::ERROR;
        visuals.override_text_color = Some(Self::TEXT);
        visuals.window_stroke = Stroke::new(1.0, Self::BORDER);
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        ctx.set_style(style);
    }

    pub fn status_color(status: &str) -> Color32 {
        let lower = status.to_lowercase();
        if lower.contains("running")
            || lower.contains("available")
            || lower.contains("complete")
            || lower.contains("succeeded")
            || lower.contains("ready")
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
