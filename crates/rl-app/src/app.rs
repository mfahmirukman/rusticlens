use std::collections::HashMap;

use eframe::egui;
use rl_core::{
    kubectl_edit_command, kubectl_exec_command, kubectl_port_forward_command,
    spawn_kubectl_attach_terminal, spawn_kubectl_exec_terminal, ClusterDashboard, ContainerInfo,
    CrdTarget, FavoriteResource, PortForwardInfo, ResourceKind, ResourceSnapshot,
};

use crate::backend::{BackendCommand, BackendEvent, BackendHandle};
use crate::log_info;
use crate::ui::cluster_tabs::{self, ClusterTabAction};
use crate::ui::detail_panel::{
    show_content as show_detail_content, show_header as show_detail_header, DetailSearchState,
    DetailState, DetailTab,
};
#[cfg(feature = "embedded-terminal")]
use crate::ui::embedded_terminal::{self, EmbeddedTerminalAction, EmbeddedTerminalState};
use crate::ui::icon_rail::{self, IconRailAction, IconRailState};
use crate::ui::log_panel::{show_tab_bar, show_tab_content, LogPanelState};
use crate::ui::log_tabs::LogTabsState;
use crate::ui::resource_table::{RowContextAction, TableState};
use crate::ui::settings_dialog::{self, SettingsDialogState};
use crate::ui::sidebar::SidebarState;
use crate::ui::theme::{Theme, ThemeMode};

struct ScaleDialog {
    kind: ResourceKind,
    name: String,
    replicas: u32,
}

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
    OpenEmbeddedShell,
    CopyPortForward,
    StartPortForward,
    ToggleTheme,
    ApplyYaml,
    ClusterSettings,
}

pub struct RusticlensApp {
    backend: BackendHandle,
    sidebar: SidebarState,
    table: TableState,
    detail: DetailState,
    detail_search: DetailSearchState,
    snapshots: HashMap<ResourceKind, ResourceSnapshot>,
    contexts: Vec<String>,
    pinned_contexts: Vec<String>,
    icon_rail: IconRailState,
    namespaces: Vec<String>,
    crd_targets: Vec<CrdTarget>,
    selected_crd: Option<CrdTarget>,
    containers: Vec<ContainerInfo>,
    menu_containers_pod: Option<String>,
    selected_container: Option<String>,
    active_context: String,
    active_namespace: String,
    status_message: String,
    error_message: Option<String>,
    connected: bool,
    connecting: bool,
    log_tabs: LogTabsState,
    pending_delete: Option<(ResourceKind, String, bool)>,
    delete_confirm: Option<(ResourceKind, String, bool)>,
    bottom_height: f32,
    /// When false, bottom panel uses half the window height until the user drags the divider.
    bottom_height_user_set: bool,
    palette_open: bool,
    palette_query: String,
    row_count: usize,
    log_panel: LogPanelState,
    detail_tab: DetailTab,
    detail_panel_width: f32,
    theme_mode: ThemeMode,
    scale_dialog: Option<ScaleDialog>,
    apply_yaml_open: bool,
    apply_yaml_text: String,
    settings_dialog: SettingsDialogState,
    favorites: Vec<FavoriteResource>,
    dashboard: Option<ClusterDashboard>,
    pending_favorite_select: Option<String>,
    cluster_tabs: Vec<String>,
    cluster_tab_picker_open: bool,
    port_forwards: Vec<PortForwardInfo>,
    #[cfg(feature = "embedded-terminal")]
    embedded_terminal: EmbeddedTerminalState,
}

