use egui::Ui;
use egui_extras::{Column, TableBuilder};
use rl_core::{ResourceKind, ResourceRow};

use crate::ui::theme::Theme;

const ROW_HEIGHT: f32 = 24.0;

pub struct TableState {
    pub selected: Option<usize>,
    pub filter: String,
}

impl TableState {
    pub fn selected_name<'a>(&self, rows: &'a [ResourceRow]) -> Option<&'a str> {
        self.selected
            .and_then(|idx| rows.get(idx))
            .map(|row| row.name.as_str())
    }
}

pub struct ListHeader<'a> {
    pub kind: ResourceKind,
    pub row_count: usize,
    pub namespace: &'a str,
    pub namespaces: &'a [String],
    pub filter: &'a mut String,
    pub on_namespace: &'a mut dyn FnMut(String),
}

pub fn show_header(ui: &mut Ui, header: ListHeader<'_>) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(header.kind.label())
                .size(18.0)
                .strong()
                .color(Theme::TEXT),
        );
        ui.label(
            egui::RichText::new(format!("{} items", header.row_count))
                .color(Theme::TEXT_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new("🔍").color(Theme::TEXT_MUTED));
            ui.add(
                egui::TextEdit::singleline(header.filter)
                    .hint_text(format!("Search {}...", header.kind.label()))
                    .desired_width(200.0),
            );
            ui.label(egui::RichText::new("Namespace").color(Theme::TEXT_MUTED));
            egui::ComboBox::from_id_salt("list_namespace")
                .selected_text(header.namespace)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for ns in header.namespaces {
                        if ui.selectable_label(header.namespace == ns, ns).clicked() {
                            (header.on_namespace)(ns.clone());
                        }
                    }
                });
        });
    });
    ui.add_space(4.0);
}

pub fn show(
    ui: &mut Ui,
    kind: ResourceKind,
    rows: &[ResourceRow],
    state: &mut TableState,
    namespace: &str,
    namespaces: &[String],
    on_namespace: &mut impl FnMut(String),
) {
    let filtered_len = rows
        .iter()
        .filter(|row| row_matches_filter(row, &state.filter))
        .count();

    show_header(
        ui,
        ListHeader {
            kind,
            row_count: filtered_len,
            namespace,
            namespaces,
            filter: &mut state.filter,
            on_namespace,
        },
    );

    let filter = state.filter.to_lowercase();
    let filtered: Vec<(usize, &ResourceRow)> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| filter.is_empty() || row.name.to_lowercase().contains(&filter))
        .collect();

    if filtered.is_empty() {
        ui.add_space(20.0);
        ui.label(
            egui::RichText::new("No resources found.")
                .color(Theme::TEXT_MUTED),
        );
        return;
    }

    if let Some(selected) = state.selected {
        if selected >= rows.len() || filtered.iter().all(|(idx, _)| *idx != selected) {
            state.selected = None;
        }
    }

    let show_metrics = kind == ResourceKind::Pod;

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(180.0))
        .column(Column::auto().at_least(100.0))
        .column(Column::auto().at_least(70.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(50.0))
        .column(Column::auto().at_least(80.0))
        .header(26.0, |mut header| {
            header.col(|ui| header_cell(ui, "Name"));
            header.col(|ui| header_cell(ui, "Namespace"));
            header.col(|ui| header_cell(ui, if show_metrics { "Containers" } else { "Ready" }));
            if show_metrics {
                header.col(|ui| header_cell(ui, "CPU"));
                header.col(|ui| header_cell(ui, "Memory"));
            } else {
                header.col(|ui| header_cell(ui, ""));
                header.col(|ui| header_cell(ui, ""));
            }
            header.col(|ui| header_cell(ui, "Restarts"));
            header.col(|ui| header_cell(ui, "Controlled By"));
            header.col(|ui| header_cell(ui, "Age"));
            header.col(|ui| header_cell(ui, "Status"));
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, filtered.len(), |mut row| {
                let row_index = row.index();
                let (original_idx, resource) = filtered[row_index];
                let selected = state.selected == Some(original_idx);

                row.col(|ui| name_cell(ui, &resource.name, selected, || {
                    state.selected = Some(original_idx);
                }));
                row.col(|ui| {
                    ui.label(egui::RichText::new(&resource.namespace).color(Theme::LINK));
                });
                row.col(|ui| {
                    ui.label(&resource.ready);
                });
                if show_metrics {
                    row.col(|ui| {
                        ui.label(
                            egui::RichText::new(&resource.cpu).color(Theme::TEXT_MUTED),
                        );
                    });
                    row.col(|ui| {
                        ui.label(
                            egui::RichText::new(&resource.memory).color(Theme::TEXT_MUTED),
                        );
                    });
                } else {
                    row.col(|ui| {
                        ui.label("");
                    });
                    row.col(|ui| {
                        ui.label("");
                    });
                }
                row.col(|ui| {
                    ui.label(&resource.restarts);
                });
                row.col(|ui| {
                    ui.label(
                        egui::RichText::new(&resource.controlled_by).color(Theme::LINK),
                    );
                });
                row.col(|ui| {
                    ui.label(&resource.age);
                });
                row.col(|ui| {
                    ui.label(
                        egui::RichText::new(&resource.status)
                            .color(Theme::status_color(&resource.status)),
                    );
                });
            });
        });
}

fn row_matches_filter(row: &ResourceRow, filter: &str) -> bool {
    filter.is_empty() || row.name.to_lowercase().contains(&filter.to_lowercase())
}

fn header_cell(ui: &mut Ui, text: &str) {
    if text.is_empty() {
        ui.label("");
    } else {
        ui.label(egui::RichText::new(text).strong().color(Theme::TEXT_MUTED));
    }
}

fn name_cell(ui: &mut Ui, name: &str, selected: bool, mut on_click: impl FnMut()) {
    let label = egui::RichText::new(name).color(Theme::LINK);
    let resp = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
    if resp.clicked() {
        on_click();
    }
    if selected {
        ui.painter().rect_filled(resp.rect, 0.0, Theme::ROW_SELECTED);
    }
}

pub fn show_overview(
    ui: &mut Ui,
    context: &str,
    namespace: &str,
    pod_rows: &[ResourceRow],
    deployment_count: usize,
    job_count: usize,
    cronjob_count: usize,
) {
    ui.add_space(12.0);
    ui.label(
        egui::RichText::new("Cluster Overview")
            .size(20.0)
            .strong()
            .color(Theme::TEXT),
    );
    ui.add_space(8.0);
    ui.label(format!("Context: {context}"));
    ui.label(format!("Namespace: {namespace}"));
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        stat_card(ui, "Pods", pod_rows.len());
        stat_card(ui, "Deployments", deployment_count);
        stat_card(ui, "Jobs", job_count);
        stat_card(ui, "Cron Jobs", cronjob_count);
        let running = pod_rows
            .iter()
            .filter(|r| r.status.eq_ignore_ascii_case("running"))
            .count();
        stat_card(ui, "Running pods", running);
    });
}

fn stat_card(ui: &mut Ui, label: &str, value: usize) {
    egui::Frame::new()
        .fill(Theme::PANEL_ELEVATED)
        .stroke(egui::Stroke::new(1.0, Theme::BORDER))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.label(egui::RichText::new(label).small().color(Theme::TEXT_MUTED));
            ui.label(
                egui::RichText::new(value.to_string())
                    .size(22.0)
                    .strong()
                    .color(Theme::ACCENT),
            );
        });
    ui.add_space(8.0);
}
