use egui::{ScrollArea, TextWrapMode, Ui};

use crate::ui::log_tabs::LogTab;
use crate::ui::theme::Theme;

const SCROLL_LOAD_THRESHOLD: f32 = 24.0;
/// Fixed row height for virtual scrolling (monospace ~13px + spacing).
const LOG_LINE_HEIGHT: f32 = 15.0;

pub struct LogPanelState {
    pub word_wrap: bool,
    pub show_timestamps: bool,
    pub log_filter: String,
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

#[derive(Debug, Default)]
pub struct LogPanelAction {
    pub select_tab: Option<u64>,
    pub close_tab: Option<u64>,
}

#[derive(Debug, Default)]
pub struct LogContentAction {
    pub container: Option<(u64, String)>,
    pub restart_stream: bool,
    pub load_older: Option<u64>,
}

pub fn show_tab_bar(
    ui: &mut Ui,
    tabs: &[LogTab],
    active_id: Option<u64>,
) -> LogPanelAction {
    let mut action = LogPanelAction::default();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for tab in tabs {
            let selected = active_id == Some(tab.id);
            let title = truncate_tab_title(&tab.pod_name);

            let tab_fill = if selected {
                Theme::ACCENT_ACTIVE_BG
            } else {
                Theme::PANEL_ELEVATED
            };
            let text_color = if selected {
                Theme::TEXT
            } else {
                Theme::TEXT_MUTED
            };

            ui.horizontal(|ui| {
                let label = egui::Button::new(egui::RichText::new(&title).size(11.0).color(text_color))
                    .fill(tab_fill)
                    .stroke(egui::Stroke::new(
                        1.0,
                        if selected {
                            Theme::ACCENT
                        } else {
                            Theme::BORDER
                        },
                    ))
                    .min_size(egui::vec2(72.0, 22.0));
                if ui.add(label).clicked() {
                    action.select_tab = Some(tab.id);
                }

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("×")
                                .size(16.0)
                                .strong()
                                .color(Theme::TEXT_MUTED),
                        )
                        .min_size(egui::vec2(22.0, 22.0))
                        .frame(false),
                    )
                    .on_hover_text("Close tab")
                    .clicked()
                {
                    action.close_tab = Some(tab.id);
                }
            });
        }
    });

    let line = ui.max_rect();
    ui.painter().hline(
        line.left()..=line.right(),
        line.bottom() - 1.0,
        egui::Stroke::new(1.0, Theme::BORDER),
    );
    ui.add_space(2.0);

    action
}

pub fn show_tab_content(
    ui: &mut Ui,
    state: &mut LogPanelState,
    tab: Option<&LogTab>,
) -> LogContentAction {
    let mut action = LogContentAction::default();

    let Some(tab) = tab else {
        ui.label(
            egui::RichText::new("Double-click a pod or use the context menu to open logs.")
                .color(Theme::TEXT_MUTED),
        );
        return action;
    };

    let prev_timestamps = state.show_timestamps;

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("Pod: {}", tab.pod_name))
                .strong()
                .color(Theme::ACCENT),
        );
        ui.separator();
        ui.label(
            egui::RichText::new(format!("ns: {}", tab.namespace))
                .small()
                .color(Theme::TEXT_MUTED),
        );
        if tab.loading_older {
            ui.label(
                egui::RichText::new("Loading older logs…")
                    .small()
                    .color(Theme::TEXT_MUTED),
            );
        } else if !tab.has_more_older && !tab.is_empty() {
            ui.label(
                egui::RichText::new("Beginning of log")
                    .small()
                    .color(Theme::TEXT_MUTED),
            );
        }
        if !tab.containers.is_empty() {
            let selected = tab.container.as_deref().unwrap_or("-");
            egui::ComboBox::from_id_salt(("log_container", tab.id))
                .selected_text(selected)
                .width(140.0)
                .show_ui(ui, |ui| {
                    for c in &tab.containers {
                        let active = tab.container.as_deref() == Some(c.name.as_str());
                        if ui.selectable_label(active, &c.name).clicked() {
                            action.container = Some((tab.id, c.name.clone()));
                        }
                    }
                });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut state.show_timestamps, "Show timestamps");
            ui.checkbox(&mut state.word_wrap, "Word wrap");
            ui.checkbox(&mut state.follow_tail, "Follow tail");
            ui.add(
                egui::TextEdit::singleline(&mut state.log_filter)
                    .hint_text("Search logs...")
                    .desired_width(140.0),
            );
        });
    });

    if state.show_timestamps != prev_timestamps {
        action.restart_stream = true;
    }

    ui.add_space(2.0);

    let wrap_mode = if state.word_wrap {
        TextWrapMode::Wrap
    } else {
        TextWrapMode::Extend
    };

    let log_area_height = ui.available_height().max(0.0);
    let filter = state.log_filter.to_lowercase();
    ui.allocate_ui(egui::vec2(ui.available_width(), log_area_height), |ui| {
        ui.set_min_height(log_area_height);
        egui::Frame::new()
            .fill(Theme::LOG_BG)
            .inner_margin(6.0)
            .show(ui, |ui| {
                let scroll_height = ui.available_height().max(0.0);
                let mut scroll = ScrollArea::both()
                    .id_salt(("log_scroll", tab.id))
                    .auto_shrink([false, false])
                    .max_height(scroll_height);
                if state.follow_tail && !tab.loading_older {
                    scroll = scroll.stick_to_bottom(true);
                }
                let scroll_out = if tab.is_empty() {
                    scroll
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Waiting for log output...")
                                    .color(Theme::TEXT_MUTED),
                            );
                        })
                } else {
                    let lines = tab.lines();
                    let filtered = tab.matching_indices(&filter);
                    let total_rows = if filter.is_empty() {
                        lines.len()
                    } else {
                        filtered.len()
                    };

                    if total_rows == 0 {
                        scroll.show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("No matching log lines.")
                                    .color(Theme::TEXT_MUTED),
                            );
                        })
                    } else {
                        scroll.show_rows(ui, LOG_LINE_HEIGHT, total_rows, |ui, row_range| {
                            for row in row_range {
                                let line = if filter.is_empty() {
                                    lines.get(row).map(String::as_str)
                                } else {
                                    filtered
                                        .get(row)
                                        .and_then(|&idx| lines.get(idx))
                                        .map(String::as_str)
                                };
                                if let Some(line) = line {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(line)
                                                .monospace()
                                                .color(Theme::TEXT),
                                        )
                                        .wrap_mode(wrap_mode)
                                        .selectable(true),
                                    );
                                }
                            }
                        })
                    }
                };

                if tab.has_more_older
                    && !tab.loading_older
                    && !state.follow_tail
                    && scroll_out.content_size.y > scroll_height + SCROLL_LOAD_THRESHOLD
                    && scroll_out.state.offset.y <= SCROLL_LOAD_THRESHOLD
                {
                    action.load_older = Some(tab.id);
                }
            });
    });

    action
}

fn truncate_tab_title(name: &str) -> String {
    if name.len() <= 18 {
        name.to_string()
    } else {
        format!("{}…", &name[..17])
    }
}
