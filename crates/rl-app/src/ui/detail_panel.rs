use egui::{Align, Label, RichText, ScrollArea, TextEdit, Ui};

use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Describe,
    Events,
    Metrics,
}

pub struct DetailState {
    pub tab: DetailTab,
    pub yaml: String,
    pub events: String,
    pub metrics: String,
    pub resource_name: String,
}

#[derive(Debug, Default)]
pub struct DetailSearchState {
    pub query: String,
    pub match_index: usize,
    pub focus_field: bool,
}

impl DetailState {
    pub fn clear(&mut self) {
        self.yaml.clear();
        self.events.clear();
        self.metrics.clear();
        self.resource_name.clear();
    }

    fn active_text(&self) -> &str {
        match self.tab {
            DetailTab::Describe => &self.yaml,
            DetailTab::Events => &self.events,
            DetailTab::Metrics => &self.metrics,
        }
    }

    fn supports_search(&self) -> bool {
        matches!(self.tab, DetailTab::Describe | DetailTab::Events)
    }
}

impl DetailSearchState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn open(&mut self) {
        self.focus_field = true;
    }
}

/// Header for the detail side panel. Returns `true` when the user closes the panel.
pub fn show_header(
    ui: &mut Ui,
    active: &mut DetailTab,
    resource_name: &str,
    search: &mut DetailSearchState,
) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        if !resource_name.is_empty() {
            ui.label(
                egui::RichText::new(resource_name)
                    .strong()
                    .color(Theme::ACCENT),
            );
            ui.separator();
        }
        tab_btn(ui, active, DetailTab::Describe, "Describe");
        tab_btn(ui, active, DetailTab::Events, "Events");
        tab_btn(ui, active, DetailTab::Metrics, "Metrics");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("X").color(Theme::TEXT_MUTED))
                        .frame(false),
                )
                .on_hover_text("Close")
                .clicked()
            {
                close = true;
            }
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("Find").color(Theme::TEXT_MUTED))
                        .frame(false),
                )
                .on_hover_text("Search (Ctrl+F)")
                .clicked()
            {
                search.open();
            }
        });
    });
    ui.separator();
    close
}

pub fn show_content(ui: &mut Ui, state: &mut DetailState, search: &mut DetailSearchState) {
    let area_height = ui.available_height().max(0.0);
    ui.allocate_ui(egui::vec2(ui.available_width(), area_height), |ui| {
        ui.set_min_height(area_height);
        ui.set_max_height(area_height);

        let text = state.active_text();
        let empty_hint = match state.tab {
            DetailTab::Describe => "Select a resource to view YAML.",
            DetailTab::Events => "Select a resource to view events.",
            DetailTab::Metrics => {
                "Pod metrics (requires metrics-server). Select pods and open Metrics."
            }
        };

        if text.is_empty() {
            ui.label(egui::RichText::new(empty_hint).color(Theme::TEXT_MUTED));
            return;
        }

        let mut scroll_to_match = false;
        if state.supports_search() {
            scroll_to_match = show_search_bar(ui, text, search);
        }

        let scroll_height = ui.available_height().max(0.0);
        let matches = find_matches(text, &search.query);
        if search.match_index >= matches.len() && !matches.is_empty() {
            search.match_index = matches.len() - 1;
        }
        let current_match = matches.get(search.match_index).copied();

        ScrollArea::both()
            .auto_shrink([false, false])
            .max_height(scroll_height)
            .show(ui, |ui| {
                if search.query.is_empty() || !state.supports_search() {
                    selectable_readonly_text(ui, text);
                    return;
                }

                let mut scroll_target = None;
                for (line_start, line) in iter_lines(text) {
                    let line_end = line_start + line.len();
                    let line_has_match = current_match
                        .is_some_and(|m| m >= line_start && m < line_end.saturating_add(1));
                    let response = ui.horizontal_wrapped(|ui| {
                        render_line_with_highlights(
                            ui,
                            line,
                            line_start,
                            &search.query,
                            current_match,
                        );
                    });
                    if line_has_match {
                        scroll_target = Some(response.response.rect);
                    }
                }
                if scroll_to_match {
                    if let Some(rect) = scroll_target {
                        ui.scroll_to_rect(rect, Some(Align::TOP));
                    }
                }
            });
    });
}

