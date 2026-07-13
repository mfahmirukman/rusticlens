use eframe::egui;

use crate::ui::theme::Theme;

pub enum ClusterTabAction {
    None,
    Select(String),
    Close(String),
    Add(String),
}

pub fn show(
    ui: &mut egui::Ui,
    open_tabs: &[String],
    active: &str,
    available: &[String],
    picker_open: &mut bool,
) -> ClusterTabAction {
    let mut action = ClusterTabAction::None;

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Clusters")
                .color(Theme::TEXT_MUTED)
                .small(),
        );
        ui.separator();

        for tab in open_tabs {
            let selected = tab == active;
            let label = short_context_label(tab);
            ui.menu_button(label, |ui| {
                if ui.button("Switch").clicked() && !selected {
                    action = ClusterTabAction::Select(tab.clone());
                    ui.close_menu();
                }
                if ui.button("Close tab").clicked() {
                    action = ClusterTabAction::Close(tab.clone());
                    ui.close_menu();
                }
            });
            if selected {
                ui.label(egui::RichText::new("●").small().color(Theme::ACCENT));
            }
        }

        if ui.small_button("+").clicked() {
            *picker_open = true;
        }
    });

    if *picker_open {
        egui::Window::new("Open cluster tab")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                for ctx in available {
                    if open_tabs.iter().any(|t| t == ctx) {
                        continue;
                    }
                    if ui.button(ctx).clicked() {
                        action = ClusterTabAction::Add(ctx.clone());
                        *picker_open = false;
                    }
                }
                if ui.button("Cancel").clicked() {
                    *picker_open = false;
                }
            });
    }

    action
}

fn short_context_label(context: &str) -> &str {
    context.rsplit('/').next().unwrap_or(context)
}
