use std::collections::HashMap;

use rl_core::ops::LOG_BUFFER_MAX_LINES;
use rl_core::{CrdTarget, ResourceKind, ResourceSnapshot};

use crate::log_debug;
use crate::log_info;

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

async fn run_backend_loop(
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<BackendCommand>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    use rl_core::ClusterManager;

    let mut manager: Option<ClusterManager> = None;
    let mut active_kind = ResourceKind::Pod;
    let mut log_polls: HashMap<u64, ActiveLogPoll> = HashMap::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut list_tick = tokio::time::interval(std::time::Duration::from_secs(3));
    let log_poll_interval = std::time::Duration::from_secs(rl_core::ops::LOG_POLL_INTERVAL_SECS);

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                if !handle_command(
                    cmd,
                    &mut manager,
                    &mut active_kind,
                    &mut log_polls,
                    event_tx,
                ).await {
                    break;
                }
            }
            _ = tick.tick() => {
                if let Some(mgr) = manager.as_ref() {
                    poll_log_tabs(mgr, &mut log_polls, log_poll_interval, event_tx).await;
                    push_all_snapshots(mgr, event_tx);
                }
            }
            _ = list_tick.tick() => {
                if let Some(mgr) = manager.as_ref() {
                    refresh_on_demand_list(mgr, active_kind, event_tx).await;
                }
            }
        }
    }

    for (_, _) in log_polls.drain() {}
}

async fn handle_command(
    cmd: BackendCommand,
    manager: &mut Option<rl_core::ClusterManager>,
    active_kind: &mut ResourceKind,
    log_polls: &mut HashMap<u64, ActiveLogPoll>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) -> bool {
    use rl_core::{format_events_text, format_metrics_text, ClusterManager};

    match cmd {
        BackendCommand::Shutdown => return false,
        BackendCommand::ConnectDefault => {
            let _ = event_tx.send(BackendEvent::Connecting);
            match ClusterManager::connect_default(*active_kind).await {
                Ok(mgr) => {
                    let contexts = ClusterManager::list_contexts().await.unwrap_or_default();
                    let namespaces = mgr.list_namespaces().await.unwrap_or_default();
                    let crd_targets = mgr.crd_targets().to_vec();
                    push_all_snapshots(&mgr, event_tx);
                    refresh_on_demand_list(&mgr, *active_kind, event_tx).await;
                    let _ = event_tx.send(BackendEvent::Connected {
                        context: mgr.context().to_string(),
                        namespace: mgr.namespace().to_string(),
                        contexts,
                        namespaces,
                        crd_targets,
                    });
                    *manager = Some(mgr);
                }
                Err(err) => {
                    let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                }
            }
        }
        BackendCommand::SwitchContext(context) => {
            if let Some(mgr) = manager.as_mut() {
                let _ = event_tx.send(BackendEvent::Connecting);
                match mgr.switch_context(&context, *active_kind).await {
                    Ok(()) => {
                        let namespaces = mgr.list_namespaces().await.unwrap_or_default();
                        let crd_targets = mgr.crd_targets().to_vec();
                        push_all_snapshots(mgr, event_tx);
                        refresh_on_demand_list(mgr, *active_kind, event_tx).await;
                        let _ = event_tx.send(BackendEvent::Connected {
                            context: mgr.context().to_string(),
                            namespace: mgr.namespace().to_string(),
                            contexts: ClusterManager::list_contexts().await.unwrap_or_default(),
                            namespaces,
                            crd_targets,
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::SetNamespace(namespace) => {
            if let Some(mgr) = manager.as_mut() {
                match mgr.set_namespace(namespace, *active_kind).await {
                    Ok(()) => {
                        push_all_snapshots(mgr, event_tx);
                        refresh_on_demand_list(mgr, *active_kind, event_tx).await;
                        let _ = event_tx.send(BackendEvent::Connected {
                            context: mgr.context().to_string(),
                            namespace: mgr.namespace().to_string(),
                            contexts: ClusterManager::list_contexts().await.unwrap_or_default(),
                            namespaces: mgr.list_namespaces().await.unwrap_or_default(),
                            crd_targets: mgr.crd_targets().to_vec(),
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::SetActiveKind(kind) => {
            *active_kind = kind;
            if let Some(mgr) = manager.as_mut() {
                if let Err(err) = mgr.set_active_kind(kind).await {
                    let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                } else {
                    push_all_snapshots(mgr, event_tx);
                    refresh_on_demand_list(mgr, kind, event_tx).await;
                }
            }
        }
        BackendCommand::SetCrdTarget(target) => {
            if let Some(mgr) = manager.as_mut() {
                mgr.set_selected_crd(target);
                refresh_on_demand_list(mgr, ResourceKind::Crd, event_tx).await;
            }
        }
        BackendCommand::RefreshWatch => {
            if let Some(mgr) = manager.as_mut() {
                match mgr.refresh_watch(*active_kind).await {
                    Ok(()) => {
                        push_all_snapshots(mgr, event_tx);
                        refresh_on_demand_list(mgr, *active_kind, event_tx).await;
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::RefreshList => {
            if let Some(mgr) = manager.as_ref() {
                refresh_on_demand_list(mgr, *active_kind, event_tx).await;
            }
        }
        BackendCommand::FetchYaml { kind, name } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.resource_yaml(kind, &name).await {
                    Ok(yaml) => {
                        let _ = event_tx.send(BackendEvent::YamlLoaded { name, yaml });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::FetchEvents { kind, name } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.resource_events(kind, &name).await {
                    Ok(events) => {
                        let _ = event_tx.send(BackendEvent::EventsLoaded {
                            text: format_events_text(&events),
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::FetchContainers { tab_id, pod_name } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.pod_containers(&pod_name).await {
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
            }
        }
        BackendCommand::FetchMetrics => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.pod_metrics().await {
                    Ok(metrics) => {
                        let _ = event_tx.send(BackendEvent::MetricsLoaded {
                            text: format_metrics_text(&metrics),
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::FetchOlderLogs {
            tab_id,
            pod_name,
            container,
            timestamps,
            tail_loaded,
        } => {
            if let Some(mgr) = manager.as_ref() {
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
                match mgr
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
            }
        }
        BackendCommand::DeleteResource { kind, name, force } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.delete_resource(kind, &name, force).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::ResourceDeleted { kind, name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::TriggerCronJob { name } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.trigger_cronjob(&name).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::CronJobTriggered { name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::SetCronjobSuspended { name, suspend } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.set_cronjob_suspended(&name, suspend).await {
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
            }
        }
        BackendCommand::RestartDeployment { name } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.restart_deployment(&name).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::DeploymentRestarted { name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::StartLogs {
            tab_id,
            pod_name,
            container,
        } => {
            if let Some(mgr) = manager.as_ref() {
                log_polls.remove(&tab_id);
                start_log_poll(mgr, tab_id, pod_name, container, log_polls, event_tx).await;
            }
        }
        BackendCommand::CloseLog { tab_id } => {
            log_polls.remove(&tab_id);
        }
        BackendCommand::CloseAllLogs => {
            log_polls.clear();
        }
        BackendCommand::PersistSettings { kind, container } => {
            if let Some(mgr) = manager.as_ref() {
                mgr.persist_settings(kind, container.as_deref());
            }
        }
    }

    true
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
