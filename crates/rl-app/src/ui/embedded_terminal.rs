use eframe::egui;

use crate::ui::theme::Theme;

pub struct EmbeddedTerminalState {
    pub open: bool,
    pub pod_name: String,
    pub lines: Vec<String>,
    pub input: String,
    pub running: bool,
}

impl EmbeddedTerminalState {
    pub fn new() -> Self {
        Self {
            open: false,
            pod_name: String::new(),
            lines: Vec::new(),
            input: String::new(),
            running: false,
        }
    }

    pub fn open_for_pod(&mut self, pod_name: String) {
        self.open = true;
        self.pod_name = pod_name;
        self.lines.clear();
        self.input.clear();
        self.running = false;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.running = false;
        self.lines.clear();
        self.input.clear();
    }
}

pub enum EmbeddedTerminalAction {
    None,
    Start,
    Stop,
    SendInput(Vec<u8>),
}

pub fn show(
    ctx: &egui::Context,
    state: &mut EmbeddedTerminalState,
    container: Option<&str>,
) -> EmbeddedTerminalAction {
    if !state.open {
        return EmbeddedTerminalAction::None;
    }

    let mut action = EmbeddedTerminalAction::None;
    let title = format!(
        "Shell: {}{}",
        state.pod_name,
        container.map(|c| format!(" ({c})")).unwrap_or_default()
    );
    let mut open = true;

    egui::Window::new(title)
        .default_size([640.0, 360.0])
        .resizable(true)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if !state.running {
                    if ui.button("Connect").clicked() {
                        action = EmbeddedTerminalAction::Start;
                    }
                } else if ui.button("Disconnect").clicked() {
                    action = EmbeddedTerminalAction::Stop;
                }
                if ui.button("Clear").clicked() {
                    state.lines.clear();
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &state.lines {
                        ui.label(egui::RichText::new(line).monospace().color(Theme::TEXT));
                    }
                });

            ui.separator();
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.input)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("Type command and press Enter"),
            );
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let mut bytes = state.input.clone().into_bytes();
                bytes.push(b'\n');
                state.lines.push(format!("$ {}", state.input));
                state.input.clear();
                action = EmbeddedTerminalAction::SendInput(bytes);
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && response.has_focus() {
                let mut bytes = state.input.clone().into_bytes();
                bytes.push(b'\n');
                state.lines.push(format!("$ {}", state.input));
                state.input.clear();
                action = EmbeddedTerminalAction::SendInput(bytes);
            }
        });

    if !open {
        state.close();
        action = EmbeddedTerminalAction::Stop;
    }

    action
}
