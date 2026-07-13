use std::collections::HashSet;

use egui::{Color32, Ui};
use egui_extras::{Column, TableBuilder};
use rl_core::{ContainerInfo, ResourceKind, ResourceRow};

use crate::ui::cronjob_menu::show_cronjob_context_menu;
use crate::ui::deployment_menu::show_deployment_context_menu;
use crate::ui::pod_menu::show_pod_context_menu;
use crate::ui::theme::Theme;

const ROW_HEIGHT: f32 = 24.0;

pub struct TableState {
    pub selected: Option<usize>,
    pub checked: HashSet<usize>,
    pub filter: String,
}

impl TableState {
    pub fn selected_name<'a>(&self, rows: &'a [ResourceRow]) -> Option<&'a str> {
        self.selected
            .and_then(|idx| rows.get(idx))
            .map(|row| row.name.as_str())
    }
}

#[derive(Debug, Clone)]
pub enum RowContextAction {
    Logs { container: Option<String> },
    Shell { container: Option<String> },
    Attach { container: Option<String> },
    Edit,
    Delete,
    ForceDelete,
    Trigger,
    Suspend,
    Resume,
    Restart,
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
            egui::RichText::new(format!("{} items", header.row_count)).color(Theme::TEXT_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    kind: ResourceKind,
    rows: &[ResourceRow],
    state: &mut TableState,
    namespace: &str,
    namespaces: &[String],
    pod_containers: &[ContainerInfo],
    menu_containers_pod: Option<&str>,
    on_namespace: &mut impl FnMut(String),
    on_pod_menu_open: &mut impl FnMut(&str),
) -> Option<(usize, RowContextAction)> {
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
        ui.label(egui::RichText::new("No resources found.").color(Theme::TEXT_MUTED));
        return None;
    }

    if let Some(selected) = state.selected {
        if selected >= rows.len() || filtered.iter().all(|(idx, _)| *idx != selected) {
            state.selected = None;
        }
    }

    match kind {
        ResourceKind::Deployment => show_deployment_table(ui, &filtered, state, &mut None, false),
        ResourceKind::CronJob => show_cronjob_table(ui, &filtered, state, &mut None, false),
        ResourceKind::Pod => show_pod_table(
            ui,
            &filtered,
            state,
            pod_containers,
            menu_containers_pod,
            &mut None,
            false,
            on_pod_menu_open,
        ),
        _ => show_default_table(ui, kind, &filtered, state, &mut None, false),
    }
}

/// Jobs owned by the selected CronJob (Freelens detail strip below the main table).
pub fn show_cronjob_jobs(ui: &mut Ui, cronjob_name: &str, job_rows: &[ResourceRow]) {
    let owned: Vec<&ResourceRow> = job_rows
        .iter()
        .filter(|row| row.owner == cronjob_name)
        .collect();

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Jobs")
                .size(16.0)
                .strong()
                .color(Theme::TEXT),
        );
        ui.label(egui::RichText::new(format!("{} items", owned.len())).color(Theme::TEXT_MUTED));
    });
    ui.add_space(4.0);

    if owned.is_empty() {
        ui.label(egui::RichText::new("No jobs for this CronJob.").color(Theme::TEXT_MUTED));
        return;
    }

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(200.0))
        .column(Column::auto().at_least(100.0))
        .column(Column::auto().at_least(70.0))
        .column(Column::auto().at_least(50.0))
        .column(Column::auto().at_least(80.0))
        .header(26.0, |mut header| {
            header.col(|ui| header_cell(ui, "Name"));
            header.col(|ui| header_cell(ui, "Namespace"));
            header.col(|ui| header_cell(ui, "Completions"));
            header.col(|ui| header_cell(ui, "Age"));
            header.col(|ui| header_cell(ui, "Status"));
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, owned.len(), |mut row| {
                let resource = owned[row.index()];
                row.col(|ui| {
                    text_cell(ui, &resource.name, false, Some(Theme::LINK));
                });
                row.col(|ui| {
                    text_cell(ui, &resource.namespace, false, Some(Theme::LINK));
                });
                row.col(|ui| text_cell(ui, &resource.ready, false, None));
                row.col(|ui| text_cell(ui, &resource.age, false, None));
                row.col(|ui| status_cell(ui, &resource.status, false));
            });
        });
}

