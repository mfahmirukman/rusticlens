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
    pub filter_match_index: usize,
    pub focus_filter: bool,
    pub save_dialog_open: bool,
    pub save_path: String,
}

impl Default for LogPanelState {
    fn default() -> Self {
        Self {
            word_wrap: true,
            show_timestamps: false,
            log_filter: String::new(),
            follow_tail: true,
            filter_match_index: 0,
            focus_filter: false,
            save_dialog_open: false,
            save_path: String::new(),
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
    pub export_logs: bool,
    pub status_message: Option<String>,
}

pub fn show_tab_bar(
    ui: &mut Ui,
    tabs: &[LogTab],
    active_id: Option<u64>,
    active_context: &str,
) -> LogPanelAction {
    let mut action = LogPanelAction::default();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for tab in tabs {
            let selected = active_id == Some(tab.id);
            let title = if tab.context == active_context {
                truncate_tab_title(&tab.pod_name)
            } else {
                format!(
                    "{} - {}",
                    truncate_context_label(&tab.context),
                    truncate_tab_title(&tab.pod_name)
                )
            };

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
                            egui::RichText::new("x")
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
    tab: Option<&mut LogTab>,
) -> LogContentAction {
    let mut action = LogContentAction::default();

    let Some(tab) = tab else {
        ui.label(
            egui::RichText::new("Double-click a pod or use the context menu to open logs.")
                .color(Theme::TEXT_MUTED),
        );
        return action;
    };

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
                egui::RichText::new("Loading older logs...")
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
            if ui
                .small_button("Save")
                .on_hover_text("Save log lines to a file")
                .clicked()
            {
                state.save_dialog_open = true;
                if state.save_path.is_empty() {
                    state.save_path = default_log_save_path(&tab.pod_name);
                }
            }
            if ui
                .small_button("Copy")
                .on_hover_text("Copy all log lines to clipboard")
                .clicked()
            {
                action.export_logs = true;
            }
            ui.checkbox(&mut state.show_timestamps, "Show timestamps");
            ui.checkbox(&mut state.word_wrap, "Word wrap");
            ui.checkbox(&mut state.follow_tail, "Follow tail");
            let filter_response = ui.add(
                egui::TextEdit::singleline(&mut state.log_filter)
                    .hint_text("Search logs (/)...")
                    .desired_width(140.0),
            );
            if state.focus_filter {
                filter_response.request_focus();
                state.focus_filter = false;
            }
            if filter_response.changed() {
                state.filter_match_index = 0;
                tab.scroll_to_match_row = Some(0);
            }
            if filter_response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let match_count = tab.matching_indices(&state.log_filter.to_lowercase()).len();
                if ui.input(|i| i.modifiers.shift) {
                    prev_log_match(state, match_count);
                } else {
                    next_log_match(state, match_count);
                }
                tab.scroll_to_match_row = Some(state.filter_match_index);
            }
        });
    });

    if !state.log_filter.is_empty() {
        let matches = tab.matching_indices(&state.log_filter.to_lowercase());
        if state.filter_match_index >= matches.len() && !matches.is_empty() {
            state.filter_match_index = matches.len() - 1;
        }
        ui.horizontal(|ui| {
            if matches.is_empty() {
                ui.label(
                    egui::RichText::new("No matches")
                        .small()
                        .color(Theme::TEXT_MUTED),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!(
                        "{} / {}",
                        state.filter_match_index + 1,
                        matches.len()
                    ))
                    .small()
                    .color(Theme::TEXT_MUTED),
                );
                if ui.small_button("^").on_hover_text("Previous (Shift+Enter)").clicked() {
                    prev_log_match(state, matches.len());
                    tab.scroll_to_match_row = Some(state.filter_match_index);
                }
                if ui.small_button("v").on_hover_text("Next (Enter)").clicked() {
                    next_log_match(state, matches.len());
                    tab.scroll_to_match_row = Some(state.filter_match_index);
                }
            }
            if ui.small_button("Clr").on_hover_text("Clear search").clicked() {
                state.log_filter.clear();
                state.filter_match_index = 0;
            }
        });
    }

    if state.save_dialog_open {
        let mut close_save = false;
        egui::Window::new("Save logs")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.label("File path:");
                ui.add(
                    egui::TextEdit::singleline(&mut state.save_path)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close_save = true;
                    }
                    if ui.button("Save").clicked() {
                        let text = tab.lines().join("\n");
                        match std::fs::write(&state.save_path, &text) {
                            Ok(()) => {
                                action.status_message = Some(format!(
                                    "Saved {} line(s) to {}",
                                    tab.line_count(),
                                    state.save_path
                                ));
                                close_save = true;
                            }
                            Err(err) => {
                                action.status_message =
                                    Some(format!("Failed to save logs: {err}"));
                            }
                        }
                    }
                });
            });
        if close_save {
            state.save_dialog_open = false;
        }
    }

    ui.add_space(2.0);

    let wrap_mode = if state.word_wrap {
        TextWrapMode::Wrap
    } else {
        TextWrapMode::Extend
    };

    let log_area_height = ui.available_height().max(0.0);
    let filter = state.log_filter.to_lowercase();
    let row_pitch = log_row_pitch(ui);

    ui.allocate_ui(egui::vec2(ui.available_width(), log_area_height), |ui| {
        ui.set_min_height(log_area_height);
        egui::Frame::new()
            .fill(Theme::LOG_BG)
            .inner_margin(6.0)
            .show(ui, |ui| {
                let scroll_height = ui.available_height().max(0.0);
                let lines = tab.lines();
                let filtered = tab.matching_indices(&filter);
                let total_rows = if filter.is_empty() {
                    lines.len()
                } else {
                    filtered.len()
                };

                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                let dragging = ui.input(|i| i.pointer.is_decidedly_dragging());
                // Only leave follow mode when the user scrolls up or drags the view.
                if state.follow_tail && (wheel > 0.0 || dragging) {
                    state.follow_tail = false;
                }

                let mut scroll = ScrollArea::both()
                    .id_salt(("log_scroll", tab.id))
                    .auto_shrink([false, false])
                    .animated(false)
                    .max_height(scroll_height);

                let follow = state.follow_tail && !tab.loading_older;
                if follow {
                    scroll = scroll.stick_to_bottom(true);
                }

                let scroll_out = if tab.is_empty() {
                    scroll.show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Waiting for log output...")
                                .color(Theme::TEXT_MUTED),
                        );
                    })
                } else if total_rows == 0 {
                    scroll.show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("No matching log lines.")
                                .color(Theme::TEXT_MUTED),
                        );
                    })
                } else {
                    let current_line_idx = if filter.is_empty() {
                        None
                    } else {
                        filtered.get(state.filter_match_index).copied()
                    };
                    let scroll_to_row = tab.scroll_to_match_row;
                    let scroll_out = scroll.show_rows(ui, LOG_LINE_HEIGHT, total_rows, |ui, row_range| {
                        let mut scroll_rect = None;
                        for row in row_range {
                            let line_idx = if filter.is_empty() {
                                row
                            } else {
                                filtered.get(row).copied().unwrap_or(row)
                            };
                            let line = lines.get(line_idx).map(String::as_str);
                            if let Some(line) = line {
                                let shown = if state.show_timestamps {
                                    rl_core::ops::strip_ansi_codes(line)
                                } else {
                                    rl_core::ops::strip_ansi_codes(rl_core::ops::strip_log_timestamp(
                                        line,
                                    ))
                                };
                                let is_current = current_line_idx == Some(line_idx);
                                let response = ui.horizontal_wrapped(|ui| {
                                    if filter.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&shown)
                                                    .monospace()
                                                    .color(Theme::TEXT),
                                            )
                                            .wrap_mode(wrap_mode)
                                            .selectable(true),
                                        );
                                    } else {
                                        render_log_line_with_highlights(
                                            ui,
                                            &shown,
                                            &filter,
                                            is_current,
                                            wrap_mode,
                                        );
                                    }
                                });
                                if scroll_to_row == Some(row) {
                                    scroll_rect = Some(response.response.rect);
                                }
                            }
                        }
                        if let Some(rect) = scroll_rect {
                            ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                        }
                    });
                    if scroll_to_row.is_some() {
                        tab.scroll_to_match_row = None;
                    }
                    scroll_out
                };

                let mut offset_y = scroll_out.state.offset.y;

                if tab.scroll_compensate_rows > 0 {
                    let bump = tab.scroll_compensate_rows as f32 * row_pitch;
                    tab.scroll_compensate_rows = 0;
                    let max_offset = (scroll_out.content_size.y - scroll_out.inner_rect.height())
                        .max(0.0);
                    let mut scroll_state = scroll_out.state;
                    offset_y = (scroll_state.offset.y + bump).min(max_offset);
                    scroll_state.offset.y = offset_y;
                    scroll_state.store(ui.ctx(), scroll_out.id);
                }

                let at_top = offset_y <= SCROLL_LOAD_THRESHOLD;
                let content_overflows =
                    scroll_out.content_size.y > scroll_height + SCROLL_LOAD_THRESHOLD;
                let prev_offset = tab
                    .last_scroll_offset_y
                    .unwrap_or(offset_y + SCROLL_LOAD_THRESHOLD + 1.0);
                // User scrolled up from below and hit the top edge.
                let scrolled_to_top = at_top && prev_offset > SCROLL_LOAD_THRESHOLD;

                if !at_top {
                    tab.older_fetch_armed = true;
                }

                // Content fits in the viewport: no scrollbar, but wheel-up at top still requests older lines.
                let wheel_up = ui.input(|i| i.smooth_scroll_delta.y > 0.0);
                let wheel_at_top = at_top
                    && !content_overflows
                    && wheel_up
                    && tab.older_fetch_armed;

                if tab.has_more_older
                    && !tab.loading_older
                    && !state.follow_tail
                    && (scrolled_to_top || wheel_at_top)
                {
                    if wheel_at_top {
                        tab.older_fetch_armed = false;
                    }
                    action.load_older = Some(tab.id);
                }

                tab.last_scroll_offset_y = Some(offset_y);
            });
    });

    action
}

