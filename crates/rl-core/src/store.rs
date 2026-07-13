use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use futures::StreamExt;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Node, Pod, Secret, Service};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{Api, ListParams};
use kube::runtime::watcher::{watcher, Config as WatchConfig, Event};
use kube::Client;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::helm;
use crate::kinds_ext;
use crate::resources::{format_age, pod_ready_string, ResourceKind, ResourceRow};

/// Snapshot of resources for UI rendering.
#[derive(Debug, Clone, Default)]
pub struct ResourceSnapshot {
    pub rows: Vec<ResourceRow>,
    pub revision: u64,
}

#[derive(Default)]
struct StoreState {
    pods: Vec<ResourceRow>,
    deployments: Vec<ResourceRow>,
    statefulsets: Vec<ResourceRow>,
    jobs: Vec<ResourceRow>,
    cronjobs: Vec<ResourceRow>,
    services: Vec<ResourceRow>,
    ingresses: Vec<ResourceRow>,
    configmaps: Vec<ResourceRow>,
    secrets: Vec<ResourceRow>,
    namespaces: Vec<ResourceRow>,
    nodes: Vec<ResourceRow>,
    networkpolicies: Vec<ResourceRow>,
    pvcs: Vec<ResourceRow>,
    storageclasses: Vec<ResourceRow>,
    roles: Vec<ResourceRow>,
    rolebindings: Vec<ResourceRow>,
    clusterroles: Vec<ResourceRow>,
    clusterrolebindings: Vec<ResourceRow>,
    revision: u64,
}

struct KindWatch {
    cancel_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

/// Namespace-scoped watchers with bounded in-memory stores.
pub struct WatchController {
    state: Arc<RwLock<StoreState>>,
    watches: HashMap<ResourceKind, KindWatch>,
}

impl WatchController {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(StoreState::default())),
            watches: HashMap::new(),
        }
    }

    pub async fn ensure_only_kind(
        &mut self,
        client: Client,
        namespace: String,
        kind: ResourceKind,
    ) -> Result<()> {
        let to_stop: Vec<ResourceKind> = self
            .watches
            .keys()
            .copied()
            .filter(|k| *k != kind)
            .collect();
        for k in to_stop {
            self.stop_kind(k).await;
        }
        if kind.uses_watch() {
            self.start_kind(client, namespace, kind).await?;
        }
        Ok(())
    }

    pub async fn start_kind(
        &mut self,
        client: Client,
        namespace: String,
        kind: ResourceKind,
    ) -> Result<()> {
        if !kind.uses_watch() || self.watches.contains_key(&kind) {
            return Ok(());
        }

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let state = Arc::clone(&self.state);
        let task = spawn_kind_watch(client, namespace, kind, state, cancel_rx);
        self.watches.insert(kind, KindWatch { cancel_tx, task });
        Ok(())
    }

    pub async fn stop_kind(&mut self, kind: ResourceKind) {
        if let Some(w) = self.watches.remove(&kind) {
            let _ = w.cancel_tx.send(true);
            w.task.abort();
        }
    }

    pub async fn stop_all(&mut self) {
        let kinds: Vec<ResourceKind> = self.watches.keys().copied().collect();
        for kind in kinds {
            self.stop_kind(kind).await;
        }
    }

    pub fn all_snapshots(&self) -> HashMap<ResourceKind, ResourceSnapshot> {
        ResourceKind::ALL
            .iter()
            .filter(|k| k.uses_watch())
            .map(|&kind| (kind, self.snapshot(kind)))
            .collect()
    }

    pub fn restore_snapshots(&mut self, snapshots: HashMap<ResourceKind, ResourceSnapshot>) {
        let mut guard = self.state.write().expect("store lock");
        for (kind, snap) in snapshots {
            apply_rows(&mut guard, kind, snap.rows);
        }
        guard.revision += 1;
    }

    pub fn snapshot(&self, kind: ResourceKind) -> ResourceSnapshot {
        let guard = self.state.read().expect("store lock");
        let rows = rows_for_kind(&guard, kind);
        ResourceSnapshot {
            rows,
            revision: guard.revision,
        }
    }
}

