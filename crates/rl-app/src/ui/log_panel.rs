use egui::{ScrollArea, Ui};

use crate::ui::theme::Theme;

const MAX_DISPLAY_LINES: usize = 500;

pub struct LogPanelState {
    pub word_wrap: bool,
    pub show_timestamps: bool,
    pub log_filter: String,
    /// When true, keep the viewport pinned to the newest log lines (terminal-style).
    pub follow_tail: bool,
}

impl Default for LogPanelState {
    fn default() -> Self {
        Self {
            word_wrap: true,
            show_timestamps: false,
            log_filter: String::new(),
            follow_tail: true,
        }
    }
}

pub fn show(
    ui: &mut Ui,
    state: &mut LogPanelState,
    pod_name: Option<&str>,
    namespace: &str,
    container: Option<&str>,
    logs: &[String],
    containers: &[(String, bool)],
) -> Option<String> {
    let mut picked_container = None;

    ui.horizontal(|ui| {
        let tab_label = pod_name
            .map(|p| format!("Pod: {p}"))
            .unwrap_or_else(|| "Logs".into());
        ui.label(
            egui::RichText::new(&tab_label)
                .strong()
                .color(Theme::ACCENT),
        );
        if pod_name.is_some() {
            ui.separator();
            ui.label(
                egui::RichText::new(format!("ns: {namespace}"))
                    .small()
                    .color(Theme::TEXT_MUTED),
            );
            if !containers.is_empty() {
                let selected = container.unwrap_or("-");
                egui::ComboBox::from_id_salt("log_container")
                    .selected_text(selected)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        for (name, _) in containers {
                            if ui.selectable_label(Some(name.as_str()) == container, name).clicked()
                            {
                                picked_container = Some(name.clone());
                            }
                        }
                    });
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut state.follow_tail, "Follow tail");
            ui.checkbox(&mut state.word_wrap, "Word wrap");
            ui.checkbox(&mut state.show_timestamps, "Show timestamps");
            ui.add(
                egui::TextEdit::singleline(&mut state.log_filter)
                    .hint_text("Search logs...")
                    .desired_width(140.0),
            );
        });
    });

    ui.add_space(2.0);

    let log_area_height = ui.available_height().max(0.0);
    ui.allocate_ui(egui::vec2(ui.available_width(), log_area_height), |ui| {
        ui.set_min_height(log_area_height);
        let filter = state.log_filter.to_lowercase();
        egui::Frame::new()
            .fill(Theme::LOG_BG)
            .inner_margin(6.0)
            .show(ui, |ui| {
                let scroll_height = ui.available_height().max(0.0);
                let mut scroll = ScrollArea::both()
                    .id_salt("log_scroll")
                    .auto_shrink([false, false])
                    .max_height(scroll_height);
                if state.follow_tail {
                    scroll = scroll.stick_to_bottom(true);
                }
                scroll.show(ui, |ui| {
                        if logs.is_empty() {
                            ui.label(
                                egui::RichText::new("Select a pod to stream logs.")
                                    .color(Theme::TEXT_MUTED),
                            );
                            return;
                        }

                        // Oldest at top, newest at bottom — stick_to_bottom shows latest first.
                        let start = logs.len().saturating_sub(MAX_DISPLAY_LINES);
                        for line in &logs[start..] {
                            if !filter.is_empty() && !line.to_lowercase().contains(&filter) {
                                continue;
                            }
                            if state.word_wrap {
                                ui.label(
                                    egui::RichText::new(line)
                                        .monospace()
                                        .color(Theme::TEXT),
                                );
                            } else {
                                ui.monospace(line);
                            }
                        }
                    });
            });
    });

    picked_container
}
