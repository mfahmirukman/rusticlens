use egui::Ui;

use crate::ui::resource_table::RowContextAction;

pub fn show_service_context_menu(
    ui: &mut Ui,
    row_idx: usize,
    service_ports: &[u16],
    action: &mut Option<(usize, RowContextAction)>,
) {
    if ui.button("Pin to favorites").clicked() {
        *action = Some((row_idx, RowContextAction::PinFavorite));
        ui.close_menu();
    }
    if ui.button("Copy kubectl edit").clicked() {
        *action = Some((row_idx, RowContextAction::Edit));
        ui.close_menu();
    }
    if !service_ports.is_empty() {
        ui.separator();
        ui.label("Port forward");
        for &port in service_ports {
            if ui.button(format!("localhost:{port} → :{port}")).clicked() {
                *action = Some((row_idx, RowContextAction::PortForward { remote_port: port }));
                ui.close_menu();
            }
        }
    }
    ui.separator();
    if ui.button("Delete").clicked() {
        *action = Some((row_idx, RowContextAction::Delete));
        ui.close_menu();
    }
}
