use egui::{ScrollArea, Ui};

const MAX_DISPLAY_LINES: usize = 500;

pub fn show(ui: &mut Ui, logs: &[String]) {
    let scroll_height = ui.available_height().max(0.0);
    ScrollArea::both()
        .auto_shrink([false, false])
        .max_height(scroll_height)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            if logs.is_empty() {
                ui.label("No logs. Select a pod and press L or open the Logs tab.");
                return;
            }

            let start = logs.len().saturating_sub(MAX_DISPLAY_LINES);
            for line in &logs[start..] {
                ui.monospace(line);
            }
        });
}
