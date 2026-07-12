use egui::{ScrollArea, TextEdit, Ui};

use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Describe,
    Events,
    Logs,
    Metrics,
}

pub struct DetailState {
    pub tab: DetailTab,
    pub yaml: String,
    pub events: String,
    pub metrics: String,
    pub resource_name: String,
}

impl DetailState {
    pub fn clear(&mut self) {
        self.yaml.clear();
        self.events.clear();
        self.metrics.clear();
        self.resource_name.clear();
    }
}

pub fn show_content(ui: &mut Ui, state: &mut DetailState, logs: &[String]) {
    let area_height = ui.available_height().max(0.0);
    ui.allocate_ui(egui::vec2(ui.available_width(), area_height), |ui| {
        ui.set_min_height(area_height);
        ui.set_max_height(area_height);
        let scroll_height = ui.available_height().max(0.0);
        match state.tab {
            DetailTab::Describe => {
                ScrollArea::both()
                    .auto_shrink([false, false])
                    .max_height(scroll_height)
                    .show(ui, |ui| {
                    if state.yaml.is_empty() {
                        ui.label(
                            egui::RichText::new("Select a resource to view YAML.")
                                .color(Theme::TEXT_MUTED),
                        );
                    } else {
                        ui.add(
                            TextEdit::multiline(&mut state.yaml)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    }
                });
        }
        DetailTab::Events => {
            ScrollArea::both()
                .auto_shrink([false, false])
                .max_height(scroll_height)
                .show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut state.events)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
        }
        DetailTab::Logs => {
            crate::ui::log_viewer::show(ui, logs);
        }
        DetailTab::Metrics => {
            ScrollArea::both()
                .auto_shrink([false, false])
                .max_height(scroll_height)
                .show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut state.metrics)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .interactive(false),
                    );
                });
        }
    }
    });
}
