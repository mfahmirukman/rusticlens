use egui::{Align, Layout, ScrollArea, Ui};

use crate::ui::theme::Theme;

const RAIL_WIDTH: f32 = 124.0;
const LABEL_WIDTH: f32 = 88.0;

#[derive(Debug, Default)]
pub struct IconRailState {
    pub context_menu_open: bool,
    pub context_search: String,
}

#[derive(Debug, Default)]
pub struct IconRailAction {
    pub switch_to: Option<String>,
    pub show_overview: bool,
    pub pin: Option<String>,
    pub unpin: Option<String>,
    pub open_settings: bool,
}

pub fn rail_width() -> f32 {
    RAIL_WIDTH
}

pub fn show(
    ui: &mut Ui,
    state: &mut IconRailState,
    pinned_contexts: &[String],
    active_context: &str,
) -> IconRailAction {
    let mut action = IconRailAction::default();

    ui.vertical_centered(|ui| {
        ui.set_width(RAIL_WIDTH - 8.0);

        let home = ui.add(
            egui::Button::new(
                egui::RichText::new("Ov")
                    .size(12.0)
                    .color(Theme::TEXT_MUTED),
            )
            .min_size(egui::vec2(36.0, 28.0)),
        );
        if home.clicked() {
            action.show_overview = true;
        }
        home.on_hover_text("Cluster overview");

        ui.add_space(4.0);

        let menu = ui.add(
            egui::Button::new(
                egui::RichText::new("Ctx")
                    .size(11.0)
                    .color(Theme::TEXT_MUTED),
            )
            .min_size(egui::vec2(36.0, 28.0)),
        );
        if menu.clicked() {
            state.context_menu_open = !state.context_menu_open;
        }
        menu.on_hover_text("All clusters - browse every kubeconfig context");

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        ui.label(
            egui::RichText::new("Pinned")
                .small()
                .color(Theme::TEXT_MUTED),
        );
        ui.add_space(4.0);
    });

    ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(RAIL_WIDTH - 8.0);
            if pinned_contexts.is_empty() {
                ui.label(
                    egui::RichText::new("No pinned\ncontexts")
                        .small()
                        .color(Theme::TEXT_MUTED),
                );
            } else {
                for ctx in pinned_contexts {
                    show_pinned_context(ui, ctx, active_context, &mut action);
                }
            }
        });

    ui.add_space(4.0);
    let add = ui.add(
        egui::Button::new(egui::RichText::new("+").size(18.0).color(Theme::ACCENT))
            .min_size(egui::vec2(36.0, 28.0)),
    );
    if add.clicked() {
        state.context_menu_open = true;
    }
    add.on_hover_text("Add cluster to pinned list");

    action
}

pub fn show_context_menu(
    ctx: &egui::Context,
    state: &mut IconRailState,
    all_contexts: &[String],
    pinned_contexts: &[String],
    active_context: &str,
) -> IconRailAction {
    let mut action = IconRailAction::default();
    if !state.context_menu_open {
        return action;
    }

    let mut open = true;
    egui::Window::new("All clusters")
        .open(&mut open)
        .collapsible(false)
        .default_width(360.0)
        .default_pos(egui::pos2(140.0, 48.0))
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Contexts from your kubeconfig")
                    .small()
                    .color(Theme::TEXT_MUTED),
            );
            ui.add(
                egui::TextEdit::singleline(&mut state.context_search)
                    .hint_text("Search contexts...")
                    .desired_width(f32::INFINITY),
            );
            ui.separator();

            let query = state.context_search.to_lowercase();
            ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(320.0)
                .show(ui, |ui| {
                    for context in all_contexts {
                        if !query.is_empty() && !context.to_lowercase().contains(&query) {
                            continue;
                        }
                        show_catalog_row(
                            ui,
                            context,
                            active_context,
                            pinned_contexts.contains(&context.to_string()),
                            &mut action,
                        );
                    }
                });
            ui.add_space(4.0);
            if crate::ui::settings_dialog::settings_link(ui) {
                action.open_settings = true;
                state.context_menu_open = false;
            }
        });

    if !open {
        state.context_menu_open = false;
    }

    action
}

