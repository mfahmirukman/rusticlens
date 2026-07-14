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
            SettingsCursor::Theme => {
                self.toggle_theme();
            }
            SettingsCursor::AddKubeconfigPath => {
                self.overlay = Some(Overlay::Input {
                    prompt: "Extra kubeconfig path".into(),
                    value: String::new(),
                    purpose: InputPurpose::AddKubeconfigPath,
                    extra: None,
                });
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
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        match manager.fetch_dashboard().await {
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
                self.manager.as_ref().map(|m| m.namespace()).unwrap_or("?")
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
        self.overlay = Some(Overlay::Input {
            prompt: format!("Replicas for {} (current {current})", row.name),
            value: current.to_string(),
            purpose: InputPurpose::ScaleReplicas,
            extra: None,
        });
    }

    pub async fn restart_selection(&mut self) {
        let Some(name) = self.selected_name() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        let result = match self.active_kind {
            ResourceKind::Deployment => manager.restart_deployment(&name).await,
            ResourceKind::StatefulSet => manager.restart_statefulset(&name).await,
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
                self.refresh().await;
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

    pub fn request_exec_shell(&mut self) {
        if self.active_kind != ResourceKind::Pod {
            self.error_message = Some("Exec is only available for pods.".into());
            return;
        }
        let Some(name) = self.selected_name() else {
            self.error_message = Some("No pod selected".into());
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        match rl_core::spawn_kubectl_exec_terminal(manager.namespace(), &name, None) {
            Ok(_) => {
                self.status_message = format!("Opened exec for {name} in a new terminal window");
                self.error_message = None;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    pub fn prompt_port_forward(&mut self) {
        let Some(row) = self.selected_row() else {
            self.error_message = Some("No resource selected".into());
            return;
        };
        let remote = row.service_ports.first().copied().unwrap_or(8080);
        self.overlay = Some(Overlay::Input {
            prompt: format!(
                "Local port for {}/{} (remote default {remote})",
                self.active_kind.api_kind(),
                row.name
            ),
            value: remote.to_string(),
            purpose: InputPurpose::PortForwardLocal,
            extra: Some(remote.to_string()),
        });
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
        let ns = self
            .manager
            .as_ref()
            .map(|m| m.namespace().to_string())
            .unwrap_or_else(|| row.namespace.clone());
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

    async fn jump_to_favorite(&mut self, fav: &FavoriteResource) {
        let kind = ResourceKind::ALL
            .iter()
            .copied()
            .find(|k| k.api_kind() == fav.kind)
            .unwrap_or(ResourceKind::Pod);

        if self.manager.as_ref().map(|m| m.namespace()) != Some(fav.namespace.as_str()) {
            self.switch_to_namespace(&fav.namespace).await;
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
        let current = self
            .manager
            .as_ref()
            .map(|m| m.context().to_string())
            .unwrap_or_default();
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
        self.switch_to_context(&context).await;
    }

    pub fn add_cluster_tab(&mut self) {
        let Some(ctx) = self.manager.as_ref().map(|m| m.context().to_string()) else {
            return;
        };
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
        let Some(current) = self.manager.as_ref().map(|m| m.context().to_string()) else {
            return;
        };
        let idx = self
            .cluster_tabs
            .iter()
            .position(|t| t == &current)
            .unwrap_or(0);
        self.cluster_tabs.remove(idx);
        let next = self.cluster_tabs[idx.min(self.cluster_tabs.len() - 1)].clone();
        self.persist_ui_settings();
        if next != current {
            self.switch_to_context(&next).await;
        }
    }

    pub async fn run_pending_action(&mut self, action: PendingAction) {
        match action {
            PendingAction::Delete => self.delete_selection().await,
            PendingAction::Scale => self.prompt_scale(),
            PendingAction::Restart => self.restart_selection().await,
            PendingAction::TriggerCronJob => self.trigger_cronjob().await,
            PendingAction::SuspendCronJob => self.set_cronjob_suspended(true).await,
            PendingAction::ResumeCronJob => self.set_cronjob_suspended(false).await,
            PendingAction::StartPortForward => self.prompt_port_forward(),
            PendingAction::ToggleFavorite => self.toggle_favorite_selection(),
            PendingAction::OpenLogs => self.start_logs_for_selection().await,
            PendingAction::OpenServiceLogs => self.start_logs_for_service_selection().await,
            PendingAction::ExecShell => self.request_exec_shell(),
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
                self.overlay = Some(Overlay::Input {
                    prompt: format!("Remote port (local {local_port})"),
                    value: remote_port.to_string(),
                    purpose: InputPurpose::PortForwardRemote,
                    extra: Some(local_port.to_string()),
                });
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
        }
    }

    async fn delete_selection(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        match manager
            .delete_resource(self.active_kind, &name, false)
            .await
        {
            Ok(()) => {
                self.status_message = format!("Deleted {name}");
                self.error_message = None;
                self.refresh().await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn scale_selection(&mut self, replicas: i32) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        let result = match self.active_kind {
            ResourceKind::Deployment => manager.scale_deployment(&name, replicas).await,
            ResourceKind::StatefulSet => manager.scale_statefulset(&name, replicas).await,
            _ => return,
        };
        match result {
            Ok(()) => {
                self.status_message = format!("Scaled {name} to {replicas}");
                self.error_message = None;
                self.refresh().await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn trigger_cronjob(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        match manager.trigger_cronjob(&name).await {
            Ok(()) => {
                self.status_message = format!("Triggered CronJob {name}");
                self.error_message = None;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn set_cronjob_suspended(&mut self, suspend: bool) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        match manager.set_cronjob_suspended(&name, suspend).await {
            Ok(()) => {
                self.status_message = format!(
                    "{} CronJob {name}",
                    if suspend { "Suspended" } else { "Resumed" }
                );
                self.error_message = None;
                self.refresh().await;
            }
            Err(err) => self.error_message = Some(err.user_message()),
        }
    }

    async fn start_port_forward(&mut self, local_port: u16, remote_port: u16) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            return;
        };
        let kind = self.active_kind;
        let id = self.next_port_forward_id;
        self.next_port_forward_id += 1;
        let label = format!("{}/{}:{remote_port}→{local_port}", kind.api_kind(), name);

        if self.use_native_port_forward && kind != ResourceKind::Service {
            match start_port_forward(
                manager.client().clone(),
                manager.namespace(),
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

        match spawn_kubectl_port_forward(manager.namespace(), kind, &name, local_port, remote_port)
        {
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