fn show_deployment_table(
    ui: &mut Ui,
    filtered: &[(usize, &ResourceRow)],
    state: &mut TableState,
    context_action: &mut Option<(usize, RowContextAction)>,
    inline_menu: bool,
) -> Option<(usize, RowContextAction)> {
    let mut action = context_action.clone();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(28.0))
        .column(Column::auto().at_least(180.0))
        .column(Column::auto().at_least(100.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(80.0))
        .column(Column::auto().at_least(70.0))
        .column(Column::auto().at_least(50.0))
        .column(Column::auto().at_least(36.0))
        .header(26.0, |mut header| {
            header.col(|ui| header_cell(ui, ""));
            header.col(|ui| header_cell(ui, "Name"));
            header.col(|ui| header_cell(ui, "Namespace"));
            header.col(|ui| header_cell(ui, "Ready"));
            header.col(|ui| header_cell(ui, "Up-to-date"));
            header.col(|ui| header_cell(ui, "Available"));
            header.col(|ui| header_cell(ui, "Age"));
            header.col(|ui| header_cell(ui, ""));
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, filtered.len(), |mut row| {
                let row_index = row.index();
                let (original_idx, resource) = filtered[row_index];
                let selected = state.selected == Some(original_idx);
                row.set_selected(selected);

                row.col(|ui| checkbox_cell(ui, original_idx, state));
                row.col(|ui| {
                    name_cell(ui, &resource.name, selected, || {
                        state.selected = Some(original_idx);
                    });
                });
                row.col(|ui| {
                    text_cell(ui, &resource.namespace, selected, Some(Theme::LINK));
                });
                row.col(|ui| text_cell(ui, &resource.ready, selected, None));
                row.col(|ui| text_cell(ui, &resource.up_to_date, selected, None));
                row.col(|ui| text_cell(ui, &resource.active, selected, None));
                row.col(|ui| text_cell(ui, &resource.age, selected, None));
                row.col(|ui| {
                    action_menu_cell(ui, original_idx, selected, &mut action, |ui, idx, act| {
                        show_deployment_context_menu(ui, idx, act);
                    });
                });

                let row_resp = row.response();
                handle_row_interaction(&row_resp, original_idx, state, &mut action);
                row_resp.context_menu(|ui| {
                    show_deployment_context_menu(ui, original_idx, &mut action);
                });
            });
        });
    if inline_menu {
        action
    } else {
        std::mem::take(&mut action)
    }
}

