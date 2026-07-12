use std::collections::HashMap;

use eframe::egui;
use rl_core::{
    kubectl_exec_command, kubectl_port_forward_command, spawn_kubectl_exec_terminal,
    spawn_kubectl_port_forward, ContainerInfo, CrdTarget, ResourceKind, ResourceSnapshot,
};

use crate::backend::{BackendCommand, BackendEvent, BackendHandle};
use crate::ui::detail_panel::{DetailState, DetailTab};
use crate::ui::log_panel::{LogPanelState, show as show_log_panel};
use crate::ui::resource_table::TableState;
use crate::ui::sidebar::SidebarState;
use crate::ui::icon_rail::{self, IconRailAction, IconRailState};
use crate::ui::theme::Theme;

const MAX_LOG_LINES: usize = 500;

struct PaletteAction {
    label: String,
    command: PaletteCommand,
}

#[derive(Clone, Copy)]
enum PaletteCommand {
    Refresh,
    Describe,
    Logs,
    Events,
    Metrics,
    Delete,
    CopyExec,
    OpenTerminal,
    CopyPortForward,
    StartPortForward,
}

pub struct RusticlensApp {
    backend: BackendHandle,
    sidebar: SidebarState,
    table: TableState,
    detail: DetailState,
    snapshots: HashMap<ResourceKind, ResourceSnapshot>,
    contexts: Vec<String>,
    pinned_contexts: Vec<String>,
    icon_rail: IconRailState,
    namespaces: Vec<String>,
    crd_targets: Vec<CrdTarget>,
    selected_crd: Option<CrdTarget>,
    containers: Vec<ContainerInfo>,
    selected_container: Option<String>,
    active_context: String,
    active_namespace: String,
    status_message: String,
    error_message: Option<String>,
    connected: bool,
    connecting: bool,
    logs: Vec<String>,
    logs_pod: Option<String>,
    pending_delete: Option<(ResourceKind, String)>,
    delete_confirm: Option<(ResourceKind, String)>,
    bottom_height: f32,
    palette_open: bool,
    palette_query: String,
    row_count: usize,
    log_panel: LogPanelState,
    bottom_tab: DetailTab,
}

impl RusticlensApp {
    pub fn new(backend: BackendHandle) -> Self {
        backend.send(BackendCommand::ConnectDefault);
        Self {
            backend,
            sidebar: SidebarState::default(),
            table: TableState {
                selected: None,
                filter: String::new(),
            },
            detail: DetailState {
                tab: DetailTab::Describe,
                yaml: String::new(),
                events: String::new(),
                metrics: String::new(),
                resource_name: String::new(),
            },
            snapshots: HashMap::new(),
            contexts: Vec::new(),
            pinned_contexts: rl_core::load_settings().pinned_contexts,
            icon_rail: IconRailState::default(),
            namespaces: Vec::new(),
            crd_targets: Vec::new(),
            selected_crd: None,
            containers: Vec::new(),
            selected_container: None,
            active_context: String::from("connecting..."),
            active_namespace: String::from("default"),
            status_message: String::from("Connecting to cluster..."),
            error_message: None,
            connected: false,
            connecting: true,
            logs: Vec::new(),
            logs_pod: None,
            pending_delete: None,
            delete_confirm: None,
            bottom_height: rl_core::load_settings()
                .bottom_panel_height
                .unwrap_or(220.0),
            palette_open: false,
            palette_query: String::new(),
            row_count: 0,
            log_panel: LogPanelState::default(),
            bottom_tab: DetailTab::Logs,
        }
    }

