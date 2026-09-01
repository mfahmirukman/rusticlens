use std::collections::HashMap;

use kube::Client;
use tokio::sync::mpsc::Sender;

use crate::config::{config_for_context, current_context_name, list_contexts};
use crate::containers::{list_pod_containers, ContainerInfo};
use crate::crd::{list_crd_instances, list_crds};
use crate::error::{Error, Result};
use crate::events::{list_events_for_resource, EventRow};
use crate::helm::list_helm_releases;
use crate::metrics::{list_pod_metrics, PodMetricSummary};
use crate::ops;
use crate::plugin_loader::{load_manifest_plugins, write_example_manifest_if_missing};
use crate::plugins::{LoggingPlugin, PluginRegistry};
use crate::resources::{CrdTarget, ResourceKind};
use crate::settings::{
    load_settings, pick_namespace_for_context, remember_namespace_for_context, save_settings,
};
use crate::store::{ResourceSnapshot, WatchController};

#[derive(Clone, Hash, PartialEq, Eq)]
struct ScopeKey {
    context: String,
    namespace: String,
}

/// Manages cluster connection, namespace selection, and resource watches.
pub struct ClusterManager {
    context: String,
    client: Client,
    namespace: String,
    watch: WatchController,
    crd_targets: Vec<CrdTarget>,
    selected_crd: Option<CrdTarget>,
    plugins: PluginRegistry,
    scope_cache: HashMap<ScopeKey, HashMap<ResourceKind, ResourceSnapshot>>,
    crd_cache: HashMap<String, Vec<CrdTarget>>,
}

fn build_plugin_registry() -> PluginRegistry {
    let mut plugins = PluginRegistry::new();
    plugins.register(Box::new(LoggingPlugin));
    write_example_manifest_if_missing();
    load_manifest_plugins(&mut plugins);
    plugins
}

impl ClusterManager {
    /// Connect using saved settings or current kubeconfig context.
    pub async fn connect_default(active_kind: ResourceKind) -> Result<Self> {
        let settings = load_settings();
        let contexts = list_contexts()?;
        let context = settings
            .last_context
            .filter(|c| contexts.iter().any(|ctx| ctx == c))
            .or_else(|| current_context_name().ok().flatten())
            .or_else(|| contexts.first().cloned())
            .ok_or(Error::NoActiveContext)?;

        Self::connect(&context, active_kind).await
    }