fn show_cronjob_table(
    ui: &mut Ui,
    filtered: &[(usize, &ResourceRow)],
    state: &mut TableState,
    context_action: &mut Option<(usize, RowContextAction)>,
    inline_menu: bool,
) -> Option<(usize, RowContextAction)> {
    let mut action = context_action.clone();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(28.0))
        .column(Column::auto().at_least(200.0))
        .column(Column::auto().at_least(100.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(70.0))
        .column(Column::auto().at_least(60.0))
        .column(Column::auto().at_least(50.0))
        .column(Column::auto().at_least(90.0))
        .column(Column::auto().at_least(50.0))
        .column(Column::auto().at_least(36.0))
        .header(26.0, |mut header| {
            header.col(|ui| header_cell(ui, ""));
            header.col(|ui| header_cell(ui, "Name"));
            header.col(|ui| header_cell(ui, "Namespace"));
            header.col(|ui| header_cell(ui, "Schedule"));
            header.col(|ui| header_cell(ui, "Timezone"));
            header.col(|ui| header_cell(ui, "Resumed"));
            header.col(|ui| header_cell(ui, "Active"));
            header.col(|ui| header_cell(ui, "Last schedule"));
            header.col(|ui| header_cell(ui, "Age"));
            header.col(|ui| header_cell(ui, ""));
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, filtered.len(), |mut row| {
                let row_index = row.index();
                let (original_idx, resource) = filtered[row_index];
                let selected = state.selected == Some(original_idx);
                let resumed = resource.resumed == "True";
                row.set_selected(selected);

                row.col(|ui| checkbox_cell(ui, original_idx, state));
                row.col(|ui| {
                    name_cell(ui, &resource.name, selected, || {
                        state.selected = Some(original_idx);
                    });
                });
                row.col(|ui| {
                    text_cell(ui, &resource.namespace, selected, Some(Theme::LINK));
                });
                row.col(|ui| text_cell(ui, &resource.schedule, selected, None));
                row.col(|ui| text_cell(ui, &resource.timezone, selected, None));
                row.col(|ui| resumed_cell(ui, &resource.resumed, selected));
                row.col(|ui| text_cell(ui, &resource.active, selected, None));
                row.col(|ui| text_cell(ui, &resource.last_schedule, selected, None));
                row.col(|ui| text_cell(ui, &resource.age, selected, None));
                row.col(|ui| {
                    action_menu_cell(ui, original_idx, selected, &mut action, |ui, idx, act| {
                        show_cronjob_context_menu(ui, idx, resumed, act);
                    });
                });

                let row_resp = row.response();
                handle_row_interaction(&row_resp, original_idx, state, &mut action);
                row_resp.context_menu(|ui| {
                    show_cronjob_context_menu(ui, original_idx, resumed, &mut action);
                });
            });
        });
    if inline_menu {
        action
    } else {
        std::mem::take(&mut action)
    }
}

#[allow(clippy::too_many_arguments)]
fn show_pod_table(
    ui: &mut Ui,
    filtered: &[(usize, &ResourceRow)],
    state: &mut TableState,
    pod_containers: &[ContainerInfo],
    menu_containers_pod: Option<&str>,
    context_action: &mut Option<(usize, RowContextAction)>,
    inline_menu: bool,
    on_pod_menu_open: &mut impl FnMut(&str),
) -> Option<(usize, RowContextAction)> {
    let mut action = context_action.clone();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
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
            header.col(|ui| header_cell(ui, "Containers"));
            header.col(|ui| header_cell(ui, "CPU"));
            header.col(|ui| header_cell(ui, "Memory"));
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
                row.set_selected(selected);

                row.col(|ui| {
                    name_cell(ui, &resource.name, selected, || {
                        state.selected = Some(original_idx);
                    });
                });
                row.col(|ui| {
                    text_cell(ui, &resource.namespace, selected, Some(Theme::LINK));
                });
                row.col(|ui| text_cell(ui, &resource.ready, selected, None));
                row.col(|ui| {
                    text_cell(ui, &resource.cpu, selected, Some(Theme::TEXT_MUTED));
                });
                row.col(|ui| {
                    text_cell(ui, &resource.memory, selected, Some(Theme::TEXT_MUTED));
                });
                row.col(|ui| text_cell(ui, &resource.restarts, selected, None));
                row.col(|ui| {
                    text_cell(ui, &resource.controlled_by, selected, Some(Theme::LINK));
                });
                row.col(|ui| text_cell(ui, &resource.age, selected, None));
                row.col(|ui| status_cell(ui, &resource.status, selected));

                let row_resp = row.response();
                if row_resp.double_clicked() {
                    state.selected = Some(original_idx);
                    action = Some((original_idx, RowContextAction::Logs { container: None }));
                } else if row_resp.clicked() {
                    state.selected = Some(original_idx);
                }

                let menu_containers = if menu_containers_pod == Some(resource.name.as_str()) {
                    pod_containers
                } else {
                    &[] as &[ContainerInfo]
                };
                row_resp.context_menu(|ui| {
                    show_pod_context_menu(
                        ui,
                        original_idx,
                        &resource.name,
                        menu_containers,
                        &mut action,
                        on_pod_menu_open,
                    );
                });
            });
        });
    if inline_menu {
        action
    } else {
        std::mem::take(&mut action)
    }
}