impl Default for WatchController {
    fn default() -> Self {
        Self::new()
    }
}

fn rows_for_kind(guard: &StoreState, kind: ResourceKind) -> Vec<ResourceRow> {
    match kind {
        ResourceKind::Pod => guard.pods.clone(),
        ResourceKind::Deployment => guard.deployments.clone(),
        ResourceKind::StatefulSet => guard.statefulsets.clone(),
        ResourceKind::Job => guard.jobs.clone(),
        ResourceKind::CronJob => guard.cronjobs.clone(),
        ResourceKind::Service => guard.services.clone(),
        ResourceKind::Ingress => guard.ingresses.clone(),
        ResourceKind::ConfigMap => guard.configmaps.clone(),
        ResourceKind::Secret => guard.secrets.clone(),
        ResourceKind::Namespace => guard.namespaces.clone(),
        ResourceKind::Node => guard.nodes.clone(),
        ResourceKind::NetworkPolicy => guard.networkpolicies.clone(),
        ResourceKind::PersistentVolumeClaim => guard.pvcs.clone(),
        ResourceKind::StorageClass => guard.storageclasses.clone(),
        ResourceKind::Role => guard.roles.clone(),
        ResourceKind::RoleBinding => guard.rolebindings.clone(),
        ResourceKind::ClusterRole => guard.clusterroles.clone(),
        ResourceKind::ClusterRoleBinding => guard.clusterrolebindings.clone(),
        ResourceKind::HelmRelease | ResourceKind::Crd => Vec::new(),
    }
}

fn apply_rows(guard: &mut StoreState, kind: ResourceKind, rows: Vec<ResourceRow>) {
    match kind {
        ResourceKind::Pod => guard.pods = rows,
        ResourceKind::Deployment => guard.deployments = rows,
        ResourceKind::StatefulSet => guard.statefulsets = rows,
        ResourceKind::Job => guard.jobs = rows,
        ResourceKind::CronJob => guard.cronjobs = rows,
        ResourceKind::Service => guard.services = rows,
        ResourceKind::Ingress => guard.ingresses = rows,
        ResourceKind::ConfigMap => guard.configmaps = rows,
        ResourceKind::Secret => guard.secrets = rows,
        ResourceKind::Namespace => guard.namespaces = rows,
        ResourceKind::Node => guard.nodes = rows,
        ResourceKind::NetworkPolicy => guard.networkpolicies = rows,
        ResourceKind::PersistentVolumeClaim => guard.pvcs = rows,
        ResourceKind::StorageClass => guard.storageclasses = rows,
        ResourceKind::Role => guard.roles = rows,
        ResourceKind::RoleBinding => guard.rolebindings = rows,
        ResourceKind::ClusterRole => guard.clusterroles = rows,
        ResourceKind::ClusterRoleBinding => guard.clusterrolebindings = rows,
        ResourceKind::HelmRelease | ResourceKind::Crd => {}
    }
}