    fn process_events(&mut self) {
        for event in self.backend.drain_events() {
            match event {
                BackendEvent::Connecting => {
                    self.connecting = true;
                    self.status_message = "Connecting...".to_string();
                }
                BackendEvent::Connected {
                    context,
                    namespace,
                    contexts,
                    namespaces,
                    crd_targets,
                } => {
                    self.connecting = false;
                    self.connected = true;
                    self.active_context = context.clone();
                    self.active_namespace = namespace;
                    self.contexts = contexts;
                    self.namespaces = namespaces;
                    self.crd_targets = crd_targets;
                    self.sync_pinned_contexts(&context);
                    self.status_message = format!(
                        "Connected to {} / {}",
                        self.active_context, self.active_namespace
                    );
                    self.error_message = None;
                }
                BackendEvent::Snapshot { kind, snapshot } => {
                    self.snapshots.insert(kind, snapshot);
                    if kind == self.sidebar.selected_kind {
                        self.row_count = self.current_rows().len();
                    }
                }
                BackendEvent::YamlLoaded { name, yaml } => {
                    self.detail.resource_name = name;
                    self.detail.yaml = yaml;
                    if self.bottom_tab != DetailTab::Logs {
                        self.bottom_tab = DetailTab::Describe;
                    }
                }
                BackendEvent::EventsLoaded { text } => {
                    self.detail.events = text;
                    self.bottom_tab = DetailTab::Events;
                }
                BackendEvent::ContainersLoaded(containers) => {
                    if self.selected_container.is_none() {
                        self.selected_container = containers
                            .iter()
                            .find(|c| c.ready)
                            .map(|c| c.name.clone())
                            .or_else(|| containers.first().map(|c| c.name.clone()));
                    }
                    self.containers = containers;
                }
                BackendEvent::MetricsLoaded { text } => {
                    self.detail.metrics = text;
                    self.bottom_tab = DetailTab::Metrics;
                }
                BackendEvent::LogLine(line) => {
                    self.logs.push(line);
                    if self.logs.len() > MAX_LOG_LINES {
                        let drain = self.logs.len() - MAX_LOG_LINES;
                        self.logs.drain(0..drain);
                    }
                }
                BackendEvent::LogError(message) => {
                    self.logs.push(message);
                    self.bottom_tab = DetailTab::Logs;
                }
                BackendEvent::LogsStopped => {
                    self.logs.clear();
                    self.logs_pod = None;
                }
                BackendEvent::ResourceDeleted { kind, name } => {
                    self.status_message = format!("Deleted {} {name}", kind.api_kind());
                    self.table.selected = None;
                    self.detail.clear();
                    self.delete_confirm = None;
                }
                BackendEvent::Error(msg) => {
                    self.error_message = Some(msg);
                    self.connecting = false;
                }
            }
        }
    }

    fn current_rows(&self) -> &[rl_core::ResourceRow] {
        self.snapshots
            .get(&self.sidebar.selected_kind)
            .map(|s| s.rows.as_slice())
            .unwrap_or(&[])
    }

    fn selected_name(&self) -> Option<String> {
        self.table
            .selected_name(self.current_rows())
            .map(str::to_string)
    }

    fn fetch_yaml_for_selection(&mut self) {
        let kind = self.sidebar.selected_kind;
        if let Some(name) = self.selected_name() {
            self.backend.send(BackendCommand::FetchYaml { kind, name });
        }
    }

    fn fetch_events_for_selection(&mut self) {
        let kind = self.sidebar.selected_kind;
        if let Some(name) = self.selected_name() {
            self.backend
                .send(BackendCommand::FetchEvents { kind, name });
        }
    }

    fn fetch_metrics(&mut self) {
        self.backend.send(BackendCommand::FetchMetrics);
        self.bottom_tab = DetailTab::Metrics;
    }

    fn start_logs_for_selection(&mut self) {
        if self.sidebar.selected_kind != ResourceKind::Pod {
            self.error_message = Some("Logs are only available for pods.".to_string());
            return;
        }
        if let Some(name) = self.selected_name() {
            self.logs.clear();
            self.logs_pod = Some(name.clone());
            self.bottom_tab = DetailTab::Logs;
            self.backend.send(BackendCommand::StartLogs {
                pod_name: name,
                container: self.selected_container.clone(),
            });
            self.persist_settings();
        }
    }

    fn fetch_containers_for_selection(&mut self) {
        if self.sidebar.selected_kind == ResourceKind::Pod {
            if let Some(name) = self.selected_name() {
                self.backend
                    .send(BackendCommand::FetchContainers { pod_name: name });
            }
        }
    }

    fn persist_settings(&self) {
        self.backend.send(BackendCommand::PersistSettings {
            kind: self.sidebar.selected_kind,
            container: self.selected_container.clone(),
        });
    }

    fn persist_pinned_contexts(&self) {
        let mut settings = rl_core::load_settings();
        settings.pinned_contexts = self.pinned_contexts.clone();
        let _ = rl_core::save_settings(&settings);
    }