fn show_default_table(
    ui: &mut Ui,
    kind: ResourceKind,
    filtered: &[(usize, &ResourceRow)],
    state: &mut TableState,
    context_action: &mut Option<(usize, RowContextAction)>,
    inline_menu: bool,
) -> Option<(usize, RowContextAction)> {
    let mut action = context_action.clone();
    let show_metrics = kind == ResourceKind::Pod;
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
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
                row.set_selected(selected);

                row.col(|ui| {
                    name_cell(ui, &resource.name, selected, || {
                        state.selected = Some(original_idx);
                    });
                });
                row.col(|ui| {
                    text_cell(ui, &resource.namespace, selected, Some(Theme::LINK));
                });
                row.col(|ui| text_cell(ui, &resource.ready, selected, None));
                if show_metrics {
                    row.col(|ui| {
                        text_cell(ui, &resource.cpu, selected, Some(Theme::TEXT_MUTED));
                    });
                    row.col(|ui| {
                        text_cell(ui, &resource.memory, selected, Some(Theme::TEXT_MUTED));
                    });
                } else {
                    row.col(|ui| {
                        ui.label("");
                    });
                    row.col(|ui| {
                        ui.label("");
                    });
                }
                row.col(|ui| text_cell(ui, &resource.restarts, selected, None));
                row.col(|ui| {
                    text_cell(ui, &resource.controlled_by, selected, Some(Theme::LINK));
                });
                row.col(|ui| text_cell(ui, &resource.age, selected, None));
                row.col(|ui| status_cell(ui, &resource.status, selected));

                if row.response().clicked() {
                    state.selected = Some(original_idx);
                }
            });
        });
    if inline_menu {
        action
    } else {
        std::mem::take(&mut action)
    }
}

fn handle_row_interaction(
    row_resp: &egui::Response,
    original_idx: usize,
    state: &mut TableState,
    _action: &mut Option<(usize, RowContextAction)>,
) {
    if row_resp.clicked() {
        state.selected = Some(original_idx);
    }
}

fn checkbox_cell(ui: &mut Ui, idx: usize, state: &mut TableState) {
    let mut checked = state.checked.contains(&idx);
    if ui.checkbox(&mut checked, "").changed() {
        if checked {
            state.checked.insert(idx);
        } else {
            state.checked.remove(&idx);
        }
    }
}

fn action_menu_cell(
    ui: &mut Ui,
    row_idx: usize,
    selected: bool,
    action: &mut Option<(usize, RowContextAction)>,
    show_menu: impl FnOnce(&mut Ui, usize, &mut Option<(usize, RowContextAction)>),
) {
    let _ = selected;
    ui.menu_button(egui::RichText::new("...").size(16.0), |ui| {
        show_menu(ui, row_idx, action)
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
    let resp = if selected {
        ui.add(egui::Label::new(egui::RichText::new(name).strong()).sense(egui::Sense::click()))
    } else {
        ui.add(
            egui::Label::new(egui::RichText::new(name).color(Theme::LINK))
                .sense(egui::Sense::click()),
        )
    };
    if resp.clicked() {
        on_click();
    }
}

fn text_cell(ui: &mut Ui, text: &str, selected: bool, color: Option<Color32>) {
    if selected {
        ui.label(text);
    } else if let Some(color) = color {
        ui.label(egui::RichText::new(text).color(color));
    } else {
        ui.label(text);
    }
}

fn resumed_cell(ui: &mut Ui, resumed: &str, selected: bool) {
    let color = if resumed == "True" {
        Theme::status_color("running")
    } else {
        Theme::TEXT_MUTED
    };
    if selected {
        ui.label(resumed);
    } else {
        ui.label(egui::RichText::new(resumed).color(color));
    }
}

fn status_cell(ui: &mut Ui, status: &str, selected: bool) {
    if selected {
        ui.label(status);
    } else {
        ui.label(egui::RichText::new(status).color(Theme::status_color(status)));
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
