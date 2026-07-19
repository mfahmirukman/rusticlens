use std::collections::HashMap;
use std::sync::Arc;

use rl_core::ops::LOG_BUFFER_MAX_LINES;
use rl_core::{CrdTarget, ResourceKind, ResourceSnapshot};
use tokio::sync::RwLock;

use crate::log_debug;
use crate::log_info;

type SharedManager = Arc<RwLock<rl_core::ClusterManager>>;

struct ExclusiveSlot {
    handle: tokio::task::JoinHandle<()>,
}

enum ExclusiveOutcome {
    Connected {
        /// `Some` replaces the manager (connect/reconnect); `None` keeps the existing Arc.
        manager: Option<SharedManager>,
        context: String,
        namespace: String,
        contexts: Vec<String>,
        namespaces: Vec<String>,
        crd_targets: Vec<CrdTarget>,
        active_kind: ResourceKind,
    },
    RefreshDone {
        manager: SharedManager,
        active_kind: ResourceKind,
    },
    Failed(String),
}

/// Commands sent from the UI thread to the background Tokio runtime.
#[derive(Debug)]
pub enum BackendCommand {
    ConnectDefault,
    SwitchContext(String),
    SetNamespace(String),
    SetActiveKind(ResourceKind),
    SetCrdTarget(Option<CrdTarget>),
    RefreshWatch,
    RefreshList,
    FetchYaml {
        kind: ResourceKind,
        name: String,
    },
    FetchEvents {
        kind: ResourceKind,
        name: String,
    },
    FetchContainers {
        tab_id: Option<u64>,
        pod_name: String,
    },
    FetchMetrics,
    FetchOlderLogs {
        tab_id: u64,
        pod_name: String,
        container: Option<String>,
        timestamps: bool,
        tail_loaded: usize,
    },
    DeleteResource {
        kind: ResourceKind,
        name: String,
        force: bool,
    },
    TriggerCronJob {
        name: String,
    },
    SetCronjobSuspended {
        name: String,
        suspend: bool,
    },
    RestartDeployment {
        name: String,
    },
    RestartStatefulSet {
        name: String,
    },
    ScaleDeployment {
        name: String,
        replicas: i32,
    },
    ScaleStatefulSet {
        name: String,
        replicas: i32,
    },
    ApplyYaml {
        yaml: String,
    },
    Reconnect,
    FetchDashboard,
    StartLogs {
        tab_id: u64,
        pod_name: String,
        container: Option<String>,
    },
    CloseLog {
        tab_id: u64,
    },
    CloseAllLogs,
    PersistSettings {
        kind: ResourceKind,
        container: Option<String>,
    },
    StartPortForward {
        kind: ResourceKind,
        name: String,
        local_port: u16,
        remote_port: u16,
    },
    StopPortForward {
        id: u64,
    },
    #[cfg(feature = "embedded-terminal")]
    StartEmbeddedExec {
        pod_name: String,
        container: Option<String>,
    },
    #[cfg(feature = "embedded-terminal")]
    StopEmbeddedExec,
    #[cfg(feature = "embedded-terminal")]
    EmbeddedExecInput {
        bytes: Vec<u8>,
    },
    Shutdown,
}

