use egui::Ui;

use crate::ui::resource_table::RowContextAction;

pub fn show_generic_context_menu(
    ui: &mut Ui,
    row_idx: usize,
    action: &mut Option<(usize, RowContextAction)>,
    include_delete: bool,
) {
    if ui.button("Pin to favorites").clicked() {
        *action = Some((row_idx, RowContextAction::PinFavorite));
        ui.close_menu();
    }
    if ui.button("Copy kubectl edit").clicked() {
        *action = Some((row_idx, RowContextAction::Edit));
        ui.close_menu();
    }
    if include_delete && ui.button("Delete").clicked() {
        *action = Some((row_idx, RowContextAction::Delete));
        ui.close_menu();
    }
}