fn log_row_pitch(ui: &Ui) -> f32 {
    LOG_LINE_HEIGHT + ui.spacing().item_spacing.y
}

fn truncate_tab_title(name: &str) -> String {
    if name.len() <= 18 {
        name.to_string()
    } else {
        format!("{}...", &name[..17])
    }
}

fn truncate_context_label(context: &str) -> String {
    let short = context.rsplit('/').next().unwrap_or(context);
    if short.len() <= 10 {
        short.to_string()
    } else {
        format!("{}...", &short[..9])
    }
}

fn next_log_match(state: &mut LogPanelState, count: usize) {
    if count == 0 {
        state.filter_match_index = 0;
        return;
    }
    state.filter_match_index = (state.filter_match_index + 1) % count;
}

fn prev_log_match(state: &mut LogPanelState, count: usize) {
    if count == 0 {
        state.filter_match_index = 0;
        return;
    }
    state.filter_match_index = if state.filter_match_index == 0 {
        count - 1
    } else {
        state.filter_match_index - 1
    };
}

fn default_log_save_path(pod_name: &str) -> String {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let safe_pod: String = pod_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    home.join(format!("rusticlens-{safe_pod}.log"))
        .to_string_lossy()
        .into_owned()
}

fn render_log_line_with_highlights(
    ui: &mut Ui,
    line: &str,
    query: &str,
    is_current: bool,
    wrap_mode: TextWrapMode,
) {
    if query.is_empty() {
        ui.add(
            egui::Label::new(egui::RichText::new(line).monospace().color(Theme::TEXT))
                .wrap_mode(wrap_mode)
                .selectable(true),
        );
        return;
    }

    let lower_line = line.to_lowercase();
    let lower_query = query.to_lowercase();
    let mut start = 0;
    let mut found = false;
    ui.horizontal_wrapped(|ui| {
        while let Some(rel) = lower_line[start..].find(&lower_query) {
            found = true;
            let match_start = start + rel;
            let match_end = match_start + query.len();
            if match_start > start {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&line[start..match_start])
                            .monospace()
                            .color(Theme::TEXT),
                    )
                    .wrap_mode(wrap_mode)
                    .selectable(true),
                );
            }
            let bg = if is_current {
                Theme::SEARCH_CURRENT
            } else {
                Theme::SEARCH_HIGHLIGHT
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&line[match_start..match_end.min(line.len())])
                        .monospace()
                        .color(Theme::TEXT)
                        .background_color(bg),
                )
                .wrap_mode(wrap_mode)
                .selectable(true),
            );
            start = match_end;
        }
        if start < line.len() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&line[start..])
                        .monospace()
                        .color(Theme::TEXT),
                )
                .wrap_mode(wrap_mode)
                .selectable(true),
            );
        }
        if !found {
            ui.add(
                egui::Label::new(egui::RichText::new(line).monospace().color(Theme::TEXT))
                    .wrap_mode(wrap_mode)
                    .selectable(true),
            );
        }
    });
}