    /// Connect to a specific kubeconfig context.
    pub async fn connect(context: &str, active_kind: ResourceKind) -> Result<Self> {
        let config = config_for_context(context).await?;
        let client = Client::try_from(config)?;
        let namespaces = Self::list_namespaces_best_effort(&client).await;
        let namespace = pick_namespace_for_context(context, &namespaces);

        let plugins = build_plugin_registry();

        let crd_targets = list_crds(&client).await.unwrap_or_default();

        let mut manager = Self {
            context: context.to_string(),
            client,
            namespace,
            watch: WatchController::new(),
            crd_targets,
            selected_crd: None,
            plugins,
            scope_cache: HashMap::new(),
            crd_cache: HashMap::new(),
        };
        manager
            .crd_cache
            .insert(context.to_string(), manager.crd_targets.clone());
        remember_namespace_for_context(context, &manager.namespace);
        manager.plugins.notify_connected(context);
        manager.restore_scope_cache();
        manager.set_active_kind(active_kind).await?;
        Ok(manager)
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn crd_targets(&self) -> &[CrdTarget] {
        &self.crd_targets
    }

    pub fn selected_crd(&self) -> Option<&CrdTarget> {
        self.selected_crd.as_ref()
    }

    pub fn set_selected_crd(&mut self, target: Option<CrdTarget>) {
        self.selected_crd = target;
    }

    pub async fn switch_context(&mut self, context: &str, active_kind: ResourceKind) -> Result<()> {
        self.save_scope_cache();

        let config = config_for_context(context).await?;
        let client = Client::try_from(config)?;
        let namespaces = Self::list_namespaces_best_effort(&client).await;
        let namespace = pick_namespace_for_context(context, &namespaces);

        self.watch.stop_all().await;
        self.client = client;
        self.context = context.to_string();
        self.namespace = namespace;
        self.selected_crd = None;

        if let Some(cached) = self.crd_cache.get(context).cloned() {
            self.crd_targets = cached;
        } else {
            self.crd_targets = list_crds(&self.client).await.unwrap_or_default();
            self.crd_cache
                .insert(context.to_string(), self.crd_targets.clone());
        }

        self.plugins.notify_connected(context);
        remember_namespace_for_context(context, &self.namespace);
        self.restore_scope_cache();
        self.set_active_kind(active_kind).await
    }

    pub async fn set_namespace(
        &mut self,
        namespace: String,
        active_kind: ResourceKind,
    ) -> Result<()> {
        if namespace == self.namespace {
            return Ok(());
        }
        self.save_scope_cache();
        self.watch.stop_all().await;
        self.namespace = namespace.clone();
        remember_namespace_for_context(&self.context, &self.namespace);
        self.restore_scope_cache();
        self.set_active_kind(active_kind).await
    }

    pub async fn set_active_kind(&mut self, kind: ResourceKind) -> Result<()> {
        self.watch
            .ensure_only_kind(self.client.clone(), self.namespace.clone(), kind)
            .await?;
        // Helm releases are fetched on demand by `list_rows` (non-watch kind). We
        // deliberately do NOT call `list_initial_rows` here — that blocked the input
        // loop on a network call whose result was discarded. The TUI loads Helm rows
        // in a background task via `load_kind_rows_async`.
        Ok(())
    }

    pub async fn list_contexts() -> Result<Vec<String>> {
        list_contexts()
    }

    pub async fn list_namespaces(&self) -> Result<Vec<String>> {
        ops::list_namespaces(&self.client).await
    }

    pub fn snapshot(&self, kind: ResourceKind) -> ResourceSnapshot {
        self.watch.snapshot(kind)
    }

    pub async fn list_rows(&self, kind: ResourceKind) -> Result<Vec<crate::ResourceRow>> {
        match kind {
            ResourceKind::HelmRelease => list_helm_releases(&self.client, &self.namespace).await,
            ResourceKind::Crd => {
                if let Some(target) = &self.selected_crd {
                    list_crd_instances(&self.client, &self.namespace, target).await
                } else {
                    Ok(Vec::new())
                }
            }
            _ => Ok(self.watch.snapshot(kind).rows),
        }
    }

    pub async fn refresh_watch(&mut self, kind: ResourceKind) -> Result<()> {
        if kind.uses_watch() {
            self.watch.stop_kind(kind).await;
            self.watch
                .start_kind(self.client.clone(), self.namespace.clone(), kind)
                .await?;
        }
        Ok(())
    }

    pub async fn resource_yaml(&self, kind: ResourceKind, name: &str) -> Result<String> {
        ops::get_resource_yaml(
            &self.client,
            &self.namespace,
            kind,
            name,
            self.selected_crd.as_ref(),
        )
        .await
    }

    pub async fn delete_resource(&self, kind: ResourceKind, name: &str, force: bool) -> Result<()> {
        ops::delete_resource(
            &self.client,
            &self.namespace,
            kind,
            name,
            self.selected_crd.as_ref(),
            force,
        )
        .await
    }

    pub async fn trigger_cronjob(&self, name: &str, job_name: Option<&str>) -> Result<()> {
        ops::trigger_cronjob(&self.client, &self.namespace, name, job_name).await
    }

    pub async fn set_cronjob_suspended(&self, name: &str, suspend: bool) -> Result<()> {
        ops::set_cronjob_suspended(&self.client, &self.namespace, name, suspend).await
    }

    pub async fn restart_deployment(&self, name: &str) -> Result<()> {
        ops::restart_deployment(&self.client, &self.namespace, name).await
    }

    pub async fn restart_statefulset(&self, name: &str) -> Result<()> {
        ops::restart_statefulset(&self.client, &self.namespace, name).await
    }

    pub async fn scale_deployment(&self, name: &str, replicas: i32) -> Result<()> {
        ops::scale_deployment(&self.client, &self.namespace, name, replicas).await
    }

    pub async fn scale_statefulset(&self, name: &str, replicas: i32) -> Result<()> {
        ops::scale_statefulset(&self.client, &self.namespace, name, replicas).await
    }

    pub async fn apply_yaml(&self, yaml: &str) -> Result<Vec<String>> {
        ops::apply_yaml(&self.client, &self.namespace, yaml).await
    }

    pub async fn fetch_dashboard(&self) -> Result<crate::ClusterDashboard> {
        crate::dashboard::fetch_cluster_dashboard(&self.client).await
    }

    pub async fn fetch_pod_logs_tail(
        &self,
        pod_name: &str,
        container: Option<&str>,
        timestamps: bool,
        tail_lines: i64,
    ) -> Result<Vec<String>> {
        ops::fetch_pod_logs_tail(
            &self.client,
            &self.namespace,
            pod_name,
            container,
            timestamps,
            tail_lines,
        )
        .await
    }

    pub async fn fetch_pod_logs_since(
        &self,
        pod_name: &str,
        container: Option<&str>,
        since_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<String>> {
        ops::fetch_pod_logs_since(
            &self.client,
            &self.namespace,
            pod_name,
            container,
            since_time,
        )
        .await
    }

    pub async fn resource_events(&self, kind: ResourceKind, name: &str) -> Result<Vec<EventRow>> {
        list_events_for_resource(&self.client, &self.namespace, kind.api_kind(), name).await
    }

    pub async fn pod_containers(&self, pod_name: &str) -> Result<Vec<ContainerInfo>> {
        list_pod_containers(&self.client, &self.namespace, pod_name).await
    }

    pub async fn pods_for_service(&self, service_name: &str) -> Result<ops::ServicePods> {
        ops::list_pods_for_service(&self.client, &self.namespace, service_name).await
    }

    pub async fn pod_metrics(&self) -> Result<Vec<PodMetricSummary>> {
        list_pod_metrics(&self.client, &self.namespace).await
    }

    pub fn persist_settings(&self, kind: ResourceKind, container: Option<&str>) {
        let mut settings = crate::settings::load_settings();
        settings.last_context = Some(self.context.clone());
        settings.last_namespace = Some(self.namespace.clone());
        settings
            .context_namespaces
            .insert(self.context.clone(), self.namespace.clone());
        settings.last_kind = Some(kind.label().to_string());
        settings.last_container = container.map(str::to_string);
        let _ = save_settings(&settings);
    }

    pub fn spawn_log_stream(
        &self,
        pod_name: String,
        container: Option<String>,
        timestamps: bool,
        tx: Sender<String>,
        err_tx: Sender<String>,
    ) -> tokio::task::JoinHandle<()> {
        let client = self.client.clone();
        let namespace = self.namespace.clone();
        tokio::spawn(async move {
            if let Err(err) = ops::stream_pod_logs(
                &client,
                &namespace,
                &pod_name,
                container.as_deref(),
                timestamps,
                tx,
            )
            .await
            {
                let message = format!("Log stream error: {}", err.user_message());
                tracing::warn!("{message}");
                let _ = err_tx.send(message).await;
            }
        })
    }

    pub fn spawn_multi_pod_log_stream(
        &self,
        pod_names: Vec<String>,
        timestamps: bool,
        tx: Sender<String>,
        err_tx: Sender<String>,
    ) -> tokio::task::JoinHandle<()> {
        let client = self.client.clone();
        let namespace = self.namespace.clone();
        tokio::spawn(async move {
            if let Err(err) =
                ops::poll_multi_pod_logs(&client, &namespace, pod_names, timestamps, tx).await
            {
                let message = format!("Log poll error: {}", err.user_message());
                tracing::warn!("{message}");
                let _ = err_tx.send(message).await;
            }
        })
    }

    fn scope_key(&self) -> ScopeKey {
        ScopeKey {
            context: self.context.clone(),
            namespace: self.namespace.clone(),
        }
    }

    fn save_scope_cache(&mut self) {
        let key = self.scope_key();
        self.scope_cache.insert(key, self.watch.all_snapshots());
        self.crd_cache
            .insert(self.context.clone(), self.crd_targets.clone());
    }

    fn restore_scope_cache(&mut self) {
        let key = self.scope_key();
        if let Some(snapshots) = self.scope_cache.get(&key).cloned() {
            self.watch.restore_snapshots(snapshots);
        }
    }

    async fn list_namespaces_best_effort(client: &Client) -> Vec<String> {
        match ops::list_namespaces(client).await {
            Ok(namespaces) => namespaces,
            Err(err) => {
                tracing::warn!(
                    "namespace list failed during context setup: {}",
                    err.user_message()
                );
                Vec::new()
            }
        }
    }
}