    fn persist_bottom_height(&self) {
        let mut settings = rl_core::load_settings();
        settings.bottom_panel_height = Some(self.bottom_height);
        let _ = rl_core::save_settings(&settings);
    }

    fn apply_bottom_panel_resize(
        &mut self,
        ctx: &egui::Context,
        resize_id: egui::Id,
        max_h: f32,
    ) -> bool {
        let Some(resp) = ctx.read_response(resize_id) else {
            return false;
        };
        if !(resp.dragged() || resp.drag_stopped()) {
            return false;
        }
        let Some(pointer) = resp.interact_pointer_pos() else {
            return false;
        };
        let new_h = (ctx.screen_rect().bottom() - pointer.y).clamp(100.0, max_h);
        if (new_h - self.bottom_height).abs() > 0.5 {
            self.bottom_height = new_h;
            if resp.drag_stopped() {
                self.persist_bottom_height();
            }
            return true;
        }
        if resp.drag_stopped() {
            self.persist_bottom_height();
        }
        false
    }

    fn sync_pinned_contexts(&mut self, active: &str) {
        self.pinned_contexts
            .retain(|ctx| self.contexts.iter().any(|c| c == ctx));
        if !self.pinned_contexts.iter().any(|c| c == active) {
            self.pinned_contexts.insert(0, active.to_string());
        }
        if self.pinned_contexts.is_empty() {
            self.pinned_contexts.push(active.to_string());
        }
        self.persist_pinned_contexts();
    }

    fn apply_icon_rail_action(&mut self, action: IconRailAction) {
        if action.show_overview {
            self.sidebar.show_overview = true;
        }
        if let Some(ctx) = action.switch_to {
            if ctx != self.active_context {
                self.backend.send(BackendCommand::SwitchContext(ctx));
            }
        }
        if let Some(ctx) = action.pin {
            if !self.pinned_contexts.iter().any(|c| c == &ctx) {
                self.pinned_contexts.push(ctx);
                self.persist_pinned_contexts();
            }
        }
        if let Some(ctx) = action.unpin {
            if self.pinned_contexts.len() > 1 {
                self.pinned_contexts.retain(|c| c != &ctx);
                self.persist_pinned_contexts();
            }
        }
    }

    fn palette_actions(&self) -> Vec<PaletteAction> {
        let mut actions = vec![
            PaletteAction {
                label: "Refresh watches".into(),
                command: PaletteCommand::Refresh,
            },
            PaletteAction {
                label: "Describe selected".into(),
                command: PaletteCommand::Describe,
            },
            PaletteAction {
                label: "Show events".into(),
                command: PaletteCommand::Events,
            },
            PaletteAction {
                label: "Show metrics".into(),
                command: PaletteCommand::Metrics,
            },
            PaletteAction {
                label: "Delete selected".into(),
                command: PaletteCommand::Delete,
            },
        ];
        if self.sidebar.selected_kind == ResourceKind::Pod {
            actions.push(PaletteAction {
                label: "Stream pod logs".into(),
                command: PaletteCommand::Logs,
            });
            actions.push(PaletteAction {
                label: "Copy kubectl exec".into(),
                command: PaletteCommand::CopyExec,
            });
            actions.push(PaletteAction {
                label: "Open terminal (kubectl exec)".into(),
                command: PaletteCommand::OpenTerminal,
            });
            actions.push(PaletteAction {
                label: "Copy port-forward command".into(),
                command: PaletteCommand::CopyPortForward,
            });
            actions.push(PaletteAction {
                label: "Start port-forward (kubectl)".into(),
                command: PaletteCommand::StartPortForward,
            });
        }
        if self.sidebar.selected_kind.supports_port_forward()
            && self.sidebar.selected_kind != ResourceKind::Pod
        {
            actions.push(PaletteAction {
                label: "Copy port-forward command".into(),
                command: PaletteCommand::CopyPortForward,
            });
        }
        actions
    }

