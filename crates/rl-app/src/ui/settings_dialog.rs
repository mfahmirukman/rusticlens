use std::path::Path;

use egui::{ScrollArea, Ui};
use rl_core::{config::kubeconfig_path, load_settings, plugins_dir, save_settings};

use crate::ui::theme::Theme;

#[derive(Debug, Default)]
pub struct SettingsDialogState {
    pub open: bool,
    pub extra_paths: Vec<String>,
    pub new_path: String,
    pub focus_new_path: bool,
    pub use_native_port_forward: bool,
    pub status: Option<String>,
}

#[derive(Debug, Default)]
pub struct SettingsDialogAction {
    pub saved: bool,
    pub reload_contexts: bool,
}

impl SettingsDialogState {
    pub fn open_from_settings(&mut self) {
        self.open = true;
        self.extra_paths = load_settings().extra_kubeconfig_paths;
        self.use_native_port_forward = load_settings().use_native_port_forward;
        self.new_path.clear();
        self.status = None;
    }
}

pub fn show(ctx: &egui::Context, state: &mut SettingsDialogState) -> SettingsDialogAction {
    let mut action = SettingsDialogAction::default();
    if !state.open {
        return action;
    }

    let mut open = true;
    let mut cancel = false;
    egui::Window::new("Cluster settings")
        .collapsible(false)
        .default_width(480.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Kubeconfig")
                    .strong()
                    .color(Theme::ACCENT),
            );
            ui.label(
                egui::RichText::new(format!(
                    "Primary: {}",
                    kubeconfig_path().display()
                ))
                .small()
                .color(Theme::TEXT_MUTED),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Additional kubeconfig files are merged with the primary config (same as KUBECONFIG merge).",
                )
                .small()
                .color(Theme::TEXT_MUTED),
            );
            ui.separator();

            ui.label("Extra kubeconfig paths:");
            ScrollArea::vertical()
                .max_height(140.0)
                .show(ui, |ui| {
                    let mut remove_idx = None;
                    for (idx, path) in state.extra_paths.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let exists = Path::new(path).exists();
                            let color = if exists {
                                Theme::TEXT
                            } else {
                                Theme::WARNING
                            };
                            ui.label(egui::RichText::new(path).small().color(color));
                            if !exists {
                                ui.label(
                                    egui::RichText::new("(missing)")
                                        .small()
                                        .color(Theme::WARNING),
                                );
                            }
                            if ui.small_button("Remove").clicked() {
                                remove_idx = Some(idx);
                            }
                        });
                    }
                    if let Some(idx) = remove_idx {
                        state.extra_paths.remove(idx);
                    }
                });

            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.new_path)
                        .hint_text("/path/to/extra-kubeconfig")
                        .desired_width(ui.available_width() - 60.0),
                );
                if state.focus_new_path {
                    response.request_focus();
                    state.focus_new_path = false;
                }
                if ui.button("Add").clicked() {
                    let trimmed = state.new_path.trim();
                    if !trimmed.is_empty()
                        && !state.extra_paths.iter().any(|p| p == trimmed)
                    {
                        state.extra_paths.push(trimmed.to_string());
                        state.new_path.clear();
                    }
                }
            });

            ui.separator();

            ui.label(
                egui::RichText::new("Port-forward")
                    .strong()
                    .color(Theme::ACCENT),
            );
            ui.checkbox(
                &mut state.use_native_port_forward,
                "Use built-in port-forward (kube-rs, no kubectl subprocess)",
            );

            ui.separator();
            ui.label(
                egui::RichText::new("Plugins")
                    .strong()
                    .color(Theme::ACCENT),
            );
            ui.label(
                egui::RichText::new(format!(
                    "Drop TOML manifests in {}",
                    plugins_dir().display()
                ))
                .small()
                .color(Theme::TEXT_MUTED),
            );

            ui.separator();
            if let Some(msg) = &state.status {
                ui.label(egui::RichText::new(msg).small().color(Theme::SUCCESS));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if ui.button("Save & reload contexts").clicked() {
                    let mut settings = load_settings();
                    settings.extra_kubeconfig_paths = state.extra_paths.clone();
                    settings.use_native_port_forward = state.use_native_port_forward;
                    match save_settings(&settings) {
                        Ok(()) => {
                            state.status = Some("Saved. Reloading contexts...".into());
                            action.saved = true;
                            action.reload_contexts = true;
                            cancel = true;
                        }
                        Err(err) => {
                            state.status =
                                Some(format!("Failed to save settings: {err}"));
                        }
                    }
                }
            });
        });

    if !open || cancel {
        state.open = false;
        if !action.saved {
            state.status = None;
        }
    }

    action
}

/// Small link row for embedding in other windows.
pub fn settings_link(ui: &mut Ui) -> bool {
    ui.add(
        egui::Button::new(
            egui::RichText::new("Manage kubeconfig files...")
                .small()
                .color(Theme::LINK),
        )
        .frame(false),
    )
    .clicked()
}