fn spawn_kind_watch(
    client: Client,
    namespace: String,
    kind: ResourceKind,
    state: Arc<RwLock<StoreState>>,
    cancel_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    match kind {
        ResourceKind::Pod => spawn_watch(
            watch_pods(client, namespace),
            state,
            |guard, rows| {
                guard.pods = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Deployment => spawn_watch(
            watch_deployments(client, namespace),
            state,
            |guard, rows| {
                guard.deployments = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::StatefulSet => spawn_watch(
            watch_statefulsets(client, namespace),
            state,
            |guard, rows| {
                guard.statefulsets = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Job => spawn_watch(
            watch_jobs(client, namespace),
            state,
            |guard, rows| {
                guard.jobs = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::CronJob => spawn_watch(
            watch_cronjobs(client, namespace),
            state,
            |guard, rows| {
                guard.cronjobs = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Service => spawn_watch(
            watch_services(client, namespace),
            state,
            |guard, rows| {
                guard.services = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Ingress => spawn_watch(
            watch_ingresses(client, namespace),
            state,
            |guard, rows| {
                guard.ingresses = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::ConfigMap => spawn_watch(
            watch_configmaps(client, namespace),
            state,
            |guard, rows| {
                guard.configmaps = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Secret => spawn_watch(
            watch_secrets(client, namespace),
            state,
            |guard, rows| {
                guard.secrets = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Namespace => spawn_watch(
            watch_namespaces(client),
            state,
            |guard, rows| {
                guard.namespaces = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Node => spawn_watch(
            watch_nodes(client),
            state,
            |guard, rows| {
                guard.nodes = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::NetworkPolicy => spawn_watch(
            kinds_ext::watch_networkpolicies(client, namespace),
            state,
            |guard, rows| {
                guard.networkpolicies = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::PersistentVolumeClaim => spawn_watch(
            kinds_ext::watch_pvcs(client, namespace),
            state,
            |guard, rows| {
                guard.pvcs = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::StorageClass => spawn_watch(
            kinds_ext::watch_storageclasses(client),
            state,
            |guard, rows| {
                guard.storageclasses = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::Role => spawn_watch(
            kinds_ext::watch_roles(client, namespace),
            state,
            |guard, rows| {
                guard.roles = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::RoleBinding => spawn_watch(
            kinds_ext::watch_rolebindings(client, namespace),
            state,
            |guard, rows| {
                guard.rolebindings = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::ClusterRole => spawn_watch(
            kinds_ext::watch_clusterroles(client),
            state,
            |guard, rows| {
                guard.clusterroles = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::ClusterRoleBinding => spawn_watch(
            kinds_ext::watch_clusterrolebindings(client),
            state,
            |guard, rows| {
                guard.clusterrolebindings = rows;
                guard.revision += 1;
            },
            cancel_rx,
        ),
        ResourceKind::HelmRelease | ResourceKind::Crd => tokio::spawn(async {}),
    }
}

fn spawn_watch<S, F>(
    stream: S,
    state: Arc<RwLock<StoreState>>,
    apply: F,
    mut cancel_rx: watch::Receiver<bool>,
) -> JoinHandle<()>
where
    S: futures::Stream<Item = Vec<ResourceRow>> + Send + 'static,
    F: Fn(&mut StoreState, Vec<ResourceRow>) + Send + Sync + 'static,
{
    tokio::spawn(async move {
        let mut stream = Box::pin(stream);
        loop {
            tokio::select! {
                changed = cancel_rx.changed() => {
                    if changed.is_err() || *cancel_rx.borrow() {
                        break;
                    }
                }
                rows = stream.next() => {
                    match rows {
                        Some(rows) => {
                            let mut guard = state.write().expect("store lock");
                            apply(&mut guard, rows);
                        }
                        None => break,
                    }
                }
            }
        }
    })
}

pub(crate) enum WatchDelta {
    Upsert(Box<ResourceRow>),
    Remove(String),
    Noop,
}

pub(crate) fn apply_delta(map: &mut HashMap<String, ResourceRow>, delta: WatchDelta) {
    match delta {
        WatchDelta::Upsert(row) if !row.name.is_empty() => {
            map.insert(row.name.clone(), *row);
        }
        WatchDelta::Remove(name) if !name.is_empty() => {
            map.remove(&name);
        }
        WatchDelta::Noop | WatchDelta::Upsert(_) | WatchDelta::Remove(_) => {}
    }
}

fn watch_pods(client: Client, namespace: String) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Pod> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(pod)) | Ok(Event::InitApply(pod)) => {
                WatchDelta::Upsert(Box::new(pod_to_row(&pod, &ns)))
            }
            Ok(Event::Delete(pod)) => WatchDelta::Remove(pod.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_deployments(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Deployment> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(dep)) | Ok(Event::InitApply(dep)) => {
                WatchDelta::Upsert(Box::new(deployment_to_row(&dep, &ns)))
            }
            Ok(Event::Delete(dep)) => WatchDelta::Remove(dep.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_statefulsets(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<StatefulSet> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(sts)) | Ok(Event::InitApply(sts)) => {
                WatchDelta::Upsert(Box::new(statefulset_to_row(&sts, &ns)))
            }
            Ok(Event::Delete(sts)) => WatchDelta::Remove(sts.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_jobs(client: Client, namespace: String) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Job> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(job)) | Ok(Event::InitApply(job)) => {
                WatchDelta::Upsert(Box::new(job_to_row(&job, &ns)))
            }
            Ok(Event::Delete(job)) => WatchDelta::Remove(job.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_cronjobs(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<CronJob> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(cj)) | Ok(Event::InitApply(cj)) => {
                WatchDelta::Upsert(Box::new(cronjob_to_row(&cj, &ns)))
            }
            Ok(Event::Delete(cj)) => WatchDelta::Remove(cj.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_services(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Service> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(svc)) | Ok(Event::InitApply(svc)) => {
                WatchDelta::Upsert(Box::new(service_to_row(&svc, &ns)))
            }
            Ok(Event::Delete(svc)) => WatchDelta::Remove(svc.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_ingresses(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Ingress> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(ing)) | Ok(Event::InitApply(ing)) => {
                WatchDelta::Upsert(Box::new(ingress_to_row(&ing, &ns)))
            }
            Ok(Event::Delete(ing)) => WatchDelta::Remove(ing.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_configmaps(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<ConfigMap> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(cm)) | Ok(Event::InitApply(cm)) => {
                WatchDelta::Upsert(Box::new(configmap_to_row(&cm, &ns)))
            }
            Ok(Event::Delete(cm)) => WatchDelta::Remove(cm.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_secrets(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Secret> = Api::namespaced(client, &namespace);
    let ns = namespace.clone();
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(secret)) | Ok(Event::InitApply(secret)) => {
                WatchDelta::Upsert(Box::new(secret_to_row(&secret, &ns)))
            }
            Ok(Event::Delete(secret)) => {
                WatchDelta::Remove(secret.metadata.name.unwrap_or_default())
            }
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_namespaces(client: Client) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Namespace> = Api::all(client);
    watcher(api, WatchConfig::default())
        .map(|event| match event {
            Ok(Event::Apply(ns)) | Ok(Event::InitApply(ns)) => {
                WatchDelta::Upsert(Box::new(namespace_to_row(&ns)))
            }
            Ok(Event::Delete(ns)) => WatchDelta::Remove(ns.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn watch_nodes(client: Client) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Node> = Api::all(client);
    watcher(api, WatchConfig::default())
        .map(|event| match event {
            Ok(Event::Apply(node)) | Ok(Event::InitApply(node)) => {
                WatchDelta::Upsert(Box::new(node_to_row(&node)))
            }
            Ok(Event::Delete(node)) => WatchDelta::Remove(node.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

fn pod_to_row(pod: &Pod, namespace: &str) -> ResourceRow {
    let name = pod.metadata.name.clone().unwrap_or_default();
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_else(|| "Unknown".into());
    let container_statuses = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref());

    let (ready_count, total, restarts) = match container_statuses {
        Some(statuses) => {
            let total = statuses.len() as i32;
            let ready = statuses.iter().filter(|c| c.ready).count() as i32;
            let restarts: i32 = statuses.iter().map(|c| c.restart_count).sum();
            (ready, total, restarts)
        }
        None => (0, 0, 0),
    };

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        pod_ready_string(ready_count, total),
        phase,
        restarts.to_string(),
        format_age(pod.metadata.creation_timestamp.as_ref()),
    );
    row.controlled_by = pod_owner_kind(pod);
    row
}

fn pod_owner_kind(pod: &Pod) -> String {
    pod.metadata
        .owner_references
        .as_ref()
        .and_then(|refs| refs.first())
        .map(|owner| owner.kind.clone())
        .unwrap_or_else(|| "-".into())
}

fn deployment_to_row(dep: &Deployment, namespace: &str) -> ResourceRow {
    let name = dep.metadata.name.clone().unwrap_or_default();
    let status = dep.status.as_ref();
    let ready = status.and_then(|s| s.ready_replicas).unwrap_or(0);
    let desired = status
        .and_then(|s| s.replicas)
        .or_else(|| dep.spec.as_ref().and_then(|s| s.replicas))
        .unwrap_or(0);
    let updated = status.and_then(|s| s.updated_replicas).unwrap_or(0);
    let available = status.and_then(|s| s.available_replicas).unwrap_or(0);
    let phase = if ready == desired && desired > 0 {
        "Available"
    } else {
        "Progressing"
    };

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        format!("{ready}/{desired}"),
        phase.to_string(),
        "-".to_string(),
        format_age(dep.metadata.creation_timestamp.as_ref()),
    );
    row.up_to_date = updated.to_string();
    row.active = available.to_string();
    row
}

fn statefulset_to_row(sts: &StatefulSet, namespace: &str) -> ResourceRow {
    let name = sts.metadata.name.clone().unwrap_or_default();
    let status = sts.status.as_ref();
    let ready = status.and_then(|s| s.ready_replicas).unwrap_or(0);
    let desired = status
        .map(|s| s.replicas)
        .or_else(|| sts.spec.as_ref().and_then(|s| s.replicas))
        .unwrap_or(0);

    ResourceRow::new(
        name,
        namespace.to_string(),
        format!("{ready}/{desired}"),
        if ready == desired && desired > 0 {
            "Ready".into()
        } else {
            "Progressing".into()
        },
        "-".to_string(),
        format_age(sts.metadata.creation_timestamp.as_ref()),
    )
}

fn job_to_row(job: &Job, namespace: &str) -> ResourceRow {
    let name = job.metadata.name.clone().unwrap_or_default();
    let status = job.status.as_ref();
    let succeeded = status.and_then(|s| s.succeeded).unwrap_or(0);
    let active = status.and_then(|s| s.active).unwrap_or(0);
    let failed = status.and_then(|s| s.failed).unwrap_or(0);
    let total = active + succeeded + failed;
    let phase = if succeeded > 0 && active == 0 {
        "Complete"
    } else if failed > 0 {
        "Failed"
    } else if active > 0 {
        "Running"
    } else {
        "Pending"
    };

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        format!("{succeeded}/{total}"),
        phase.to_string(),
        "-".to_string(),
        format_age(job.metadata.creation_timestamp.as_ref()),
    );
    row.owner = job
        .metadata
        .owner_references
        .as_ref()
        .and_then(|refs| {
            refs.iter()
                .find(|r| r.kind == "CronJob")
                .map(|r| r.name.clone())
        })
        .unwrap_or_else(|| "-".into());
    row
}

fn cronjob_to_row(cj: &CronJob, namespace: &str) -> ResourceRow {
    let name = cj.metadata.name.clone().unwrap_or_default();
    let status = cj.status.as_ref();
    let suspend = cj.spec.as_ref().and_then(|s| s.suspend).unwrap_or(false);
    let schedule = cj
        .spec
        .as_ref()
        .map(|s| s.schedule.clone())
        .unwrap_or_else(|| "-".into());
    let timezone = cj
        .spec
        .as_ref()
        .and_then(|s| s.time_zone.clone())
        .unwrap_or_else(|| "UTC".into());
    let active = status
        .and_then(|s| s.active.as_ref())
        .map(|jobs| jobs.len().to_string())
        .unwrap_or_else(|| "0".into());
    let last_schedule = status
        .and_then(|s| s.last_schedule_time.as_ref())
        .map(|t| format_age(Some(t)))
        .unwrap_or_else(|| "-".into());
    let phase = if suspend {
        "Suspended".to_string()
    } else {
        "Active".to_string()
    };

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        schedule.clone(),
        phase,
        "-".to_string(),
        format_age(cj.metadata.creation_timestamp.as_ref()),
    );
    row.schedule = schedule;
    row.timezone = timezone;
    row.resumed = if suspend {
        "False".into()
    } else {
        "True".into()
    };
    row.active = active;
    row.last_schedule = last_schedule;
    row
}

fn service_to_row(svc: &Service, namespace: &str) -> ResourceRow {
    let name = svc.metadata.name.clone().unwrap_or_default();
    let spec = svc.spec.as_ref();
    let svc_type = spec
        .and_then(|s| s.type_.clone())
        .unwrap_or_else(|| "ClusterIP".into());

    let cluster_ip = spec
        .and_then(|s| s.cluster_ip.clone())
        .filter(|ip| !ip.is_empty())
        .unwrap_or_else(|| "None".into());

    let external_ip = format_service_external_ips(svc);
    let (ports, service_ports) = format_service_ports(spec);

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        "-".to_string(),
        "Active".to_string(),
        "-".to_string(),
        format_age(svc.metadata.creation_timestamp.as_ref()),
    );
    row.service_type = svc_type;
    row.cluster_ip = cluster_ip;
    row.external_ip = external_ip;
    row.ports = ports;
    row.service_ports = service_ports;
    row
}

fn format_service_ports(spec: Option<&k8s_openapi::api::core::v1::ServiceSpec>) -> (String, Vec<u16>) {
    let Some(ports) = spec.and_then(|s| s.ports.as_ref()) else {
        return ("-".into(), Vec::new());
    };
    if ports.is_empty() {
        return ("-".into(), Vec::new());
    }
    let mut numbers = Vec::new();
    let display = ports
        .iter()
        .map(|p| {
            numbers.push(p.port as u16);
            let proto = p.protocol.as_deref().unwrap_or("TCP");
            format!("{}/{}", p.port, proto)
        })
        .collect::<Vec<_>>()
        .join(", ");
    (display, numbers)
}

fn format_service_external_ips(svc: &Service) -> String {
    let mut ips = Vec::new();
    if let Some(spec) = &svc.spec {
        if let Some(ext) = &spec.external_ips {
            ips.extend(ext.iter().cloned());
        }
    }
    if ips.is_empty() {
        if let Some(lb) = svc
            .status
            .as_ref()
            .and_then(|s| s.load_balancer.as_ref())
        {
            if let Some(ingress) = &lb.ingress {
                for entry in ingress {
                    if let Some(ip) = &entry.ip {
                        ips.push(ip.clone());
                    } else if let Some(host) = &entry.hostname {
                        ips.push(host.clone());
                    }
                }
            }
        }
    }
    if ips.is_empty() {
        "-".into()
    } else {
        ips.join(", ")
    }
}

fn ingress_to_row(ing: &Ingress, namespace: &str) -> ResourceRow {
    let name = ing.metadata.name.clone().unwrap_or_default();
    let hosts = ing
        .spec
        .as_ref()
        .and_then(|s| s.rules.as_ref())
        .map(|rules| rules.len())
        .unwrap_or(0);

    ResourceRow::new(
        name,
        namespace.to_string(),
        "-".to_string(),
        format!("{hosts} rules"),
        "-".to_string(),
        format_age(ing.metadata.creation_timestamp.as_ref()),
    )
}

fn configmap_to_row(cm: &ConfigMap, namespace: &str) -> ResourceRow {
    let name = cm.metadata.name.clone().unwrap_or_default();
    let keys = cm.data.as_ref().map(|d| d.len()).unwrap_or(0);

    ResourceRow::new(
        name,
        namespace.to_string(),
        "-".to_string(),
        format!("{keys} keys"),
        "-".to_string(),
        format_age(cm.metadata.creation_timestamp.as_ref()),
    )
}

fn secret_to_row(secret: &Secret, namespace: &str) -> ResourceRow {
    let name = secret.metadata.name.clone().unwrap_or_default();
    let keys = secret
        .data
        .as_ref()
        .map(|d| d.len())
        .or_else(|| secret.string_data.as_ref().map(|d| d.len()))
        .unwrap_or(0);
    let secret_type = secret.type_.clone().unwrap_or_else(|| "Opaque".into());

    ResourceRow::new(
        name,
        namespace.to_string(),
        "-".to_string(),
        format!("{secret_type} ({keys} keys)"),
        "-".to_string(),
        format_age(secret.metadata.creation_timestamp.as_ref()),
    )
}

fn namespace_to_row(ns: &Namespace) -> ResourceRow {
    let name = ns.metadata.name.clone().unwrap_or_default();
    let phase = ns
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_else(|| "Unknown".into());

    ResourceRow::new(
        name.clone(),
        name,
        "-".to_string(),
        phase,
        "-".to_string(),
        format_age(ns.metadata.creation_timestamp.as_ref()),
    )
}

fn node_to_row(node: &Node) -> ResourceRow {
    let name = node.metadata.name.clone().unwrap_or_default();
    let ready = node
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| {
            conds
                .iter()
                .find(|c| c.type_ == "Ready")
                .map(|c| c.status.clone())
        })
        .unwrap_or_else(|| "Unknown".into());

    ResourceRow::new(
        name.clone(),
        "-".to_string(),
        ready,
        node.status
            .as_ref()
            .and_then(|s| s.node_info.as_ref())
            .map(|i| i.kubelet_version.clone())
            .unwrap_or_else(|| "-".into()),
        "-".to_string(),
        format_age(node.metadata.creation_timestamp.as_ref()),
    )
}

/// Initial list fetch used to seed stores or on-demand kinds.
pub async fn list_initial_rows(
    client: &Client,
    namespace: &str,
    kind: ResourceKind,
) -> Result<Vec<ResourceRow>> {
    match kind {
        ResourceKind::Pod => {
            let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|p| pod_to_row(p, namespace))
                .collect())
        }
        ResourceKind::Deployment => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|d| deployment_to_row(d, namespace))
                .collect())
        }
        ResourceKind::StatefulSet => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|s| statefulset_to_row(s, namespace))
                .collect())
        }
        ResourceKind::Job => {
            let api: Api<Job> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|j| job_to_row(j, namespace))
                .collect())
        }
        ResourceKind::CronJob => {
            let api: Api<CronJob> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|c| cronjob_to_row(c, namespace))
                .collect())
        }
        ResourceKind::Service => {
            let api: Api<Service> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|s| service_to_row(s, namespace))
                .collect())
        }
        ResourceKind::Ingress => {
            let api: Api<Ingress> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|i| ingress_to_row(i, namespace))
                .collect())
        }
        ResourceKind::ConfigMap => {
            let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|c| configmap_to_row(c, namespace))
                .collect())
        }
        ResourceKind::Secret => {
            let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(|s| secret_to_row(s, namespace))
                .collect())
        }
        ResourceKind::Namespace => {
            let api: Api<Namespace> = Api::all(client.clone());
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(namespace_to_row)
                .collect())
        }
        ResourceKind::Node => {
            let api: Api<Node> = Api::all(client.clone());
            Ok(api
                .list(&ListParams::default())
                .await?
                .items
                .iter()
                .map(node_to_row)
                .collect())
        }
        ResourceKind::NetworkPolicy => kinds_ext::list_networkpolicies(client, namespace).await,
        ResourceKind::PersistentVolumeClaim => kinds_ext::list_pvcs(client, namespace).await,
        ResourceKind::StorageClass => kinds_ext::list_storageclasses(client).await,
        ResourceKind::Role => kinds_ext::list_roles(client, namespace).await,
        ResourceKind::RoleBinding => kinds_ext::list_rolebindings(client, namespace).await,
        ResourceKind::ClusterRole => kinds_ext::list_clusterroles(client).await,
        ResourceKind::ClusterRoleBinding => kinds_ext::list_clusterrolebindings(client).await,
        ResourceKind::HelmRelease => helm::list_helm_releases(client, namespace).await,
        ResourceKind::Crd => Ok(Vec::new()),
    }
}