    fn run_palette_command(&mut self, ctx: &egui::Context, command: PaletteCommand) {
        match command {
            PaletteCommand::Refresh => self.backend.send(BackendCommand::RefreshWatch),
            PaletteCommand::Describe => self.fetch_yaml_for_selection(),
            PaletteCommand::Logs => self.start_logs_for_selection(),
            PaletteCommand::Events => self.fetch_events_for_selection(),
            PaletteCommand::Metrics => self.fetch_metrics(),
            PaletteCommand::Delete => {
                let kind = self.sidebar.selected_kind;
                if let Some(name) = self.selected_name() {
                    self.pending_delete = Some((kind, name));
                }
            }
            PaletteCommand::CopyExec => {
                if let Some(name) = self.selected_name() {
                    let cmd = kubectl_exec_command(
                        &self.active_namespace,
                        &name,
                        self.selected_container.as_deref(),
                    );
                    ctx.copy_text(cmd);
                    self.status_message = "Copied kubectl exec command.".into();
                }
            }
            PaletteCommand::OpenTerminal => {
                if let Some(name) = self.selected_name() {
                    match spawn_kubectl_exec_terminal(
                        &self.active_namespace,
                        &name,
                        self.selected_container.as_deref(),
                    ) {
                        Ok(_) => self.status_message = "Opened external terminal.".into(),
                        Err(err) => self.error_message = Some(err.user_message()),
                    }
                }
            }
            PaletteCommand::CopyPortForward => {
                if let Some(name) = self.selected_name() {
                    let cmd = kubectl_port_forward_command(
                        &self.active_namespace,
                        self.sidebar.selected_kind,
                        &name,
                        8080,
                        80,
                    );
                    ctx.copy_text(cmd);
                    self.status_message = "Copied port-forward command.".into();
                }
            }
            PaletteCommand::StartPortForward => {
                if let Some(name) = self.selected_name() {
                    match spawn_kubectl_port_forward(
                        &self.active_namespace,
                        self.sidebar.selected_kind,
                        &name,
                        8080,
                        80,
                    ) {
                        Ok(_) => {
                            self.status_message =
                                "Started kubectl port-forward on localhost:8080.".into()
                        }
                        Err(err) => self.error_message = Some(err.user_message()),
                    }
                }
            }
        }
        self.palette_open = false;
        self.palette_query.clear();
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::K)) {
            self.palette_open = true;
        }

        if self.palette_open || ctx.wants_keyboard_input() {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::R)) {
            self.backend.send(BackendCommand::RefreshWatch);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::D)) {
            self.fetch_yaml_for_selection();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::L)) {
            self.start_logs_for_selection();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.fetch_events_for_selection();
        }
    }

    fn show_palette(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }

        let mut open = true;
        egui::Window::new("Command palette")
            .open(&mut open)
            .collapsible(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label("Type to filter actions (Ctrl+K)");
                ui.text_edit_singleline(&mut self.palette_query);
                ui.separator();
                let query = self.palette_query.to_lowercase();
                for action in self.palette_actions() {
                    if !query.is_empty() && !action.label.to_lowercase().contains(&query) {
                        continue;
                    }
                    if ui.button(&action.label).clicked() {
                        let cmd = action.command;
                        self.run_palette_command(ctx, cmd);
                    }
                }
            });
        if !open {
            self.palette_open = false;
        }
    }
}