fn show_pinned_context(
    ui: &mut Ui,
    context: &str,
    active_context: &str,
    action: &mut IconRailAction,
) {
    let active = context == active_context;
    let labels = context_label_lines(context);
    let body = labels.join("\n");

    let fill = if active {
        Theme::ACCENT_ACTIVE_BG
    } else {
        Theme::PANEL_ELEVATED
    };
    let text_color = if active {
        Theme::TEXT
    } else {
        Theme::TEXT_MUTED
    };
    let border = if active { Theme::ACCENT } else { Theme::BORDER };

    ui.horizontal(|ui| {
        let switch = ui.add(
            egui::Button::new(egui::RichText::new(body).size(10.5).color(text_color))
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, border))
                .min_size(egui::vec2(LABEL_WIDTH, 32.0)),
        );

        if active {
            let bar = egui::Rect::from_min_size(
                switch.rect.left_top(),
                egui::vec2(3.0, switch.rect.height()),
            );
            ui.painter().rect_filled(bar, 0.0, Theme::ACCENT);
        }

        if (switch.clicked() || switch.double_clicked()) && !active {
            action.switch_to = Some(context.to_string());
        }
        switch.on_hover_text(if active {
            format!("{context}\n(current)")
        } else {
            format!("{context}\nClick to switch")
        });

        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("x")
                        .size(14.0)
                        .strong()
                        .color(Theme::TEXT_MUTED),
                )
                .min_size(egui::vec2(22.0, 22.0))
                .frame(false),
            )
            .on_hover_text("Unpin from rail")
            .clicked()
        {
            action.unpin = Some(context.to_string());
        }
    });
}

fn show_catalog_row(
    ui: &mut Ui,
    context: &str,
    active_context: &str,
    pinned: bool,
    action: &mut IconRailAction,
) {
    let active = context == active_context;
    ui.horizontal(|ui| {
        let label = if active {
            egui::RichText::new(context).color(Theme::ACCENT)
        } else {
            egui::RichText::new(context).color(Theme::TEXT)
        };
        let row = ui.selectable_label(active, label);
        if (row.clicked() || row.double_clicked()) && !active {
            action.switch_to = Some(context.to_string());
            action.show_overview = false;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if pinned {
                if ui.small_button("Unpin").clicked() {
                    action.unpin = Some(context.to_string());
                }
            } else if ui.small_button("Pin").clicked() {
                action.pin = Some(context.to_string());
            }
            if active {
                ui.label(egui::RichText::new("active").small().color(Theme::SUCCESS));
            }
        });
    });
}

/// Build 1–3 short lines for the narrow rail, preferring the most specific suffix.
pub fn context_label_lines(name: &str) -> Vec<String> {
    const MAX_LINE: usize = 13;
    const MAX_LINES: usize = 3;

    let short = name
        .rsplit_once('/')
        .map(|(_, tail)| tail)
        .or_else(|| name.rsplit_once(':').map(|(_, tail)| tail))
        .unwrap_or(name);

    if short.len() <= MAX_LINE {
        return vec![short.to_string()];
    }

    let parts: Vec<&str> = short.split('-').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        let tail = parts[parts.len().saturating_sub(3)..].join("-");
        if tail.len() <= MAX_LINE * MAX_LINES {
            return wrap_lines(&tail, MAX_LINE, MAX_LINES);
        }
    }

    wrap_lines(short, MAX_LINE, MAX_LINES)
}

fn wrap_lines(text: &str, max_line: usize, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while lines.len() < max_lines && !rest.is_empty() {
        if rest.len() <= max_line {
            lines.push(rest.to_string());
            break;
        }
        let mut split_at = max_line;
        while split_at > 0 && !rest.is_char_boundary(split_at) {
            split_at -= 1;
        }
        if split_at == 0 {
            break;
        }
        lines.push(rest[..split_at].to_string());
        rest = rest[split_at..].trim_start_matches('-');
    }
    if lines.len() == max_lines && !rest.is_empty() {
        if let Some(last) = lines.last_mut() {
            if last.len() < max_line {
                last.push_str("...");
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_context_is_one_line() {
        assert_eq!(context_label_lines("dev"), vec!["dev"]);
    }

    #[test]
    fn long_context_uses_tail_segments() {
        let lines = context_label_lines("teleport-v2.example.net-tke-rumah123-dev");
        assert!(!lines.is_empty());
        assert!(lines.join("-").contains("rumah123"));
    }
}
