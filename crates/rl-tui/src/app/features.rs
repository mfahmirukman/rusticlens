use rl_core::{
    config::context_is_usable, spawn_kubectl_port_forward, start_port_forward, FavoriteResource,
    PortForwardInfo, ResourceKind,
};

use super::{
    ExternalRequest, InputPurpose, ListPickerState, Overlay, PendingAction, PortForwardEntry,
    PortForwardSession, SettingsCursor, TuiApp, ViewMode,
};
use crate::actions::actions_for_kind;

impl TuiApp {
    pub fn open_help(&mut self) {
        self.overlay = Some(Overlay::Help);
    }

    pub fn open_action_menu(&mut self) {
        if !self.is_connected() {
            return;
        }
        let has_selection = self.selected_row_index().is_some();
        let items = actions_for_kind(self.active_kind, has_selection);
        if items.is_empty() {
            self.status_message = "No actions for this selection.".into();
            return;
        }
        self.overlay = Some(Overlay::ActionMenu {
            items,
            selected: 0,
            filter: String::new(),
        });
    }

    pub fn open_settings(&mut self) {
        self.overlay = Some(Overlay::Settings {
            cursor: SettingsCursor::NativePortForward,
            path_selected: 0,
        });
    }

    pub fn toggle_theme(&mut self) {
        self.theme_mode = self.theme_mode.toggle();
        self.persist_ui_settings();
        self.status_message = format!("Theme: {}", self.theme_mode.as_str());
    }

    pub fn toggle_settings_item(&mut self, cursor: SettingsCursor) {
        match cursor {
            SettingsCursor::NativePortForward => {
                self.use_native_port_forward = !self.use_native_port_forward;
                self.persist_ui_settings();
                self.status_message = format!(
                    "Native port-forward: {}",
                    if self.use_native_port_forward {
                        "on"
                    } else {
                        "off"
                    }
                );
            }
            SettingsCursor::ExternalLogs => {
                self.external_logs = !self.external_logs;
                self.persist_ui_settings();
                self.status_message = format!(
                    "Logs in external terminal: {}",
                    if self.external_logs { "on" } else { "off" }
                );
            }
            SettingsCursor::Theme => {
                self.toggle_theme();
            }
            SettingsCursor::AddKubeconfigPath => {
                self.overlay = Some(Overlay::input(
                    "Extra kubeconfig path".into(),
                    String::new(),
                    InputPurpose::AddKubeconfigPath,
                    None,
                ));
            }
            SettingsCursor::ExtraKubeconfigList => {
                if let Some(Overlay::Settings { path_selected, .. }) = &self.overlay {
                    let idx = *path_selected;
                    if idx < self.extra_kubeconfig_paths.len() {
                        let removed = self.extra_kubeconfig_paths.remove(idx);
                        self.persist_ui_settings();
                        self.status_message = format!("Removed kubeconfig: {removed}");
                    }
                }
            }
            SettingsCursor::Editor => {
                self.overlay = Some(Overlay::input(
                    "Editor command (e.g. zed --wait)".into(),
                    self.editor.clone().unwrap_or_default(),
                    InputPurpose::SetEditor,
                    None,
                ));
            }
        }
    }

    pub async fn toggle_overview(&mut self) {
        match self.view_mode {
            ViewMode::Browser => {
                self.view_mode = ViewMode::Overview;
                self.refresh_dashboard().await;
            }
            ViewMode::Overview => {
                self.view_mode = ViewMode::Browser;
                self.dashboard = None;
            }
        }
    }

