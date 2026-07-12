use egui::Ui;
use rl_core::ContainerInfo;

use crate::ui::resource_table::RowContextAction;
use crate::ui::theme::Theme;

pub fn show_pod_context_menu(
    ui: &mut Ui,
    row_idx: usize,
    pod_name: &str,
    containers: &[ContainerInfo],
    action: &mut Option<(usize, RowContextAction)>,
    on_open: &mut impl FnMut(&str),
) {
    on_open(pod_name);

    ui.set_min_width(200.0);

    attach_submenu(ui, row_idx, containers, action);
    shell_submenu(ui, row_idx, containers, action);
    logs_submenu(ui, row_idx, containers, action);

    ui.separator();

    if menu_item(ui, "Edit", "✎").clicked() {
        *action = Some((row_idx, RowContextAction::Edit));
        ui.close_menu();
    }
    if menu_item(ui, "Delete", "🗑").clicked() {
        *action = Some((row_idx, RowContextAction::Delete));
        ui.close_menu();
    }
    if menu_item(ui, "Force Delete", "🗑").clicked() {
        *action = Some((row_idx, RowContextAction::ForceDelete));
        ui.close_menu();
    }
}

fn attach_submenu(
    ui: &mut Ui,
    row_idx: usize,
    containers: &[ContainerInfo],
    action: &mut Option<(usize, RowContextAction)>,
) {
    ui.menu_button(menu_label("⌕", "Attach to Pod"), |ui| {
        pick_container(ui, row_idx, containers, action, |container| {
            RowContextAction::Attach { container }
        });
    });
}

fn shell_submenu(
    ui: &mut Ui,
    row_idx: usize,
    containers: &[ContainerInfo],
    action: &mut Option<(usize, RowContextAction)>,
) {
    ui.menu_button(menu_label(">_", "Shell"), |ui| {
        pick_container(ui, row_idx, containers, action, |container| {
            RowContextAction::Shell { container }
        });
    });
}

fn logs_submenu(
    ui: &mut Ui,
    row_idx: usize,
    containers: &[ContainerInfo],
    action: &mut Option<(usize, RowContextAction)>,
) {
    ui.menu_button(menu_label("☰", "Logs"), |ui| {
        pick_container(ui, row_idx, containers, action, |container| {
            RowContextAction::Logs { container }
        });
    });
}

fn pick_container(
    ui: &mut Ui,
    row_idx: usize,
    containers: &[ContainerInfo],
    action: &mut Option<(usize, RowContextAction)>,
    make_action: impl Fn(Option<String>) -> RowContextAction,
) {
    ui.set_min_width(180.0);
    if containers.is_empty() {
        ui.label(
            egui::RichText::new("Loading containers…")
                .color(Theme::TEXT_MUTED),
        );
        return;
    }
    if ui.button("Default container").clicked() {
        *action = Some((row_idx, make_action(None)));
        ui.close_menu();
    }
    ui.separator();
    for c in containers {
        let label = if c.ready {
            format!("{}  ✓", c.name)
        } else {
            format!("{}  (not ready)", c.name)
        };
        if ui.button(label).clicked() {
            *action = Some((row_idx, make_action(Some(c.name.clone()))));
            ui.close_menu();
        }
    }
}

fn menu_item(ui: &mut Ui, label: &str, icon: &str) -> egui::Response {
    ui.add(
        egui::Button::new(menu_label(icon, label))
            .fill(Theme::PANEL_ELEVATED)
            .stroke(egui::Stroke::NONE)
            .min_size(egui::vec2(ui.available_width(), 28.0)),
    )
}

fn menu_label(icon: &str, label: &str) -> egui::RichText {
    egui::RichText::new(format!("{icon}  {label}")).color(Theme::TEXT)
}