fn show_search_bar(ui: &mut Ui, text: &str, search: &mut DetailSearchState) -> bool {
    let matches = find_matches(text, &search.query);
    let mut scroll_to_match = false;
    let search_id = egui::Id::new("detail_panel_search");

    ui.horizontal(|ui| {
        let response = ui.add(
            TextEdit::singleline(&mut search.query)
                .id(search_id)
                .hint_text("Search...")
                .desired_width(ui.available_width().min(180.0)),
        );
        if search.focus_field {
            response.request_focus();
            search.focus_field = false;
        }
        if response.changed() {
            search.match_index = 0;
            scroll_to_match = true;
        }

        if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if ui.input(|i| i.modifiers.shift) {
                prev_match(search, matches.len());
            } else {
                next_match(search, matches.len());
            }
            scroll_to_match = true;
        }

        if matches.is_empty() {
            if !search.query.is_empty() {
                ui.label(egui::RichText::new("No matches").color(Theme::TEXT_MUTED));
            }
        } else {
            ui.label(
                egui::RichText::new(format!("{} / {}", search.match_index + 1, matches.len()))
                    .color(Theme::TEXT_MUTED),
            );
            if ui
                .small_button("^")
                .on_hover_text("Previous (Shift+Enter)")
                .clicked()
            {
                prev_match(search, matches.len());
                scroll_to_match = true;
            }
            if ui.small_button("v").on_hover_text("Next (Enter)").clicked() {
                next_match(search, matches.len());
                scroll_to_match = true;
            }
        }

        if ui
            .small_button("Clr")
            .on_hover_text("Clear search")
            .clicked()
        {
            search.query.clear();
            search.match_index = 0;
        }
    });
    ui.add_space(4.0);
    scroll_to_match
}

fn next_match(search: &mut DetailSearchState, count: usize) {
    if count == 0 {
        search.match_index = 0;
        return;
    }
    search.match_index = (search.match_index + 1) % count;
}

fn prev_match(search: &mut DetailSearchState, count: usize) {
    if count == 0 {
        search.match_index = 0;
        return;
    }
    search.match_index = if search.match_index == 0 {
        count - 1
    } else {
        search.match_index - 1
    };
}

fn find_matches(text: &str, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let haystack = text.to_lowercase();
    let needle = query.to_lowercase();
    let mut matches = Vec::new();
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(&needle) {
        matches.push(start + rel);
        start += rel + needle.len().max(1);
    }
    matches
}

fn iter_lines(text: &str) -> impl Iterator<Item = (usize, &str)> + '_ {
    let mut offset = 0;
    text.split('\n').map(move |line| {
        let start = offset;
        offset += line.len() + 1;
        (start, line)
    })
}

fn render_line_with_highlights(
    ui: &mut Ui,
    line: &str,
    line_start: usize,
    query: &str,
    current_match: Option<usize>,
) {
    let lower_line = line.to_lowercase();
    let lower_query = query.to_lowercase();
    let mut byte = 0;
    while byte < line.len() {
        if let Some(rel) = lower_line[byte..].find(&lower_query) {
            if rel > 0 {
                ui.add(
                    Label::new(RichText::new(&line[byte..byte + rel]).monospace()).selectable(true),
                );
            }
            let match_start = byte + rel;
            let match_end = match_start + lower_query.len();
            let global = line_start + match_start;
            let is_current = current_match == Some(global);
            let bg = if is_current {
                Theme::SEARCH_CURRENT
            } else {
                Theme::SEARCH_HIGHLIGHT
            };
            ui.add(
                Label::new(
                    RichText::new(&line[match_start..match_end])
                        .monospace()
                        .background_color(bg),
                )
                .selectable(true),
            );
            byte = match_end;
        } else {
            ui.add(Label::new(RichText::new(&line[byte..]).monospace()).selectable(true));
            break;
        }
    }
    if line.is_empty() {
        ui.add_space(ui.text_style_height(&egui::TextStyle::Monospace));
    }
}

/// Read-only multiline text that supports click-drag selection and Ctrl+C copy.
fn selectable_readonly_text(ui: &mut Ui, text: &str) {
    let mut buffer = text;
    ui.add(
        TextEdit::multiline(&mut buffer)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .interactive(true)
            .frame(false),
    );
}

fn tab_btn(ui: &mut Ui, active: &mut DetailTab, tab: DetailTab, label: &str) {
    let selected = *active == tab;
    let text = if selected {
        egui::RichText::new(label).color(Theme::ACCENT)
    } else {
        egui::RichText::new(label).color(Theme::TEXT_MUTED)
    };
    if ui.add(egui::Button::new(text).frame(false)).clicked() {
        *active = tab;
    }
}