/// Events sent from the background runtime to the UI thread.
#[derive(Debug, Clone)]
pub enum BackendEvent {
    Connecting,
    Connected {
        context: String,
        namespace: String,
        contexts: Vec<String>,
        namespaces: Vec<String>,
        crd_targets: Vec<CrdTarget>,
    },
    Snapshot {
        kind: ResourceKind,
        snapshot: ResourceSnapshot,
    },
    YamlLoaded {
        name: String,
        yaml: String,
    },
    EventsLoaded {
        text: String,
    },
    ContainersLoaded {
        tab_id: Option<u64>,
        pod_name: String,
        containers: Vec<rl_core::ContainerInfo>,
    },
    MetricsLoaded {
        text: String,
    },
    LogLine {
        tab_id: u64,
        line: String,
    },
    LogError {
        tab_id: u64,
        message: String,
    },
    OlderLogsLoaded {
        tab_id: u64,
        prepended: Vec<String>,
        has_more: bool,
    },
    ResourceDeleted {
        kind: ResourceKind,
        name: String,
    },
    CronJobTriggered {
        name: String,
    },
    CronJobSuspendChanged {
        name: String,
        suspended: bool,
    },
    DeploymentRestarted {
        name: String,
    },
    StatefulSetRestarted {
        name: String,
    },
    WorkloadScaled {
        kind: ResourceKind,
        name: String,
        replicas: i32,
    },
    YamlApplied {
        resources: Vec<String>,
    },
    DashboardLoaded {
        dashboard: rl_core::ClusterDashboard,
    },
    PortForwardStarted {
        info: rl_core::PortForwardInfo,
    },
    PortForwardStopped {
        id: u64,
    },
    #[cfg(feature = "embedded-terminal")]
    EmbeddedExecOutput {
        line: String,
    },
    #[cfg(feature = "embedded-terminal")]
    EmbeddedExecStopped,
    /// Exclusive op already running (connect / switch / refresh).
    Busy,
    /// Exclusive op finished without a full reconnect (e.g. refresh watch).
    ExclusiveDone,
    Error(String),
}

/// Handle used by the UI to talk to the backend without blocking.
pub struct BackendHandle {
    pub cmd_tx: tokio::sync::mpsc::UnboundedSender<BackendCommand>,
    pub event_rx: std::sync::mpsc::Receiver<BackendEvent>,
}

impl BackendHandle {
    pub fn send(&self, cmd: BackendCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn drain_events(&self) -> Vec<BackendEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }
}

pub fn spawn_backend() -> BackendHandle {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<BackendCommand>();
    let (event_tx, event_rx) = std::sync::mpsc::channel::<BackendEvent>();

    std::thread::Builder::new()
        .name("rusticlens-backend".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("tokio runtime");

            runtime.block_on(async move {
                run_backend_loop(&mut cmd_rx, &event_tx).await;
            });
        })
        .expect("spawn backend thread");

    BackendHandle { cmd_tx, event_rx }
}

struct ActiveLogPoll {
    pod_name: String,
    container: Option<String>,
    /// RFC3339 `sinceTime` cursor for the next incremental fetch.
    last_since: Option<String>,
    last_poll: std::time::Instant,
}

enum PortForwardSession {
    Native(rl_core::PortForwardHandle),
    Kubectl(std::process::Child),
}

