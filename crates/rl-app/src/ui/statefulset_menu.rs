use egui::Ui;

use crate::ui::resource_table::RowContextAction;
use crate::ui::theme::Theme;

pub fn show_statefulset_context_menu(
    ui: &mut Ui,
    row_idx: usize,
    action: &mut Option<(usize, RowContextAction)>,
) {
    ui.set_min_width(200.0);

    if menu_item(ui, "Restart").clicked() {
        *action = Some((row_idx, RowContextAction::Restart));
        ui.close_menu();
    }
    if menu_item(ui, "Scale").clicked() {
        *action = Some((row_idx, RowContextAction::Scale));
        ui.close_menu();
    }
    if menu_item(ui, "Edit").clicked() {
        *action = Some((row_idx, RowContextAction::Edit));
        ui.close_menu();
    }
    if menu_item(ui, "Delete").clicked() {
        *action = Some((row_idx, RowContextAction::Delete));
        ui.close_menu();
    }
    if menu_item(ui, "Pin to favorites").clicked() {
        *action = Some((row_idx, RowContextAction::PinFavorite));
        ui.close_menu();
    }
}

fn menu_item(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(Theme::TEXT))
            .fill(Theme::PANEL_ELEVATED)
            .stroke(egui::Stroke::NONE)
            .min_size(egui::vec2(ui.available_width(), 28.0)),
    )
}