impl eframe::App for RusticlensApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        Theme::apply(ctx);
        self.process_events();
        self.handle_keyboard(ctx);
        self.show_palette(ctx);

        if let Some((kind, name)) = self.pending_delete.take() {
            self.delete_confirm = Some((kind, name));
        }

        if let Some((kind, name)) = &self.delete_confirm {
            let kind = *kind;
            let name = name.clone();
            let mut open = true;
            egui::Window::new("Confirm delete")
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Delete {} {} in namespace {}?",
                        kind.api_kind(),
                        name,
                        self.active_namespace
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            self.backend.send(BackendCommand::DeleteResource {
                                kind,
                                name: name.clone(),
                            });
                            self.delete_confirm = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.delete_confirm = None;
                        }
                    });
                });
            if !open {
                self.delete_confirm = None;
            }
        }

        // Context rail (Freelens-style cluster switcher)
        egui::SidePanel::left("icon_rail")
            .exact_width(icon_rail::rail_width())
            .resizable(false)
            .frame(egui::Frame::side_top_panel(&ctx.style()).fill(Theme::BG))
            .show(ctx, |ui| {
                let pinned = self.pinned_contexts.clone();
                let active = self.active_context.clone();
                let action = icon_rail::show(ui, &mut self.icon_rail, &pinned, &active);
                self.apply_icon_rail_action(action);
            });

        let menu_action = icon_rail::show_context_menu(
            ctx,
            &mut self.icon_rail,
            &self.contexts,
            &self.pinned_contexts,
            &self.active_context,
        );
        self.apply_icon_rail_action(menu_action);

        let prev_kind = self.sidebar.selected_kind;
        let prev_overview = self.sidebar.show_overview;

        // Navigation sidebar
        egui::SidePanel::left("sidebar")
            .default_width(210.0)
            .frame(egui::Frame::side_top_panel(&ctx.style()).fill(Theme::PANEL))
            .show(ctx, |ui| {
                let context = self.active_context.clone();
                crate::ui::sidebar::show(ui, &mut self.sidebar, &context);

                if self.sidebar.selected_kind == ResourceKind::Crd && !self.crd_targets.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("CRD type").color(Theme::TEXT_MUTED));
                    for target in self.crd_targets.clone() {
                        let selected = self.selected_crd.as_ref() == Some(&target);
                        if ui
                            .selectable_label(selected, &target.display_name)
                            .clicked()
                        {
                            self.selected_crd = Some(target.clone());
                            self.backend
                                .send(BackendCommand::SetCrdTarget(Some(target)));
                        }
                    }
                }
            });

        if (self.sidebar.selected_kind != prev_kind || self.sidebar.show_overview != prev_overview)
            && !self.sidebar.show_overview
        {
            self.table.selected = None;
            self.detail.clear();
            self.backend
                .send(BackendCommand::SetActiveKind(self.sidebar.selected_kind));
            self.backend.send(BackendCommand::RefreshList);
            self.persist_settings();
        }

        // Bottom panel: logs + describe/events/metrics
        const BOTTOM_PANEL_ID: &str = "bottom_panel";
        let max_bottom_h = (ctx.screen_rect().height() * 0.85).max(120.0);
        let panel_id = egui::Id::new(BOTTOM_PANEL_ID);
        let resize_id = panel_id.with("__resize");

        self.bottom_height = self.bottom_height.clamp(100.0, max_bottom_h);
        self.apply_bottom_panel_resize(ctx, resize_id, max_bottom_h);
        // egui stores content height in PanelState, not the dragged edge — seed before show.
        set_bottom_panel_persisted_height(ctx, panel_id, self.bottom_height);

        egui::TopBottomPanel::bottom(BOTTOM_PANEL_ID)
            .resizable(true)
            .default_height(self.bottom_height)
            .height_range(100.0..=max_bottom_h)
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(Theme::PANEL)
                    .inner_margin(egui::Margin::symmetric(6, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    tab_btn(ui, &mut self.bottom_tab, DetailTab::Logs, "Logs");
                    tab_btn(ui, &mut self.bottom_tab, DetailTab::Describe, "Describe");
                    tab_btn(ui, &mut self.bottom_tab, DetailTab::Events, "Events");
                    tab_btn(ui, &mut self.bottom_tab, DetailTab::Metrics, "Metrics");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Refresh").clicked() {
                            self.backend.send(BackendCommand::RefreshWatch);
                        }
                        if ui.small_button("⌘K").clicked() {
                            self.palette_open = true;
                        }
                    });
                });
                ui.separator();

                let body_height = ui.available_height().max(0.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), body_height), |ui| {
                    ui.set_min_height(body_height);
                    ui.set_max_height(body_height);
                    match self.bottom_tab {
                        DetailTab::Logs => {
                            let containers: Vec<(String, bool)> = self
                                .containers
                                .iter()
                                .map(|c| (c.name.clone(), c.ready))
                                .collect();
                            let pod = self.logs_pod.clone();
                            let ns = self.active_namespace.clone();
                            let container = self.selected_container.clone();
                            if let Some(name) = show_log_panel(
                                ui,
                                &mut self.log_panel,
                                pod.as_deref(),
                                &ns,
                                container.as_deref(),
                                &self.logs,
                                &containers,
                            ) {
                                self.selected_container = Some(name);
                                self.start_logs_for_selection();
                            }
                        }
                        _ => {
                            self.detail.tab = self.bottom_tab;
                            crate::ui::detail_panel::show_content(ui, &mut self.detail, &self.logs);
                        }
                    }

                    if let Some(err) = &self.error_message {
                        ui.colored_label(Theme::ERROR, err);
                    }
                });
            });

        self.apply_bottom_panel_resize(ctx, resize_id, max_bottom_h);
        set_bottom_panel_persisted_height(ctx, panel_id, self.bottom_height);

        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).fill(Theme::BG))
            .show(ctx, |ui| {
                if self.connecting && !self.connected {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                        ui.label("Connecting to cluster...");
                        ui.label("For Teleport clusters, run `tsh login` first.");
                    });
                    return;
                }

                if self.sidebar.selected_kind.category() == rl_core::ResourceCategory::Workloads
                    || self.sidebar.show_overview
                {
                    crate::ui::sidebar::show_workload_tabs(ui, &mut self.sidebar);
                    ui.add_space(6.0);
                }

                if self.sidebar.show_overview {
                    let pods = self
                        .snapshots
                        .get(&ResourceKind::Pod)
                        .map(|s| s.rows.as_slice())
                        .unwrap_or(&[]);
                    let dep_count = self
                        .snapshots
                        .get(&ResourceKind::Deployment)
                        .map(|s| s.rows.len())
                        .unwrap_or(0);
                    let job_count = self
                        .snapshots
                        .get(&ResourceKind::Job)
                        .map(|s| s.rows.len())
                        .unwrap_or(0);
                    let cronjob_count = self
                        .snapshots
                        .get(&ResourceKind::CronJob)
                        .map(|s| s.rows.len())
                        .unwrap_or(0);
                    crate::ui::resource_table::show_overview(
                        ui,
                        &self.active_context,
                        &self.active_namespace,
                        pods,
                        dep_count,
                        job_count,
                        cronjob_count,
                    );
                    return;
                }

                if self.sidebar.selected_kind == ResourceKind::Crd && self.selected_crd.is_none() {
                    ui.centered_and_justified(|ui| {
                        ui.label("Select a custom resource type from the sidebar.");
                    });
                    return;
                }

                let kind = self.sidebar.selected_kind;
                let rows = self
                    .snapshots
                    .get(&kind)
                    .map(|s| s.rows.clone())
                    .unwrap_or_default();
                self.row_count = rows.len();

                let namespaces = self.namespaces.clone();
                let namespace = self.active_namespace.clone();
                let prev_selected = self.table.selected;

                crate::ui::resource_table::show(
                    ui,
                    kind,
                    &rows,
                    &mut self.table,
                    &namespace,
                    &namespaces,
                    &mut |ns| self.backend.send(BackendCommand::SetNamespace(ns)),
                );

                if self.table.selected != prev_selected {
                    self.fetch_yaml_for_selection();
                    self.fetch_events_for_selection();
                    if kind == ResourceKind::Pod {
                        self.fetch_containers_for_selection();
                        self.start_logs_for_selection();
                        self.backend.send(BackendCommand::FetchMetrics);
                    } else {
                        self.backend.send(BackendCommand::StopLogs);
                        self.logs.clear();
                        self.logs_pod = None;
                        self.containers.clear();
                        self.selected_container = None;
                    }
                }
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_settings();
        self.backend.send(BackendCommand::Shutdown);
    }
}

fn tab_btn(ui: &mut egui::Ui, active: &mut DetailTab, tab: DetailTab, label: &str) {
    let selected = *active == tab;
    let text = if selected {
        egui::RichText::new(label).color(Theme::ACCENT)
    } else {
        egui::RichText::new(label).color(Theme::TEXT_MUTED)
    };
    if ui.add(egui::Button::new(text).frame(false)).clicked() {
        *active = tab;
    }
}

/// egui's TopBottomPanel persists `inner_response.response.rect` (content bounds), which is
/// often shorter than the resized panel. Overwrite with the user-chosen height each frame.
fn set_bottom_panel_persisted_height(ctx: &egui::Context, panel_id: egui::Id, height: f32) {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, height));
    ctx.data_mut(|d| {
        d.insert_persisted(
            panel_id,
            egui::containers::panel::PanelState { rect },
        );
    });
}
