use egui::{Ui, WidgetText};
use rl_core::ResourceKind;

use crate::ui::theme::Theme;

pub struct SidebarState {
    pub show_overview: bool,
    pub selected_kind: ResourceKind,
    pub workloads_open: bool,
    pub config_open: bool,
    pub network_open: bool,
    pub cluster_open: bool,
    pub helm_open: bool,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            show_overview: false,
            selected_kind: ResourceKind::Pod,
            workloads_open: true,
            config_open: false,
            network_open: false,
            cluster_open: false,
            helm_open: false,
        }
    }
}

fn section_header(ui: &mut Ui, title: &str) {
    ui.label(
        egui::RichText::new(title)
            .small()
            .strong()
            .color(Theme::TEXT_MUTED),
    );
}

pub fn show(ui: &mut Ui, state: &mut SidebarState, context: &str) {
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new("⬢")
                .size(22.0)
                .color(Theme::WARNING),
        );
    });
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(truncate(context, 28))
            .small()
            .color(Theme::TEXT_MUTED),
    );
    ui.separator();

    section_header(ui, "Favorites");
    ui.label(
        egui::RichText::new("  (none)")
            .small()
            .color(Theme::TEXT_MUTED),
    );
    ui.add_space(6.0);

    egui::CollapsingHeader::new("Workloads")
        .default_open(state.workloads_open)
        .show_unindented(ui, |ui| {
            state.workloads_open = true;
            if nav_item(ui, state.show_overview, "Overview") {
                state.show_overview = true;
            }
            for kind in [
                ResourceKind::Pod,
                ResourceKind::Deployment,
                ResourceKind::StatefulSet,
                ResourceKind::Job,
                ResourceKind::CronJob,
            ] {
                let active = !state.show_overview && state.selected_kind == kind;
                if nav_item(ui, active, kind.label()) {
                    state.show_overview = false;
                    state.selected_kind = kind;
                }
            }
        });

    egui::CollapsingHeader::new("Config")
        .default_open(state.config_open)
        .show_unindented(ui, |ui| {
            state.config_open = true;
            for kind in [ResourceKind::ConfigMap, ResourceKind::Secret] {
                if nav_item(ui, state.selected_kind == kind, kind.label()) {
                    state.show_overview = false;
                    state.selected_kind = kind;
                }
            }
        });

    egui::CollapsingHeader::new("Network")
        .default_open(state.network_open)
        .show_unindented(ui, |ui| {
            state.network_open = true;
            for kind in [ResourceKind::Service, ResourceKind::Ingress] {
                if nav_item(ui, state.selected_kind == kind, kind.label()) {
                    state.show_overview = false;
                    state.selected_kind = kind;
                }
            }
            ui.label(
                egui::RichText::new("  Port Forwarding")
                    .small()
                    .color(Theme::TEXT_MUTED),
            );
        });

    egui::CollapsingHeader::new("Cluster")
        .default_open(state.cluster_open)
        .show_unindented(ui, |ui| {
            state.cluster_open = true;
            for kind in [ResourceKind::Namespace, ResourceKind::Node] {
                if nav_item(ui, state.selected_kind == kind, kind.label()) {
                    state.show_overview = false;
                    state.selected_kind = kind;
                }
            }
        });

    ui.label(
        egui::RichText::new("  Events")
            .small()
            .color(Theme::TEXT_MUTED),
    );

    egui::CollapsingHeader::new("Helm")
        .default_open(state.helm_open)
        .show_unindented(ui, |ui| {
            state.helm_open = true;
            ui.label(
                egui::RichText::new("  Charts")
                    .small()
                    .color(Theme::TEXT_MUTED),
            );
            if nav_item(
                ui,
                state.selected_kind == ResourceKind::HelmRelease,
                "Releases",
            ) {
                state.show_overview = false;
                state.selected_kind = ResourceKind::HelmRelease;
            }
        });

    if nav_item(ui, state.selected_kind == ResourceKind::Crd, "Custom Resources") {
        state.show_overview = false;
        state.selected_kind = ResourceKind::Crd;
    }
}

fn nav_item(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let text = if selected {
        egui::RichText::new(format!("  {label}")).color(Theme::ACCENT)
    } else {
        egui::RichText::new(format!("  {label}")).color(Theme::TEXT)
    };
    let resp = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                resp.rect.left_top() - egui::vec2(4.0, 0.0),
                egui::vec2(3.0, resp.rect.height()),
            ),
            0.0,
            Theme::ACCENT,
        );
    }
    resp.clicked()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

pub fn show_workload_tabs(ui: &mut Ui, state: &mut SidebarState) {
    ui.horizontal(|ui| {
        if tab_button(ui, state.show_overview, "Overview") {
            state.show_overview = true;
            state.selected_kind = ResourceKind::Pod;
        }
        for kind in ResourceKind::WORKLOAD_TABS {
            let active = !state.show_overview && state.selected_kind == kind;
            if tab_button(ui, active, kind.label().trim_end_matches('s')) {
                state.show_overview = false;
                state.selected_kind = kind;
            }
        }
    });
    ui.add_space(2.0);
    let rect = ui.max_rect();
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.bottom() - 1.0,
        egui::Stroke::new(1.0, Theme::BORDER),
    );
}

fn tab_button(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let text: WidgetText = if selected {
        egui::RichText::new(label).color(Theme::ACCENT).into()
    } else {
        egui::RichText::new(label).color(Theme::TEXT_MUTED).into()
    };
    let resp = ui.add(egui::Button::new(text).frame(false));
    if selected {
        let r = resp.rect;
        ui.painter().hline(
            r.left()..=r.right(),
            r.bottom(),
            egui::Stroke::new(2.0, Theme::ACCENT),
        );
    }
    resp.clicked()
}