impl RusticlensApp {
    pub fn new(backend: BackendHandle) -> Self {
        backend.send(BackendCommand::ConnectDefault);
        let settings = rl_core::load_settings();
        Self {
            backend,
            sidebar: SidebarState::default(),
            table: TableState {
                selected: None,
                checked: std::collections::HashSet::new(),
                filter: String::new(),
            },
            detail: DetailState {
                tab: DetailTab::Describe,
                yaml: String::new(),
                events: String::new(),
                metrics: String::new(),
                resource_name: String::new(),
            },
            detail_search: DetailSearchState::default(),
            snapshots: HashMap::new(),
            contexts: Vec::new(),
            pinned_contexts: settings.pinned_contexts,
            icon_rail: IconRailState::default(),
            namespaces: Vec::new(),
            crd_targets: Vec::new(),
            selected_crd: None,
            containers: Vec::new(),
            menu_containers_pod: None,
            selected_container: None,
            active_context: String::from("connecting..."),
            active_namespace: String::from("default"),
            status_message: String::from("Connecting to cluster..."),
            error_message: None,
            connected: false,
            connecting: true,
            log_tabs: LogTabsState::default(),
            pending_delete: None,
            delete_confirm: None,
            bottom_height: settings.bottom_panel_height.unwrap_or(0.0),
            bottom_height_user_set: settings.bottom_panel_height.is_some(),
            palette_open: false,
            palette_query: String::new(),
            row_count: 0,
            log_panel: LogPanelState::default(),
            detail_tab: DetailTab::Describe,
            detail_panel_width: settings.detail_panel_width.unwrap_or(420.0),
            theme_mode: ThemeMode::from_settings(),
            scale_dialog: None,
            apply_yaml_open: false,
            apply_yaml_text: String::new(),
            settings_dialog: SettingsDialogState::default(),
            favorites: settings.favorites,
            dashboard: None,
            pending_favorite_select: None,
            cluster_tabs: if settings.open_cluster_tabs.is_empty() {
                settings.last_context.map(|c| vec![c]).unwrap_or_default()
            } else {
                settings.open_cluster_tabs
            },
            cluster_tab_picker_open: false,
            port_forwards: Vec::new(),
            #[cfg(feature = "embedded-terminal")]
            embedded_terminal: EmbeddedTerminalState::new(),
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
                    self.ensure_cluster_tab(&context);
                    // Pause live streams (wrong cluster client) but keep tab buffers.
                    self.backend.send(BackendCommand::CloseAllLogs);
                    self.table.selected = None;
                    self.detail.clear();
                    if let Some(id) = self.log_tabs.active_id() {
                        self.resume_log_tab_stream(id);
                    }
                    self.status_message = format!(
                        "Connected to {} / {}",
                        self.active_context, self.active_namespace
                    );
                    self.error_message = None;
                    log_info!(
                        context = %self.active_context,
                        namespace = %self.active_namespace,
                        "connected to cluster"
                    );
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
                }
                BackendEvent::EventsLoaded { text } => {
                    self.detail.events = text;
                }
                BackendEvent::ContainersLoaded {
                    tab_id,
                    pod_name,
                    containers,
                } => {
                    if let Some(id) = tab_id {
                        let had_container =
                            self.log_tabs.tab_mut(id).and_then(|t| t.container.clone());
                        self.log_tabs.set_containers(id, containers);
                        if had_container.is_none() {
                            if let Some(tab) = self.log_tabs.tab_mut(id) {
                                let pod_name = tab.pod_name.clone();
                                let container = tab.container.clone();
                                self.backend.send(BackendCommand::StartLogs {
                                    tab_id: id,
                                    pod_name,
                                    container,
                                });
                            }
                        }
                    } else {
                        self.menu_containers_pod = Some(pod_name);
                        self.containers = containers;
                        if self.selected_container.is_none() {
                            self.selected_container = self
                                .containers
                                .iter()
                                .find(|c| c.ready)
                                .map(|c| c.name.clone())
                                .or_else(|| self.containers.first().map(|c| c.name.clone()));
                        }
                    }
                }
                BackendEvent::MetricsLoaded { text } => {
                    self.detail.metrics = text;
                }
                BackendEvent::LogLine { tab_id, line } => {
                    self.log_tabs.push_line(tab_id, line);
                }
                BackendEvent::LogError { tab_id, message } => {
                    self.log_tabs.push_line(tab_id, message);
                    self.log_tabs.set_active(tab_id);
                    self.log_tabs.set_loading_older(tab_id, false);
                }
                BackendEvent::OlderLogsLoaded {
                    tab_id,
                    prepended,
                    has_more,
                } => {
                    self.log_tabs.apply_older_logs(tab_id, prepended, has_more);
                }
                BackendEvent::ResourceDeleted { kind, name } => {
                    self.status_message = format!("Deleted {} {name}", kind.api_kind());
                    self.table.selected = None;
                    self.detail.clear();
                    self.delete_confirm = None;
                }
                BackendEvent::CronJobTriggered { name } => {
                    self.status_message = format!("Triggered CronJob {name}");
                    self.backend.send(BackendCommand::RefreshList);
                }
                BackendEvent::CronJobSuspendChanged { name, suspended } => {
                    let action = if suspended { "Suspended" } else { "Resumed" };
                    self.status_message = format!("{action} CronJob {name}");
                    self.backend.send(BackendCommand::RefreshList);
                }
                BackendEvent::DeploymentRestarted { name } => {
                    self.status_message = format!("Restarted deployment {name}");
                    self.backend.send(BackendCommand::RefreshList);
                }
                BackendEvent::StatefulSetRestarted { name } => {
                    self.status_message = format!("Restarted statefulset {name}");
                    self.backend.send(BackendCommand::RefreshList);
                }
                BackendEvent::WorkloadScaled {
                    kind,
                    name,
                    replicas,
                } => {
                    self.status_message =
                        format!("Scaled {} {name} to {replicas} replicas", kind.api_kind());
                    self.scale_dialog = None;
                    self.backend.send(BackendCommand::RefreshList);
                }
                BackendEvent::YamlApplied { resources } => {
                    self.apply_yaml_open = false;
                    self.status_message = format!("Applied: {}", resources.join(", "));
                    self.backend.send(BackendCommand::RefreshList);
                }
                BackendEvent::DashboardLoaded { dashboard } => {
                    self.dashboard = Some(dashboard);
                }
                BackendEvent::PortForwardStarted { info } => {
                    self.port_forwards.retain(|p| p.id != info.id);
                    self.port_forwards.push(info.clone());
                    self.status_message = format!(
                        "Port-forward {} → localhost:{}",
                        info.label, info.local_port
                    );
                }
                BackendEvent::PortForwardStopped { id } => {
                    self.port_forwards.retain(|p| p.id != id);
                }
                #[cfg(feature = "embedded-terminal")]
                BackendEvent::EmbeddedExecOutput { line } => {
                    self.embedded_terminal.lines.push(line);
                    self.embedded_terminal.running = true;
                }
                #[cfg(feature = "embedded-terminal")]
                BackendEvent::EmbeddedExecStopped => {
                    self.embedded_terminal.running = false;
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

    /// Local and remote ports for port-forward (from Service spec when available).
    fn port_forward_ports_for_selection(&self) -> (u16, u16) {
        let Some(idx) = self.table.selected else {
            return (8080, 80);
        };
        let Some(row) = self.current_rows().get(idx) else {
            return (8080, 80);
        };
        if let Some(&remote) = row.service_ports.first() {
            return (remote, remote);
        }
        (8080, 80)
    }

    fn start_port_forward_for_selection(&mut self, kind: ResourceKind) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let (local_port, remote_port) = self.port_forward_ports_for_selection();
        self.backend.send(BackendCommand::StartPortForward {
            kind,
            name,
            local_port,
            remote_port,
        });
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
        self.detail_tab = DetailTab::Metrics;
        self.detail.tab = DetailTab::Metrics;
    }

    fn restart_active_log_stream(&mut self, clear_lines: bool) {
        let Some(tab) = self.log_tabs.active_tab() else {
            return;
        };
        if tab.context != self.active_context {
            return;
        }
        let tab_id = tab.id;
        let pod_name = tab.pod_name.clone();
        let container = tab.container.clone();
        if clear_lines {
            self.log_tabs.clear_lines(tab_id);
        }
        self.backend.send(BackendCommand::StartLogs {
            tab_id,
            pod_name,
            container,
        });
    }

    /// Resume tailing for a tab that belongs to the active cluster (after context switch or tab focus).
    fn resume_log_tab_stream(&mut self, tab_id: u64) {
        let (pod_name, container) = {
            let Some(tab) = self.log_tabs.tab_mut(tab_id) else {
                return;
            };
            if tab.context != self.active_context {
                return;
            }
            (tab.pod_name.clone(), tab.container.clone())
        };
        if container.is_some() {
            self.backend.send(BackendCommand::StartLogs {
                tab_id,
                pod_name,
                container,
            });
        } else {
            self.backend.send(BackendCommand::FetchContainers {
                tab_id: Some(tab_id),
                pod_name,
            });
        }
    }

    fn open_pod_logs(&mut self, pod_name: String, container: Option<String>) {
        let context = self.active_context.clone();
        let namespace = self.active_namespace.clone();
        let tab_id =
            self.log_tabs
                .open_tab(context, pod_name.clone(), namespace, container.clone());
        self.backend.send(BackendCommand::FetchContainers {
            tab_id: Some(tab_id),
            pod_name: pod_name.clone(),
        });
        self.backend.send(BackendCommand::StartLogs {
            tab_id,
            pod_name,
            container,
        });
        self.persist_settings();
    }

    fn fetch_older_logs(&mut self, tab_id: u64) {
        let (context, pod_name, container, tail_loaded, can_load) = {
            let Some(tab) = self.log_tabs.tab_mut(tab_id) else {
                return;
            };
            (
                tab.context.clone(),
                tab.pod_name.clone(),
                tab.container.clone(),
                tab.line_count(),
                !tab.loading_older && tab.has_more_older,
            )
        };
        if context != self.active_context || !can_load {
            return;
        }
        log_info!(
            tab_id,
            pod = %pod_name,
            tail_loaded_lines = tail_loaded,
            "requesting older logs from cluster"
        );
        self.log_tabs.set_loading_older(tab_id, true);
        self.backend.send(BackendCommand::FetchOlderLogs {
            tab_id,
            pod_name,
            container,
            timestamps: self.log_panel.show_timestamps,
            tail_loaded,
        });
    }

    fn close_log_tab(&mut self, tab_id: u64) {
        if self.log_tabs.close_tab(tab_id).is_some() {
            self.backend.send(BackendCommand::CloseLog { tab_id });
        }
    }

    fn start_logs_for_selection(&mut self) {
        if self.sidebar.selected_kind != ResourceKind::Pod {
            self.error_message = Some("Logs are only available for pods.".to_string());
            return;
        }
        if let Some(name) = self.selected_name() {
            self.open_pod_logs(name, None);
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

    fn pin_favorite(&mut self, fav: FavoriteResource) {
        if self.favorites.iter().any(|f| f == &fav) {
            self.status_message = "Already in favorites.".into();
            return;
        }
        self.favorites.push(fav);
        let mut settings = rl_core::load_settings();
        settings.favorites = self.favorites.clone();
        let _ = rl_core::save_settings(&settings);
        self.status_message = "Pinned to favorites.".into();
    }

    fn unpin_favorite(&mut self, fav: &FavoriteResource) {
        self.favorites.retain(|f| f != fav);
        let mut settings = rl_core::load_settings();
        settings.favorites = self.favorites.clone();
        let _ = rl_core::save_settings(&settings);
        self.status_message = "Removed from favorites.".into();
    }

    fn navigate_to_favorite(&mut self, fav: &FavoriteResource) {
        let Some(kind) = ResourceKind::from_api_kind(&fav.kind) else {
            return;
        };
        self.sidebar.show_overview = false;
        self.sidebar.selected_kind = kind;
        self.backend.send(BackendCommand::SetActiveKind(kind));
        if !kind.is_cluster_scoped()
            && fav.namespace != "-"
            && fav.namespace != self.active_namespace
        {
            self.backend
                .send(BackendCommand::SetNamespace(fav.namespace.clone()));
        }
        self.pending_favorite_select = Some(fav.name.clone());
        self.backend.send(BackendCommand::RefreshList);
    }

    fn apply_sidebar_action(&mut self, action: crate::ui::sidebar::SidebarAction) {
        if let Some(fav) = action.selected_favorite {
            self.navigate_to_favorite(&fav);
        }
        if let Some(fav) = action.unpin_favorite {
            self.unpin_favorite(&fav);
        }
    }

    fn persist_bottom_height(&self) {
        let mut settings = rl_core::load_settings();
        settings.bottom_panel_height = Some(self.bottom_height);
        let _ = rl_core::save_settings(&settings);
    }

    fn persist_detail_panel_width(&self) {
        let mut settings = rl_core::load_settings();
        settings.detail_panel_width = Some(self.detail_panel_width);
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
                self.bottom_height_user_set = true;
                self.persist_bottom_height();
            }
            return true;
        }
        if resp.drag_stopped() {
            self.bottom_height_user_set = true;
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

    fn ensure_cluster_tab(&mut self, context: &str) {
        if !self.cluster_tabs.iter().any(|c| c == context) {
            self.cluster_tabs.push(context.to_string());
            self.persist_cluster_tabs();
        }
    }

    fn persist_cluster_tabs(&self) {
        let mut settings = rl_core::load_settings();
        settings.open_cluster_tabs = self.cluster_tabs.clone();
        let _ = rl_core::save_settings(&settings);
    }

    fn apply_cluster_tab_action(&mut self, action: ClusterTabAction) {
        match action {
            ClusterTabAction::None => {}
            ClusterTabAction::Select(ctx) => {
                if ctx != self.active_context {
                    self.status_message = format!("Switching to {ctx}...");
                    self.backend.send(BackendCommand::SwitchContext(ctx));
                }
            }
            ClusterTabAction::Close(ctx) => {
                self.cluster_tabs.retain(|c| c != &ctx);
                if self.cluster_tabs.is_empty() {
                    self.cluster_tabs.push(self.active_context.clone());
                }
                self.persist_cluster_tabs();
                if ctx == self.active_context {
                    let next = self.cluster_tabs.first().cloned().unwrap_or(ctx);
                    self.backend.send(BackendCommand::SwitchContext(next));
                }
            }
            ClusterTabAction::Add(ctx) => {
                self.ensure_cluster_tab(&ctx);
                self.backend.send(BackendCommand::SwitchContext(ctx));
            }
        }
    }

    fn apply_icon_rail_action(&mut self, action: IconRailAction) {
        if action.show_overview {
            self.sidebar.show_overview = true;
        }
        if let Some(ctx) = action.switch_to {
            if ctx != self.active_context {
                self.icon_rail.context_menu_open = false;
                self.status_message = format!("Switching to {ctx}...");
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
        if action.open_settings {
            self.settings_dialog.open_from_settings();
        }
    }

    fn close_detail_panel(&mut self) {
        self.table.selected = None;
        self.detail.clear();
        self.detail_search.reset();
    }

    fn on_table_selection_changed(&mut self, _kind: ResourceKind) {
        self.selected_container = None;
        self.containers.clear();
        self.fetch_yaml_for_selection();
        self.detail_tab = DetailTab::Describe;
        self.detail.tab = DetailTab::Describe;
    }

    fn handle_row_context(
        &mut self,
        ctx: &egui::Context,
        kind: ResourceKind,
        row_idx: usize,
        action: RowContextAction,
    ) {
        self.table.selected = Some(row_idx);
        match action {
            RowContextAction::Logs { container } => {
                if let Some(name) = self.selected_name() {
                    self.open_pod_logs(name, container);
                }
            }
            RowContextAction::Shell { container } => {
                if let Some(name) = self.selected_name() {
                    match spawn_kubectl_exec_terminal(
                        &self.active_namespace,
                        &name,
                        container.as_deref(),
                    ) {
                        Ok(_) => self.status_message = "Opened shell in external terminal.".into(),
                        Err(err) => self.error_message = Some(err.user_message()),
                    }
                }
            }
            RowContextAction::Attach { container } => {
                if let Some(name) = self.selected_name() {
                    match spawn_kubectl_attach_terminal(
                        &self.active_namespace,
                        &name,
                        container.as_deref(),
                    ) {
                        Ok(_) => {
                            self.status_message = "Attached to pod in external terminal.".into()
                        }
                        Err(err) => self.error_message = Some(err.user_message()),
                    }
                }
            }
            RowContextAction::Edit => {
                if let Some(name) = self.selected_name() {
                    let cmd = kubectl_edit_command(&self.active_namespace, kind.api_kind(), &name);
                    ctx.copy_text(cmd);
                    self.status_message = "Copied kubectl edit command.".into();
                }
            }
            RowContextAction::Delete => {
                if let Some(name) = self.selected_name() {
                    self.pending_delete = Some((kind, name, false));
                }
            }
            RowContextAction::ForceDelete => {
                if let Some(name) = self.selected_name() {
                    self.pending_delete = Some((kind, name, true));
                }
            }
            RowContextAction::Trigger => {
                if let Some(name) = self.selected_name() {
                    self.backend.send(BackendCommand::TriggerCronJob {
                        name: name.to_string(),
                    });
                }
            }
            RowContextAction::Suspend => {
                if let Some(name) = self.selected_name() {
                    self.backend.send(BackendCommand::SetCronjobSuspended {
                        name: name.to_string(),
                        suspend: true,
                    });
                }
            }
            RowContextAction::Resume => {
                if let Some(name) = self.selected_name() {
                    self.backend.send(BackendCommand::SetCronjobSuspended {
                        name: name.to_string(),
                        suspend: false,
                    });
                }
            }
            RowContextAction::Restart => {
                if let Some(name) = self.selected_name() {
                    match kind {
                        ResourceKind::StatefulSet => {
                            self.backend.send(BackendCommand::RestartStatefulSet {
                                name: name.to_string(),
                            });
                        }
                        _ => {
                            self.backend.send(BackendCommand::RestartDeployment {
                                name: name.to_string(),
                            });
                        }
                    }
                }
            }
            RowContextAction::Scale => {
                let replicas = self
                    .current_rows()
                    .get(row_idx)
                    .and_then(|row| parse_desired_replicas(&row.ready))
                    .unwrap_or(1);
                if let Some(name) = self.selected_name() {
                    self.scale_dialog = Some(ScaleDialog {
                        kind,
                        name,
                        replicas,
                    });
                }
            }
            RowContextAction::PinFavorite => {
                if let Some(row) = self.current_rows().get(row_idx) {
                    self.pin_favorite(FavoriteResource {
                        kind: kind.api_kind().to_string(),
                        namespace: row.namespace.clone(),
                        name: row.name.clone(),
                    });
                }
            }
            RowContextAction::PortForward { remote_port } => {
                if let Some(name) = self.selected_name() {
                    self.backend.send(BackendCommand::StartPortForward {
                        kind,
                        name,
                        local_port: remote_port,
                        remote_port,
                    });
                }
            }
        }
        ctx.request_repaint();
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
            PaletteAction {
                label: "Apply YAML".into(),
                command: PaletteCommand::ApplyYaml,
            },
            PaletteAction {
                label: if self.theme_mode == ThemeMode::Dark {
                    "Switch to light theme".into()
                } else {
                    "Switch to dark theme".into()
                },
                command: PaletteCommand::ToggleTheme,
            },
            PaletteAction {
                label: "Cluster / kubeconfig settings".into(),
                command: PaletteCommand::ClusterSettings,
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
            #[cfg(feature = "embedded-terminal")]
            actions.push(PaletteAction {
                label: "Open embedded shell".into(),
                command: PaletteCommand::OpenEmbeddedShell,
            });
            actions.push(PaletteAction {
                label: "Copy port-forward command".into(),
                command: PaletteCommand::CopyPortForward,
            });
            actions.push(PaletteAction {
                label: "Start port-forward".into(),
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
            actions.push(PaletteAction {
                label: "Start port-forward".into(),
                command: PaletteCommand::StartPortForward,
            });
        }
        actions
    }

    fn run_palette_command(&mut self, ctx: &egui::Context, command: PaletteCommand) {
        match command {
            PaletteCommand::Refresh => self.backend.send(BackendCommand::RefreshWatch),
            PaletteCommand::Describe => {
                self.fetch_yaml_for_selection();
                self.detail_tab = DetailTab::Describe;
                self.detail.tab = DetailTab::Describe;
            }
            PaletteCommand::Logs => self.start_logs_for_selection(),
            PaletteCommand::Events => {
                self.fetch_events_for_selection();
                self.detail_tab = DetailTab::Events;
                self.detail.tab = DetailTab::Events;
            }
            PaletteCommand::Metrics => self.fetch_metrics(),
            PaletteCommand::Delete => {
                let kind = self.sidebar.selected_kind;
                if let Some(name) = self.selected_name() {
                    self.pending_delete = Some((kind, name, false));
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
            #[cfg(feature = "embedded-terminal")]
            PaletteCommand::OpenEmbeddedShell => {
                if let Some(name) = self.selected_name() {
                    self.embedded_terminal.open_for_pod(name);
                }
            }
            PaletteCommand::CopyPortForward => {
                if let Some(name) = self.selected_name() {
                    let (local_port, remote_port) = self.port_forward_ports_for_selection();
                    let cmd = kubectl_port_forward_command(
                        &self.active_namespace,
                        self.sidebar.selected_kind,
                        &name,
                        local_port,
                        remote_port,
                    );
                    ctx.copy_text(cmd);
                    self.status_message = "Copied port-forward command.".into();
                }
            }
            PaletteCommand::StartPortForward => {
                self.start_port_forward_for_selection(self.sidebar.selected_kind);
            }
            PaletteCommand::ToggleTheme => {
                self.theme_mode = self.theme_mode.toggle();
                let mut settings = rl_core::load_settings();
                settings.theme = Some(self.theme_mode.as_str().to_string());
                let _ = rl_core::save_settings(&settings);
                Theme::apply_mode(ctx, self.theme_mode);
            }
            PaletteCommand::ApplyYaml => {
                self.apply_yaml_open = true;
            }
            PaletteCommand::ClusterSettings => {
                self.settings_dialog.open_from_settings();
            }
        }
        self.palette_open = false;
        self.palette_query.clear();
    }

    fn show_scale_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.scale_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut cancel = false;
        egui::Window::new("Scale workload")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(format!("{} {}", dialog.kind.api_kind(), dialog.name));
                ui.horizontal(|ui| {
                    ui.label("Replicas:");
                    ui.add(egui::DragValue::new(&mut dialog.replicas).range(0..=1000));
                });
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Scale").clicked() {
                        let cmd = match dialog.kind {
                            ResourceKind::Deployment => BackendCommand::ScaleDeployment {
                                name: dialog.name.clone(),
                                replicas: dialog.replicas as i32,
                            },
                            ResourceKind::StatefulSet => BackendCommand::ScaleStatefulSet {
                                name: dialog.name.clone(),
                                replicas: dialog.replicas as i32,
                            },
                            _ => return,
                        };
                        self.backend.send(cmd);
                    }
                });
            });
        if open && !cancel {
            self.scale_dialog = Some(dialog);
        }
    }

    fn show_apply_yaml_window(&mut self, ctx: &egui::Context) {
        if !self.apply_yaml_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Apply YAML")
            .default_size(egui::vec2(520.0, 420.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Paste one or more YAML documents (server-side apply):");
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.apply_yaml_text)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(16),
                        );
                    });
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.apply_yaml_open = false;
                    }
                    let can_apply = !self.apply_yaml_text.trim().is_empty();
                    if ui
                        .add_enabled(can_apply, egui::Button::new("Apply"))
                        .clicked()
                    {
                        self.backend.send(BackendCommand::ApplyYaml {
                            yaml: self.apply_yaml_text.clone(),
                        });
                    }
                });
            });
        if !open {
            self.apply_yaml_open = false;
        }
    }

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::K)) {
            self.palette_open = true;
        }

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F))
            && self.table.selected.is_some()
        {
            self.detail_search.open();
        }

        if self.palette_open {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && self.table.selected.is_some()
            && !ctx.wants_keyboard_input()
        {
            self.close_detail_panel();
            return;
        }

        if ctx.wants_keyboard_input() {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::R)) {
            self.backend.send(BackendCommand::RefreshWatch);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::D)) {
            self.fetch_yaml_for_selection();
            self.detail_tab = DetailTab::Describe;
            self.detail.tab = DetailTab::Describe;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::L)) {
            self.start_logs_for_selection();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.fetch_events_for_selection();
            self.detail_tab = DetailTab::Events;
            self.detail.tab = DetailTab::Events;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Slash)) && self.log_tabs.active_id().is_some() {
            self.log_panel.focus_filter = true;
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
        Theme::apply_mode(ctx, self.theme_mode);
        self.process_events();
        self.handle_keyboard(ctx);
        self.show_scale_dialog(ctx);
        self.show_apply_yaml_window(ctx);
        let settings_action = settings_dialog::show(ctx, &mut self.settings_dialog);
        if settings_action.reload_contexts {
            self.status_message = "Reloading contexts from kubeconfig...".into();
            self.backend.send(BackendCommand::Reconnect);
        }
        self.show_palette(ctx);

        egui::TopBottomPanel::top("cluster_tabs")
            .frame(egui::Frame::new().fill(Theme::PANEL).inner_margin(4.0))
            .show(ctx, |ui| {
                let contexts = self.contexts.clone();
                let tab_action = cluster_tabs::show(
                    ui,
                    &self.cluster_tabs,
                    &self.active_context,
                    &contexts,
                    &mut self.cluster_tab_picker_open,
                );
                self.apply_cluster_tab_action(tab_action);

                if !self.port_forwards.is_empty() {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new("Forwards:")
                                .small()
                                .color(Theme::TEXT_MUTED),
                        );
                        let mut stop_id = None;
                        for pf in &self.port_forwards {
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} → localhost:{}",
                                    pf.label, pf.local_port
                                ))
                                .small(),
                            );
                            if ui.small_button(format!("Stop#{}", pf.id)).clicked() {
                                stop_id = Some(pf.id);
                            }
                        }
                        if let Some(id) = stop_id {
                            self.backend.send(BackendCommand::StopPortForward { id });
                        }
                    });
                }
            });

        #[cfg(feature = "embedded-terminal")]
        {
            let term_action = embedded_terminal::show(
                ctx,
                &mut self.embedded_terminal,
                self.selected_container.as_deref(),
            );
            match term_action {
                EmbeddedTerminalAction::None => {}
                EmbeddedTerminalAction::Start => {
                    self.backend.send(BackendCommand::StartEmbeddedExec {
                        pod_name: self.embedded_terminal.pod_name.clone(),
                        container: self.selected_container.clone(),
                    });
                    self.embedded_terminal.running = true;
                }
                EmbeddedTerminalAction::Stop => {
                    self.backend.send(BackendCommand::StopEmbeddedExec);
                    self.embedded_terminal.running = false;
                }
                EmbeddedTerminalAction::SendInput(bytes) => {
                    self.backend
                        .send(BackendCommand::EmbeddedExecInput { bytes });
                }
            }
        }

        if let Some((kind, name, force)) = self.pending_delete.take() {
            self.delete_confirm = Some((kind, name, force));
        }

        if let Some((kind, name, force)) = &self.delete_confirm {
            let kind = *kind;
            let name = name.clone();
            let force = *force;
            let mut open = true;
            let title = if force {
                "Confirm force delete"
            } else {
                "Confirm delete"
            };
            egui::Window::new(title)
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{} {} {} in namespace {}?",
                        if force { "Force delete" } else { "Delete" },
                        kind.api_kind(),
                        name,
                        self.active_namespace
                    ));
                    ui.horizontal(|ui| {
                        if ui
                            .button(if force { "Force delete" } else { "Delete" })
                            .clicked()
                        {
                            self.backend.send(BackendCommand::DeleteResource {
                                kind,
                                name: name.clone(),
                                force,
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
                let favorites = self.favorites.clone();
                let sidebar_action =
                    crate::ui::sidebar::show(ui, &mut self.sidebar, &context, &favorites);
                self.apply_sidebar_action(sidebar_action);

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

        if self.sidebar.show_overview
            && self.connected
            && self.sidebar.show_overview != prev_overview
        {
            self.dashboard = None;
            self.backend.send(BackendCommand::FetchDashboard);
        }

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

        // Bottom panel: streaming logs
        const BOTTOM_PANEL_ID: &str = "bottom_panel";
        let max_bottom_h = (ctx.screen_rect().height() * 0.85).max(120.0);
        if !self.bottom_height_user_set {
            self.bottom_height = (ctx.screen_rect().height() * 0.5).clamp(100.0, max_bottom_h);
        }
        let panel_id = egui::Id::new(BOTTOM_PANEL_ID);
        let resize_id = panel_id.with("__resize");

        self.bottom_height = self.bottom_height.clamp(100.0, max_bottom_h);
        self.apply_bottom_panel_resize(ctx, resize_id, max_bottom_h);
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
                    ui.label(egui::RichText::new("Logs").color(Theme::ACCENT).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Refresh").clicked() {
                            self.backend.send(BackendCommand::RefreshWatch);
                        }
                        if ui.small_button("Ctrl+K").clicked() {
                            self.palette_open = true;
                        }
                    });
                });
                ui.separator();

                let body_height = ui.available_height().max(0.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), body_height), |ui| {
                    ui.set_min_height(body_height);
                    ui.set_max_height(body_height);

                    let active_id = self.log_tabs.active_id();
                    let bar_action =
                        show_tab_bar(ui, self.log_tabs.tabs(), active_id, &self.active_context);
                    if let Some(id) = bar_action.select_tab {
                        self.log_tabs.set_active(id);
                        self.resume_log_tab_stream(id);
                    }
                    if let Some(id) = bar_action.close_tab {
                        self.close_log_tab(id);
                    }

                    let content_action = if let Some(id) = self.log_tabs.active_id() {
                        if let Some(tab) = self.log_tabs.tab_mut(id) {
                            show_tab_content(ui, &mut self.log_panel, Some(tab))
                        } else {
                            show_tab_content(ui, &mut self.log_panel, None)
                        }
                    } else {
                        show_tab_content(ui, &mut self.log_panel, None)
                    };
                    if let Some((tab_id, container)) = content_action.container {
                        let matches_cluster = self
                            .log_tabs
                            .tab_mut(tab_id)
                            .is_some_and(|t| t.context == self.active_context);
                        if matches_cluster {
                            if let Some(tab) = self.log_tabs.tab_mut(tab_id) {
                                tab.container = Some(container.clone());
                            }
                            self.log_tabs.clear_lines(tab_id);
                            if let Some(tab) = self.log_tabs.tab_mut(tab_id) {
                                let pod_name = tab.pod_name.clone();
                                self.backend.send(BackendCommand::StartLogs {
                                    tab_id,
                                    pod_name,
                                    container: Some(container),
                                });
                            }
                        }
                    } else if content_action.restart_stream {
                        self.restart_active_log_stream(true);
                    } else if let Some(tab_id) = content_action.load_older {
                        self.fetch_older_logs(tab_id);
                    } else if content_action.export_logs {
                        if let Some(tab) = self.log_tabs.active_tab() {
                            let count = tab.line_count();
                            ctx.copy_text(tab.lines().join("\n"));
                            self.status_message =
                                format!("Copied {count} log line(s) to clipboard.");
                        }
                    } else if let Some(msg) = content_action.status_message {
                        self.status_message = msg;
                    }

                    if let Some(err) = &self.error_message {
                        ui.colored_label(Theme::ERROR, err);
                    }
                });
            });

        self.apply_bottom_panel_resize(ctx, resize_id, max_bottom_h);
        set_bottom_panel_persisted_height(ctx, panel_id, self.bottom_height);

        let show_detail =
            self.table.selected.is_some() && !self.sidebar.show_overview && self.connected;

        if show_detail {
            const DETAIL_PANEL_ID: &str = "detail_panel";
            let detail_id = egui::Id::new(DETAIL_PANEL_ID);
            let max_detail_w = (ctx.screen_rect().width() * 0.65).max(320.0);
            self.detail_panel_width = self.detail_panel_width.clamp(280.0, max_detail_w);

            let prev_detail_tab = self.detail_tab;
            egui::SidePanel::right(DETAIL_PANEL_ID)
                .resizable(true)
                .default_width(self.detail_panel_width)
                .width_range(280.0..=max_detail_w)
                .frame(
                    egui::Frame::side_top_panel(&ctx.style())
                        .fill(Theme::PANEL)
                        .inner_margin(egui::Margin::symmetric(8, 6)),
                )
                .show(ctx, |ui| {
                    let resource_name = self.detail.resource_name.clone();
                    if show_detail_header(
                        ui,
                        &mut self.detail_tab,
                        &resource_name,
                        &mut self.detail_search,
                    ) {
                        self.close_detail_panel();
                    } else {
                        self.detail.tab = self.detail_tab;
                        show_detail_content(ui, &mut self.detail, &mut self.detail_search);
                    }
                });

            if let Some(state) =
                ctx.data_mut(|d| d.get_persisted::<egui::containers::panel::PanelState>(detail_id))
            {
                let w = state.rect.width();
                if (w - self.detail_panel_width).abs() > 1.0 {
                    self.detail_panel_width = w;
                    self.persist_detail_panel_width();
                }
            }

            if self.detail_tab != prev_detail_tab {
                self.detail_search.match_index = 0;
                match self.detail_tab {
                    DetailTab::Describe => self.fetch_yaml_for_selection(),
                    DetailTab::Events => self.fetch_events_for_selection(),
                    DetailTab::Metrics => self.fetch_metrics(),
                }
            }
        }

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

                if self.connecting {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(&self.status_message);
                    });
                    ui.add_space(4.0);
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
                        self.dashboard.as_ref(),
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

                if let Some(name) = self.pending_favorite_select.take() {
                    if let Some(idx) = rows.iter().position(|r| r.name == name) {
                        self.table.selected = Some(idx);
                        self.on_table_selection_changed(kind);
                    }
                }

                let namespaces = self.namespaces.clone();
                let namespace = self.active_namespace.clone();
                let prev_selected = self.table.selected;

                let containers = self.containers.clone();
                let menu_containers_pod = self.menu_containers_pod.clone();
                let cmd_tx = self.backend.cmd_tx.clone();
                let mut pending_menu_pod = None;
                let row_context = crate::ui::resource_table::show(
                    ui,
                    kind,
                    &rows,
                    &mut self.table,
                    &namespace,
                    &namespaces,
                    &containers,
                    menu_containers_pod.as_deref(),
                    &mut |ns| self.backend.send(BackendCommand::SetNamespace(ns)),
                    &mut |pod_name| {
                        pending_menu_pod = Some(pod_name.to_string());
                        let _ = cmd_tx.send(BackendCommand::FetchContainers {
                            tab_id: None,
                            pod_name: pod_name.to_string(),
                        });
                    },
                );
                if let Some(pod) = pending_menu_pod {
                    self.menu_containers_pod = Some(pod);
                }

                if let Some((idx, action)) = row_context {
                    self.handle_row_context(ctx, kind, idx, action);
                } else if self.table.selected != prev_selected {
                    self.on_table_selection_changed(kind);
                }

                if kind == ResourceKind::CronJob {
                    if let Some(name) = self.table.selected_name(&rows) {
                        let job_rows = self
                            .snapshots
                            .get(&ResourceKind::Job)
                            .map(|s| s.rows.as_slice())
                            .unwrap_or(&[]);
                        crate::ui::resource_table::show_cronjob_jobs(ui, name, job_rows);
                    }
                }
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_settings();
        self.persist_detail_panel_width();
        for pf in self.port_forwards.drain(..) {
            self.backend
                .send(BackendCommand::StopPortForward { id: pf.id });
        }
        #[cfg(feature = "embedded-terminal")]
        self.backend.send(BackendCommand::StopEmbeddedExec);
        self.backend.send(BackendCommand::CloseAllLogs);
        self.backend.send(BackendCommand::Shutdown);
    }
}

/// egui's TopBottomPanel persists `inner_response.response.rect` (content bounds), which is
/// often shorter than the resized panel. Overwrite with the user-chosen height each frame.
fn set_bottom_panel_persisted_height(ctx: &egui::Context, panel_id: egui::Id, height: f32) {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1.0, height));
    ctx.data_mut(|d| {
        d.insert_persisted(panel_id, egui::containers::panel::PanelState { rect });
    });
}

fn parse_desired_replicas(ready: &str) -> Option<u32> {
    ready.split('/').nth(1)?.trim().parse().ok()
}