    pub async fn refresh_dashboard(&mut self) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        match guard.fetch_dashboard().await {
            Ok(dashboard) => {
                self.dashboard = Some(dashboard);
                self.error_message = None;
                self.status_message = "Overview refreshed".into();
            }
            Err(err) => {
                self.error_message = Some(err.user_message());
            }
        }
    }

    pub fn prompt_delete(&mut self) {
        let Some(name) = self.selected_name() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        self.overlay = Some(Overlay::Confirm {
            title: "Delete resource".into(),
            detail: format!(
                "Delete {}/{} in namespace {}?",
                self.active_kind.api_kind(),
                name,
                self.manager
                    .as_ref()
                    .map(|_| self.active_namespace.as_str())
                    .unwrap_or("?")
            ),
            action: PendingAction::Delete,
        });
    }

    pub fn prompt_scale(&mut self) {
        if !matches!(
            self.active_kind,
            ResourceKind::Deployment | ResourceKind::StatefulSet
        ) {
            self.error_message =
                Some("Scale is only available for Deployments/StatefulSets.".into());
            return;
        }
        let Some(row) = self.selected_row() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let current = row
            .ready
            .split('/')
            .next()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(1);
        self.overlay = Some(Overlay::input(
            format!("Replicas for {} (current {current})", row.name),
            current.to_string(),
            InputPurpose::ScaleReplicas,
            None,
        ));
    }

    pub async fn restart_selection(&mut self) {
        let Some(name) = self.selected_name() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        let result = match self.active_kind {
            ResourceKind::Deployment => guard.restart_deployment(&name).await,
            ResourceKind::StatefulSet => guard.restart_statefulset(&name).await,
            _ => {
                self.error_message =
                    Some("Restart is only available for Deployments/StatefulSets.".into());
                return;
            }
        };
        match result {
            Ok(()) => {
                self.status_message = format!("Restarted {name}");
                self.error_message = None;
                self.reload_rows().await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    pub fn request_apply_yaml(&mut self) {
        self.pending_external = Some(ExternalRequest::ApplyYaml);
    }

    pub fn request_edit_yaml(&mut self) {
        let Some(name) = self.selected_name() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        self.pending_external = Some(ExternalRequest::EditYaml { name });
    }

    pub async fn request_exec_shell(&mut self) {
        if self.active_kind != ResourceKind::Pod {
            self.error_message = Some("Exec is only available for pods.".into());
            return;
        }
        let Some(name) = self.selected_name() else {
            self.error_message = Some("No pod selected".into());
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };

        let guard = manager.read().await;
        match guard.pod_containers(&name).await {
            Ok(containers) if containers.len() > 1 => {
                self.overlay = Some(Overlay::Container {
                    pod_name: name,
                    containers: containers.into_iter().map(|c| c.name).collect(),
                    state: ListPickerState {
                        search: String::new(),
                        selected: 0,
                    },
                    purpose: crate::app::ContainerPickerPurpose::Exec,
                });
            }
            Ok(containers) => {
                let container = containers.first().map(|c| c.name.clone());
                self.pending_external = Some(ExternalRequest::ExecShell { name, container });
            }
            Err(err) => {
                // Still try exec without an explicit container; kubectl will pick the default.
                self.error_message = Some(format!(
                    "Could not list containers ({}); trying default container.",
                    err.user_message()
                ));
                self.pending_external = Some(ExternalRequest::ExecShell {
                    name,
                    container: None,
                });
            }
        }
    }

    pub fn prompt_port_forward(&mut self) {
        let Some(row) = self.selected_row() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let remote = row.service_ports.first().copied().unwrap_or(8080);
        self.overlay = Some(Overlay::input(
            format!(
                "Local port for {}/{} (remote default {remote})",
                self.active_kind.api_kind(),
                row.name
            ),
            remote.to_string(),
            InputPurpose::PortForwardLocal,
            Some(remote.to_string()),
        ));
    }

    pub fn open_port_forward_list(&mut self) {
        self.overlay = Some(Overlay::PortForwardList { selected: 0 });
    }

    pub fn stop_port_forward_at(&mut self, selected: usize) {
        if let Some(entry) = self.port_forward_entries.get(selected).cloned() {
            if let Some(session) = self.port_forwards.remove(&entry.id) {
                session.stop();
            }
            self.port_forward_entries.retain(|e| e.id != entry.id);
            self.status_message = format!("Stopped port-forward {}", entry.label);
        }
        if self.port_forward_entries.is_empty() {
            self.close_overlay();
        } else if let Some(Overlay::PortForwardList { selected }) = &mut self.overlay {
            *selected = (*selected).min(self.port_forward_entries.len() - 1);
        }
    }

    pub fn toggle_favorite_selection(&mut self) {
        let Some(row) = self.selected_row() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let ns = if self.active_namespace.is_empty() {
            row.namespace.clone()
        } else {
            self.active_namespace.clone()
        };
        let fav = FavoriteResource {
            kind: self.active_kind.api_kind().to_string(),
            namespace: ns,
            name: row.name.clone(),
        };
        if let Some(pos) = self.favorites.iter().position(|f| f == &fav) {
            self.favorites.remove(pos);
            self.status_message = "Removed from favorites.".into();
        } else {
            self.favorites.push(fav);
            self.status_message = "Pinned to favorites.".into();
        }
        self.persist_ui_settings();
    }

    pub fn open_favorites_picker(&mut self) {
        if self.favorites.is_empty() {
            self.status_message = "No favorites yet (press f on a resource).".into();
            return;
        }
        self.overlay = Some(Overlay::Favorites(ListPickerState {
            search: String::new(),
            selected: 0,
        }));
    }

    pub async fn confirm_favorite_picker(&mut self, state: ListPickerState) {
        let indices = self.filtered_favorite_indices(&state.search);
        let Some(&fav_idx) = indices.get(state.selected) else {
            return;
        };
        let Some(fav) = self.favorites.get(fav_idx).cloned() else {
            return;
        };
        self.close_overlay();
        self.jump_to_favorite(&fav).await;
    }

    pub async fn confirm_editor_picker(&mut self, state: ListPickerState) {
        let indices = self.filtered_editor_indices(&state.search);
        let Some(&cand_idx) = indices.get(state.selected) else {
            return;
        };
        let Some(candidate) = self.editor_candidates.get(cand_idx).cloned() else {
            return;
        };
        self.editor = Some(candidate.command.to_string());
        self.persist_ui_settings();
        self.close_overlay();
        self.status_message = format!("Editor: {} ({})", candidate.label, candidate.command);
    }

    async fn jump_to_favorite(&mut self, fav: &FavoriteResource) {
        let kind = ResourceKind::ALL
            .iter()
            .copied()
            .find(|k| k.api_kind() == fav.kind)
            .unwrap_or(ResourceKind::Pod);

        if self.active_namespace != fav.namespace {
            self.switch_to_namespace(fav.namespace.clone()).await;
        }
        self.set_kind(kind).await;
        if let Some(idx) = self.rows.iter().position(|r| r.name == fav.name) {
            let filtered = self.filtered_row_indices();
            if let Some(sel) = filtered.iter().position(|i| *i == idx) {
                self.selected = sel;
            }
        }
        self.status_message = format!("Jumped to {}/{}", fav.kind, fav.name);
    }

    pub async fn cycle_cluster_tab(&mut self, delta: i32) {
        if self.cluster_tabs.is_empty() {
            return;
        }
        let current = self.active_context.clone();
        let cur = self
            .cluster_tabs
            .iter()
            .position(|t| t == &current)
            .unwrap_or(0) as i32;
        let next = (cur + delta).rem_euclid(self.cluster_tabs.len() as i32) as usize;
        let context = self.cluster_tabs[next].clone();
        if !context_is_usable(&context) {
            self.error_message = Some(format!("Tab context \"{context}\" is not usable"));
            return;
        }
        self.switch_to_context(context).await;
    }

    pub fn add_cluster_tab(&mut self) {
        if self.active_context.is_empty() {
            return;
        }
        let ctx = self.active_context.clone();
        if !self.cluster_tabs.iter().any(|t| t == &ctx) {
            self.cluster_tabs.push(ctx);
            self.persist_ui_settings();
            self.status_message = "Cluster tab added.".into();
        } else {
            self.status_message = "Context already in tabs.".into();
        }
    }

    pub async fn close_cluster_tab(&mut self) {
        if self.cluster_tabs.len() <= 1 {
            self.status_message = "Keep at least one cluster tab.".into();
            return;
        }
        let current = self.active_context.clone();
        let idx = self
            .cluster_tabs
            .iter()
            .position(|t| t == &current)
            .unwrap_or(0);
        self.cluster_tabs.remove(idx);
        let next = self.cluster_tabs[idx.min(self.cluster_tabs.len() - 1)].clone();
        self.persist_ui_settings();
        if next != current {
            self.switch_to_context(next).await;
        }
    }

    pub async fn run_pending_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Delete => self.delete_selection().await,
            PendingAction::Scale => self.prompt_scale(),
            PendingAction::Restart => self.restart_selection().await,
            PendingAction::TriggerCronJob => self.prompt_trigger_cronjob(),
            PendingAction::SuspendCronJob => self.set_cronjob_suspended(true).await,
            PendingAction::ResumeCronJob => self.set_cronjob_suspended(false).await,
            PendingAction::StartPortForward => self.prompt_port_forward(),
            PendingAction::ToggleFavorite => self.toggle_favorite_selection(),
            PendingAction::OpenLogs => self.start_logs_for_selection().await,
            PendingAction::OpenServiceLogs => self.start_logs_for_service_selection().await,
            PendingAction::ExecShell => self.request_exec_shell().await,
            PendingAction::ApplyYaml => self.request_apply_yaml(),
            PendingAction::EditYaml => self.request_edit_yaml(),
        }
    }

    pub async fn confirm_input(
        &mut self,
        purpose: InputPurpose,
        value: String,
        extra: Option<String>,
    ) {
        match purpose {
            InputPurpose::ScaleReplicas => {
                let Ok(replicas) = value.trim().parse::<i32>() else {
                    self.error_message = Some("Invalid replica count".into());
                    return;
                };
                self.scale_selection(replicas).await;
            }
            InputPurpose::PortForwardLocal => {
                let Ok(local_port) = value.trim().parse::<u16>() else {
                    self.error_message = Some("Invalid local port".into());
                    return;
                };
                let remote_port = extra
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(local_port);
                // Ask for remote if they only entered local differently
                self.overlay = Some(Overlay::input(
                    format!("Remote port (local {local_port})"),
                    remote_port.to_string(),
                    InputPurpose::PortForwardRemote,
                    Some(local_port.to_string()),
                ));
            }
            InputPurpose::PortForwardRemote => {
                let Ok(remote_port) = value.trim().parse::<u16>() else {
                    self.error_message = Some("Invalid remote port".into());
                    return;
                };
                let Ok(local_port) = extra.as_deref().unwrap_or("0").parse::<u16>() else {
                    self.error_message = Some("Invalid local port".into());
                    return;
                };
                self.start_port_forward(local_port, remote_port).await;
            }
            InputPurpose::AddKubeconfigPath => {
                let path = value.trim().to_string();
                if path.is_empty() {
                    return;
                }
                if !self.extra_kubeconfig_paths.iter().any(|p| p == &path) {
                    self.extra_kubeconfig_paths.push(path.clone());
                    self.persist_ui_settings();
                    self.status_message = format!("Added kubeconfig: {path}");
                }
            }
            InputPurpose::SetEditor => {
                let trimmed = value.trim().to_string();
                self.editor = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.clone())
                };
                self.persist_ui_settings();
                self.status_message = match &self.editor {
                    Some(e) => format!("Editor: {e}"),
                    None => "Editor cleared (use $VISUAL/$EDITOR)".into(),
                };
            }
            InputPurpose::TriggerCronJob => {
                let job_name = value.trim().to_string();
                if job_name.is_empty() {
                    self.error_message = Some("Job name is empty".into());
                    return;
                }
                let Some(cronjob) = extra else {
                    return;
                };
                self.trigger_cronjob_named(cronjob, job_name).await;
            }
        }
    }

    async fn delete_selection(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        match guard.delete_resource(self.active_kind, &name, false).await {
            Ok(()) => {
                self.status_message = format!("Deleted {name}");
                self.error_message = None;
                self.reload_rows().await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn scale_selection(&mut self, replicas: i32) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        let result = match self.active_kind {
            ResourceKind::Deployment => guard.scale_deployment(&name, replicas).await,
            ResourceKind::StatefulSet => guard.scale_statefulset(&name, replicas).await,
            _ => return,
        };
        match result {
            Ok(()) => {
                self.status_message = format!("Scaled {name} to {replicas}");
                self.error_message = None;
                self.reload_rows().await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    pub fn prompt_trigger_cronjob(&mut self) {
        let Some(name) = self.selected_name() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        self.overlay = Some(Overlay::input(
            format!("Job name for CronJob {name}"),
            rl_core::ops::manual_job_name(&name),
            InputPurpose::TriggerCronJob,
            Some(name.to_string()),
        ));
    }

    async fn trigger_cronjob_named(&mut self, name: String, job_name: String) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        match guard.trigger_cronjob(&name, Some(&job_name)).await {
            Ok(()) => {
                self.status_message = format!("Triggered CronJob {name} as Job {job_name}");
                self.error_message = None;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn set_cronjob_suspended(&mut self, suspend: bool) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        match guard.set_cronjob_suspended(&name, suspend).await {
            Ok(()) => {
                self.status_message = format!(
                    "{} CronJob {name}",
                    if suspend { "Suspended" } else { "Resumed" }
                );
                self.error_message = None;
                self.reload_rows().await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn start_port_forward(&mut self, local_port: u16, remote_port: u16) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.clone() else {
            return;
        };
        let guard = manager.read().await;
        let kind = self.active_kind;
        let id = self.next_port_forward_id;
        self.next_port_forward_id += 1;
        let label = format!("{}/{}:{remote_port}→{local_port}", kind.api_kind(), name);
        let namespace = guard.namespace().to_string();

        if self.use_native_port_forward && kind != ResourceKind::Service {
            match start_port_forward(
                guard.client().clone(),
                guard.namespace(),
                kind,
                &name,
                local_port,
                remote_port,
                id,
            )
            .await
            {
                Ok(handle) => {
                    let info = handle.info.clone();
                    self.port_forwards
                        .insert(id, PortForwardSession::Native(handle));
                    self.port_forward_entries.push(PortForwardEntry {
                        id: info.id,
                        label: info.label,
                        local_port: info.local_port,
                        remote_port: info.remote_port,
                    });
                    self.status_message = format!("Port-forward started (native): {label}");
                    self.error_message = None;
                    return;
                }
                Err(err) => {
                    tracing::info!(
                        "native port-forward unavailable, using kubectl: {}",
                        err.user_message()
                    );
                }
            }
        }

        match spawn_kubectl_port_forward(&namespace, kind, &name, local_port, remote_port) {
            Ok(child) => {
                let info = PortForwardInfo {
                    id,
                    label: label.clone(),
                    local_port,
                    remote_port,
                };
                self.port_forwards
                    .insert(id, PortForwardSession::Kubectl(child));
                self.port_forward_entries.push(PortForwardEntry {
                    id: info.id,
                    label: info.label,
                    local_port: info.local_port,
                    remote_port: info.remote_port,
                });
                self.status_message = format!("Port-forward started (kubectl): {label}");
                self.error_message = None;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    fn selected_name(&self) -> Option<String> {
        self.selected_row().map(|r| r.name.clone())
    }

    fn selected_row(&self) -> Option<&rl_core::ResourceRow> {
        let idx = self.selected_row_index()?;
        self.rows.get(idx)
    }
}