impl PortForwardSession {
    fn stop(self) {
        match self {
            PortForwardSession::Native(handle) => handle.stop(),
            PortForwardSession::Kubectl(mut child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(feature = "embedded-terminal")]
struct EmbeddedExecSession {
    cancel_tx: tokio::sync::oneshot::Sender<()>,
    input_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

async fn run_backend_loop(
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<BackendCommand>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    let (outcome_tx, mut outcome_rx) = tokio::sync::mpsc::unbounded_channel::<ExclusiveOutcome>();

    let mut manager: Option<SharedManager> = None;
    let mut exclusive: Option<ExclusiveSlot> = None;
    let mut active_kind = ResourceKind::Pod;
    let mut log_polls: HashMap<u64, ActiveLogPoll> = HashMap::new();
    let mut port_forwards: HashMap<u64, PortForwardSession> = HashMap::new();
    let mut next_port_forward_id: u64 = 1;
    #[cfg(feature = "embedded-terminal")]
    let mut embedded_exec: Option<EmbeddedExecSession> = None;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut list_tick = tokio::time::interval(std::time::Duration::from_secs(3));
    let log_poll_interval = std::time::Duration::from_secs(rl_core::ops::LOG_POLL_INTERVAL_SECS);

    loop {
        if exclusive.as_ref().is_some_and(|e| e.handle.is_finished()) {
            exclusive = None;
        }

        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                if !handle_command(
                    cmd,
                    &mut manager,
                    &mut exclusive,
                    &outcome_tx,
                    &mut active_kind,
                    &mut log_polls,
                    &mut port_forwards,
                    &mut next_port_forward_id,
                    #[cfg(feature = "embedded-terminal")]
                    &mut embedded_exec,
                    event_tx,
                ).await {
                    break;
                }
            }
            outcome = outcome_rx.recv() => {
                let Some(outcome) = outcome else { break };
                exclusive = None;
                apply_exclusive_outcome(outcome, &mut manager, event_tx).await;
            }
            _ = tick.tick() => {
                if let Some(mgr) = manager.as_ref() {
                    if let Ok(guard) = mgr.try_read() {
                        poll_log_tabs(&guard, &mut log_polls, log_poll_interval, event_tx).await;
                        push_all_snapshots(&guard, event_tx);
                    }
                }
            }
            _ = list_tick.tick() => {
                if let Some(mgr) = manager.as_ref() {
                    if let Ok(guard) = mgr.try_read() {
                        refresh_on_demand_list(&guard, active_kind, event_tx).await;
                    }
                }
            }
        }
    }

    for (_, session) in port_forwards.drain() {
        session.stop();
    }
    #[cfg(feature = "embedded-terminal")]
    if let Some(session) = embedded_exec.take() {
        let _ = session.cancel_tx.send(());
    }
}

async fn apply_exclusive_outcome(
    outcome: ExclusiveOutcome,
    manager: &mut Option<SharedManager>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    match outcome {
        ExclusiveOutcome::Connected {
            manager: new_mgr,
            context,
            namespace,
            contexts,
            namespaces,
            crd_targets,
            active_kind,
        } => {
            if let Some(shared) = new_mgr {
                *manager = Some(shared);
            }
            if let Some(shared) = manager.as_ref() {
                let guard = shared.read().await;
                push_all_snapshots(&guard, event_tx);
                refresh_on_demand_list(&guard, active_kind, event_tx).await;
            }
            let _ = event_tx.send(BackendEvent::Connected {
                context,
                namespace,
                contexts,
                namespaces,
                crd_targets,
            });
        }
        ExclusiveOutcome::RefreshDone {
            manager: shared,
            active_kind,
        } => {
            let guard = shared.read().await;
            push_all_snapshots(&guard, event_tx);
            refresh_on_demand_list(&guard, active_kind, event_tx).await;
            let _ = event_tx.send(BackendEvent::ExclusiveDone);
        }
        ExclusiveOutcome::Failed(msg) => {
            let _ = event_tx.send(BackendEvent::Error(msg));
        }
    }
}

fn try_begin_exclusive(
    exclusive: &mut Option<ExclusiveSlot>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) -> bool {
    if exclusive.as_ref().is_some_and(|e| !e.handle.is_finished()) {
        let _ = event_tx.send(BackendEvent::Busy);
        return false;
    }
    *exclusive = None;
    true
}

#[allow(clippy::too_many_arguments)]
async fn handle_command(
    cmd: BackendCommand,
    manager: &mut Option<SharedManager>,
    exclusive: &mut Option<ExclusiveSlot>,
    outcome_tx: &tokio::sync::mpsc::UnboundedSender<ExclusiveOutcome>,
    active_kind: &mut ResourceKind,
    log_polls: &mut HashMap<u64, ActiveLogPoll>,
    port_forwards: &mut HashMap<u64, PortForwardSession>,
    next_port_forward_id: &mut u64,
    #[cfg(feature = "embedded-terminal")] embedded_exec: &mut Option<EmbeddedExecSession>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) -> bool {
    use rl_core::{format_events_text, format_metrics_text, load_settings, ClusterManager};

    match cmd {
        BackendCommand::Shutdown => {
            for (_, handle) in port_forwards.drain() {
                handle.stop();
            }
            #[cfg(feature = "embedded-terminal")]
            if let Some(session) = embedded_exec.take() {
                let _ = session.cancel_tx.send(());
            }
            return false;
        }
        BackendCommand::ConnectDefault => {
            if !try_begin_exclusive(exclusive, event_tx) {
                return true;
            }
            let _ = event_tx.send(BackendEvent::Connecting);
            let kind = *active_kind;
            let outcome_tx = outcome_tx.clone();
            let handle = tokio::spawn(async move {
                match ClusterManager::connect_default(kind).await {
                    Ok(mgr) => {
                        let contexts = ClusterManager::list_contexts().await.unwrap_or_default();
                        let namespaces = mgr.list_namespaces().await.unwrap_or_default();
                        let crd_targets = mgr.crd_targets().to_vec();
                        let context = mgr.context().to_string();
                        let namespace = mgr.namespace().to_string();
                        let shared = Arc::new(RwLock::new(mgr));
                        let _ = outcome_tx.send(ExclusiveOutcome::Connected {
                            manager: Some(shared),
                            context,
                            namespace,
                            contexts,
                            namespaces,
                            crd_targets,
                            active_kind: kind,
                        });
                    }
                    Err(err) => {
                        let _ = outcome_tx.send(ExclusiveOutcome::Failed(err.user_message()));
                    }
                }
            });
            *exclusive = Some(ExclusiveSlot { handle });
        }
        BackendCommand::SwitchContext(context) => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            if !try_begin_exclusive(exclusive, event_tx) {
                return true;
            }
            let _ = event_tx.send(BackendEvent::Connecting);
            let kind = *active_kind;
            let outcome_tx = outcome_tx.clone();
            let handle = tokio::spawn(async move {
                let mut guard = shared.write().await;
                match guard.switch_context(&context, kind).await {
                    Ok(()) => {
                        let namespaces = guard.list_namespaces().await.unwrap_or_default();
                        let crd_targets = guard.crd_targets().to_vec();
                        let contexts = ClusterManager::list_contexts().await.unwrap_or_default();
                        let _ = outcome_tx.send(ExclusiveOutcome::Connected {
                            manager: None,
                            context: guard.context().to_string(),
                            namespace: guard.namespace().to_string(),
                            contexts,
                            namespaces,
                            crd_targets,
                            active_kind: kind,
                        });
                    }
                    Err(err) => {
                        let _ = outcome_tx.send(ExclusiveOutcome::Failed(err.user_message()));
                    }
                }
            });
            *exclusive = Some(ExclusiveSlot { handle });
        }
        BackendCommand::SetNamespace(namespace) => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            if !try_begin_exclusive(exclusive, event_tx) {
                return true;
            }
            let _ = event_tx.send(BackendEvent::Connecting);
            let kind = *active_kind;
            let outcome_tx = outcome_tx.clone();
            let handle = tokio::spawn(async move {
                let mut guard = shared.write().await;
                match guard.set_namespace(namespace, kind).await {
                    Ok(()) => {
                        let namespaces = guard.list_namespaces().await.unwrap_or_default();
                        let crd_targets = guard.crd_targets().to_vec();
                        let contexts = ClusterManager::list_contexts().await.unwrap_or_default();
                        let _ = outcome_tx.send(ExclusiveOutcome::Connected {
                            manager: None,
                            context: guard.context().to_string(),
                            namespace: guard.namespace().to_string(),
                            contexts,
                            namespaces,
                            crd_targets,
                            active_kind: kind,
                        });
                    }
                    Err(err) => {
                        let _ = outcome_tx.send(ExclusiveOutcome::Failed(err.user_message()));
                    }
                }
            });
            *exclusive = Some(ExclusiveSlot { handle });
        }
        BackendCommand::SetActiveKind(kind) => {
            *active_kind = kind;
            if let Some(shared) = manager.as_ref() {
                let mut guard = shared.write().await;
                if let Err(err) = guard.set_active_kind(kind).await {
                    let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                } else {
                    push_all_snapshots(&guard, event_tx);
                    refresh_on_demand_list(&guard, kind, event_tx).await;
                }
            }
        }
        BackendCommand::SetCrdTarget(target) => {
            if let Some(shared) = manager.as_ref() {
                let mut guard = shared.write().await;
                guard.set_selected_crd(target);
                refresh_on_demand_list(&guard, ResourceKind::Crd, event_tx).await;
            }
        }
        BackendCommand::RefreshWatch => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            if !try_begin_exclusive(exclusive, event_tx) {
                return true;
            }
            let kind = *active_kind;
            let outcome_tx = outcome_tx.clone();
            let handle = tokio::spawn(async move {
                let mut guard = shared.write().await;
                match guard.refresh_watch(kind).await {
                    Ok(()) => {
                        drop(guard);
                        let _ = outcome_tx.send(ExclusiveOutcome::RefreshDone {
                            manager: shared,
                            active_kind: kind,
                        });
                    }
                    Err(err) => {
                        let _ = outcome_tx.send(ExclusiveOutcome::Failed(err.user_message()));
                    }
                }
            });
            *exclusive = Some(ExclusiveSlot { handle });
        }
        BackendCommand::RefreshList => {
            if let Some(shared) = manager.as_ref() {
                let guard = shared.read().await;
                refresh_on_demand_list(&guard, *active_kind, event_tx).await;
            }
        }
        BackendCommand::FetchYaml { kind, name } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.resource_yaml(kind, &name).await {
                    Ok(yaml) => {
                        let _ = event_tx.send(BackendEvent::YamlLoaded { name, yaml });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::FetchEvents { kind, name } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.resource_events(kind, &name).await {
                    Ok(events) => {
                        let _ = event_tx.send(BackendEvent::EventsLoaded {
                            text: format_events_text(&events),
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::FetchContainers { tab_id, pod_name } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.pod_containers(&pod_name).await {
                    Ok(containers) => {
                        let _ = event_tx.send(BackendEvent::ContainersLoaded {
                            tab_id,
                            pod_name,
                            containers,
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::FetchMetrics => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.pod_metrics().await {
                    Ok(metrics) => {
                        let _ = event_tx.send(BackendEvent::MetricsLoaded {
                            text: format_metrics_text(&metrics),
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::FetchOlderLogs {
            tab_id,
            pod_name,
            container,
            timestamps,
            tail_loaded,
        } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                use rl_core::ops::LOG_CHUNK_LINES;
                let buffered = tail_loaded.min(LOG_BUFFER_MAX_LINES);
                let request_tail = buffered as i64 + LOG_CHUNK_LINES;
                log_debug!(
                    tab_id,
                    pod = %pod_name,
                    buffered_lines = buffered,
                    request_tail,
                    "fetching older log chunk from API"
                );
                let guard = shared.read().await;
                match guard
                    .fetch_pod_logs_tail(&pod_name, container.as_deref(), timestamps, request_tail)
                    .await
                {
                    Ok(fetched) => {
                        let prepended = if fetched.len() > buffered {
                            fetched[..fetched.len() - buffered].to_vec()
                        } else {
                            Vec::new()
                        };
                        let fetched_bytes: usize = fetched.iter().map(|l| l.len()).sum();
                        let prepended_bytes: usize = prepended.iter().map(|l| l.len()).sum();
                        log_info!(
                            tab_id,
                            pod = %pod_name,
                            api_lines = fetched.len(),
                            api_kb = fetched_bytes / 1024,
                            prepended_lines = prepended.len(),
                            prepended_kb = prepended_bytes / 1024,
                            "older log API response"
                        );
                        let has_more = !fetched.is_empty() && fetched.len() as i64 >= request_tail;
                        let _ = event_tx.send(BackendEvent::OlderLogsLoaded {
                            tab_id,
                            prepended,
                            has_more,
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::LogError {
                            tab_id,
                            message: format!("Failed to load older logs: {}", err.user_message()),
                        });
                    }
                }
            });
        }
        BackendCommand::DeleteResource { kind, name, force } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.delete_resource(kind, &name, force).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::ResourceDeleted { kind, name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::TriggerCronJob { name } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.trigger_cronjob(&name).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::CronJobTriggered { name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::SetCronjobSuspended { name, suspend } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.set_cronjob_suspended(&name, suspend).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::CronJobSuspendChanged {
                            name,
                            suspended: suspend,
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::RestartDeployment { name } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.restart_deployment(&name).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::DeploymentRestarted { name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::RestartStatefulSet { name } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.restart_statefulset(&name).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::StatefulSetRestarted { name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::ScaleDeployment { name, replicas } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.scale_deployment(&name, replicas).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::WorkloadScaled {
                            kind: ResourceKind::Deployment,
                            name,
                            replicas,
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::ScaleStatefulSet { name, replicas } => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.scale_statefulset(&name, replicas).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::WorkloadScaled {
                            kind: ResourceKind::StatefulSet,
                            name,
                            replicas,
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::ApplyYaml { yaml } => {
            if let Some(shared) = manager.as_ref() {
                // Inline: serde_yaml deserializer is !Send, so this cannot be spawned.
                let guard = shared.read().await;
                match guard.apply_yaml(&yaml).await {
                    Ok(resources) => {
                        let _ = event_tx.send(BackendEvent::YamlApplied { resources });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::Reconnect => {
            if !try_begin_exclusive(exclusive, event_tx) {
                return true;
            }
            let kind = *active_kind;
            let context = if let Some(shared) = manager.as_ref() {
                let guard = shared.try_read();
                match guard {
                    Ok(g) => Some(g.context().to_string()),
                    Err(_) => rl_core::config::current_context_name().ok().flatten(),
                }
            } else {
                rl_core::config::current_context_name().ok().flatten()
            };
            let _ = event_tx.send(BackendEvent::Connecting);
            let outcome_tx = outcome_tx.clone();
            let handle = tokio::spawn(async move {
                let result = match context {
                    Some(ctx) => ClusterManager::connect(&ctx, kind).await,
                    None => ClusterManager::connect_default(kind).await,
                };
                match result {
                    Ok(mgr) => {
                        let contexts = ClusterManager::list_contexts().await.unwrap_or_default();
                        let namespaces = mgr.list_namespaces().await.unwrap_or_default();
                        let crd_targets = mgr.crd_targets().to_vec();
                        let context = mgr.context().to_string();
                        let namespace = mgr.namespace().to_string();
                        let shared = Arc::new(RwLock::new(mgr));
                        let _ = outcome_tx.send(ExclusiveOutcome::Connected {
                            manager: Some(shared),
                            context,
                            namespace,
                            contexts,
                            namespaces,
                            crd_targets,
                            active_kind: kind,
                        });
                    }
                    Err(err) => {
                        let _ = outcome_tx.send(ExclusiveOutcome::Failed(err.user_message()));
                    }
                }
            });
            *exclusive = Some(ExclusiveSlot { handle });
        }
        BackendCommand::FetchDashboard => {
            let Some(shared) = manager.clone() else {
                return true;
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let guard = shared.read().await;
                match guard.fetch_dashboard().await {
                    Ok(dashboard) => {
                        let _ = event_tx.send(BackendEvent::DashboardLoaded { dashboard });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            });
        }
        BackendCommand::StartLogs {
            tab_id,
            pod_name,
            container,
        } => {
            if let Some(shared) = manager.as_ref() {
                log_polls.remove(&tab_id);
                let guard = shared.read().await;
                start_log_poll(&guard, tab_id, pod_name, container, log_polls, event_tx).await;
            }
        }
        BackendCommand::CloseLog { tab_id } => {
            log_polls.remove(&tab_id);
        }
        BackendCommand::CloseAllLogs => {
            log_polls.clear();
        }
        BackendCommand::PersistSettings { kind, container } => {
            if let Some(shared) = manager.as_ref() {
                let guard = shared.read().await;
                guard.persist_settings(kind, container.as_deref());
            }
        }
        BackendCommand::StartPortForward {
            kind,
            name,
            local_port,
            remote_port,
        } => {
            let settings = load_settings();
            if let Some(shared) = manager.as_ref() {
                let id = *next_port_forward_id;
                *next_port_forward_id += 1;
                let guard = shared.read().await;
                if settings.use_native_port_forward && kind != ResourceKind::Service {
                    match rl_core::start_port_forward(
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
                            port_forwards.insert(id, PortForwardSession::Native(handle));
                            let _ = event_tx.send(BackendEvent::PortForwardStarted { info });
                        }
                        Err(err) => {
                            tracing::info!(
                                "native port-forward unavailable, using kubectl: {}",
                                err.user_message()
                            );
                            try_kubectl_port_forward(
                                &guard,
                                kind,
                                &name,
                                local_port,
                                remote_port,
                                id,
                                port_forwards,
                                event_tx,
                            );
                        }
                    }
                } else {
                    try_kubectl_port_forward(
                        &guard,
                        kind,
                        &name,
                        local_port,
                        remote_port,
                        id,
                        port_forwards,
                        event_tx,
                    );
                }
            }
        }
        BackendCommand::StopPortForward { id } => {
            if let Some(session) = port_forwards.remove(&id) {
                session.stop();
            }
            let _ = event_tx.send(BackendEvent::PortForwardStopped { id });
        }
        #[cfg(feature = "embedded-terminal")]
        BackendCommand::StartEmbeddedExec {
            pod_name,
            container,
        } => {
            if let Some(shared) = manager.as_ref() {
                if let Some(session) = embedded_exec.take() {
                    let _ = session.cancel_tx.send(());
                }
                let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
                let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
                let guard = shared.read().await;
                let client = guard.client().clone();
                let namespace = guard.namespace().to_string();
                drop(guard);
                let event_tx_out = event_tx.clone();
                let event_tx_exec = event_tx.clone();
                tokio::spawn(async move {
                    while let Some(line) = output_rx.recv().await {
                        let _ = event_tx_out.send(BackendEvent::EmbeddedExecOutput { line });
                    }
                });
                tokio::spawn(async move {
                    let result = rl_core::run_pod_exec(
                        client,
                        &namespace,
                        &pod_name,
                        container.as_deref(),
                        output_tx,
                        input_rx,
                        cancel_rx,
                    )
                    .await;
                    if let Err(err) = result {
                        let _ = event_tx_exec.send(BackendEvent::EmbeddedExecOutput {
                            line: format!("[error] {}", err.user_message()),
                        });
                    }
                    let _ = event_tx_exec.send(BackendEvent::EmbeddedExecStopped);
                });
                *embedded_exec = Some(EmbeddedExecSession {
                    cancel_tx,
                    input_tx,
                });
            }
        }
        #[cfg(feature = "embedded-terminal")]
        BackendCommand::StopEmbeddedExec => {
            if let Some(session) = embedded_exec.take() {
                let _ = session.cancel_tx.send(());
            }
            let _ = event_tx.send(BackendEvent::EmbeddedExecStopped);
        }
        #[cfg(feature = "embedded-terminal")]
        BackendCommand::EmbeddedExecInput { bytes } => {
            if let Some(session) = embedded_exec.as_ref() {
                let _ = session.input_tx.send(bytes);
            }
        }
    }

    true
}

#[allow(clippy::too_many_arguments)]
fn try_kubectl_port_forward(
    mgr: &rl_core::ClusterManager,
    kind: ResourceKind,
    name: &str,
    local_port: u16,
    remote_port: u16,
    id: u64,
    port_forwards: &mut HashMap<u64, PortForwardSession>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    match rl_core::spawn_kubectl_port_forward(mgr.namespace(), kind, name, local_port, remote_port)
    {
        Ok(child) => {
            let info = rl_core::PortForwardInfo {
                id,
                label: format!("{}/{}:{remote_port}", kind.api_kind(), name),
                local_port,
                remote_port,
            };
            port_forwards.insert(id, PortForwardSession::Kubectl(child));
            let _ = event_tx.send(BackendEvent::PortForwardStarted { info });
        }
        Err(err) => {
            let _ = event_tx.send(BackendEvent::Error(err.user_message()));
        }
    }
}

async fn start_log_poll(
    mgr: &rl_core::ClusterManager,
    tab_id: u64,
    pod_name: String,
    container: Option<String>,
    log_polls: &mut HashMap<u64, ActiveLogPoll>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    use rl_core::ops::{since_time_after_lines, LOG_INITIAL_TAIL_LINES};

    // Always request API timestamps so incremental `sinceTime` polls work (Freelens model).
    match mgr
        .fetch_pod_logs_tail(
            &pod_name,
            container.as_deref(),
            true,
            LOG_INITIAL_TAIL_LINES,
        )
        .await
    {
        Ok(lines) => {
            let last_since = since_time_after_lines(&lines)
                .map(|t| t.to_rfc3339())
                .or_else(|| Some(rl_core::ops::fallback_log_since_time().to_rfc3339()));
            for line in lines {
                let _ = event_tx.send(BackendEvent::LogLine { tab_id, line });
            }
            log_info!(
                tab_id,
                pod = %pod_name,
                poll_secs = rl_core::ops::LOG_POLL_INTERVAL_SECS,
                "started log polling (initial tail + sinceTime)"
            );
            log_polls.insert(
                tab_id,
                ActiveLogPoll {
                    pod_name,
                    container,
                    last_since,
                    last_poll: std::time::Instant::now(),
                },
            );
        }
        Err(err) => {
            let _ = event_tx.send(BackendEvent::LogError {
                tab_id,
                message: format!("Failed to load logs: {}", err.user_message()),
            });
        }
    }
}

async fn poll_log_tabs(
    mgr: &rl_core::ClusterManager,
    log_polls: &mut HashMap<u64, ActiveLogPoll>,
    interval: std::time::Duration,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    use rl_core::ops::since_time_after_lines;

    let now = std::time::Instant::now();
    let tab_ids: Vec<u64> = log_polls.keys().copied().collect();
    for tab_id in tab_ids {
        let Some(poll) = log_polls.get_mut(&tab_id) else {
            continue;
        };
        if now.duration_since(poll.last_poll) < interval {
            continue;
        }
        poll.last_poll = now;
        let Some(since_raw) = poll.last_since.as_deref() else {
            continue;
        };
        let since = match rl_core::ops::parse_log_since_time(since_raw) {
            Some(t) => t,
            None => {
                let _ = event_tx.send(BackendEvent::LogError {
                    tab_id,
                    message: format!("Invalid log sinceTime cursor: {since_raw}"),
                });
                poll.last_since = Some(rl_core::ops::fallback_log_since_time().to_rfc3339());
                continue;
            }
        };
        let pod_name = poll.pod_name.clone();
        let container = poll.container.clone();
        match mgr
            .fetch_pod_logs_since(&pod_name, container.as_deref(), since)
            .await
        {
            Ok(new_lines) if new_lines.is_empty() => {}
            Ok(new_lines) => {
                if let Some(next) = since_time_after_lines(&new_lines) {
                    poll.last_since = Some(next.to_rfc3339());
                }
                for line in new_lines {
                    let _ = event_tx.send(BackendEvent::LogLine { tab_id, line });
                }
            }
            Err(err) => {
                let _ = event_tx.send(BackendEvent::LogError {
                    tab_id,
                    message: format!("Failed to poll logs: {}", err.user_message()),
                });
            }
        }
    }
}

fn push_all_snapshots(
    mgr: &rl_core::ClusterManager,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    for kind in ResourceKind::ALL {
        if matches!(kind, ResourceKind::HelmRelease | ResourceKind::Crd) {
            continue;
        }
        let snapshot = mgr.snapshot(kind);
        let _ = event_tx.send(BackendEvent::Snapshot { kind, snapshot });
    }
}

async fn refresh_on_demand_list(
    mgr: &rl_core::ClusterManager,
    kind: ResourceKind,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    if !matches!(kind, ResourceKind::HelmRelease | ResourceKind::Crd) {
        return;
    }
    if let Ok(rows) = mgr.list_rows(kind).await {
        let _ = event_tx.send(BackendEvent::Snapshot {
            kind,
            snapshot: ResourceSnapshot { rows, revision: 0 },
        });
    }
}
