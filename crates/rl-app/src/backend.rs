use rl_core::{CrdTarget, ResourceKind, ResourceSnapshot};

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
    FetchYaml { kind: ResourceKind, name: String },
    FetchEvents { kind: ResourceKind, name: String },
    FetchContainers { pod_name: String },
    FetchMetrics,
    DeleteResource { kind: ResourceKind, name: String },
    StartLogs {
        pod_name: String,
        container: Option<String>,
    },
    StopLogs,
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
    YamlLoaded { name: String, yaml: String },
    EventsLoaded { text: String },
    ContainersLoaded(Vec<rl_core::ContainerInfo>),
    MetricsLoaded { text: String },
    LogLine(String),
    LogError(String),
    LogsStopped,
    ResourceDeleted { kind: ResourceKind, name: String },
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

async fn run_backend_loop(
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<BackendCommand>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    use rl_core::ClusterManager;

    let mut manager: Option<ClusterManager> = None;
    let mut active_kind = ResourceKind::Pod;
    let mut log_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut log_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>> = None;
    let mut log_err_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>> = None;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
    let mut list_tick = tokio::time::interval(std::time::Duration::from_secs(3));

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                let Some(cmd) = cmd else { break };
                if !handle_command(
                    cmd,
                    &mut manager,
                    &mut active_kind,
                    &mut log_task,
                    &mut log_rx,
                    &mut log_err_rx,
                    event_tx,
                ).await {
                    break;
                }
            }
            _ = tick.tick() => {
                forward_log_lines(&mut log_rx, event_tx);
                forward_log_errors(&mut log_err_rx, event_tx);
                if let Some(mgr) = manager.as_ref() {
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

    if let Some(task) = log_task.take() {
        task.abort();
    }
}

async fn handle_command(
    cmd: BackendCommand,
    manager: &mut Option<rl_core::ClusterManager>,
    active_kind: &mut ResourceKind,
    log_task: &mut Option<tokio::task::JoinHandle<()>>,
    log_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    log_err_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) -> bool {
    use rl_core::{format_events_text, format_metrics_text, ClusterManager};

    match cmd {
        BackendCommand::Shutdown => return false,
        BackendCommand::ConnectDefault => {
            let _ = event_tx.send(BackendEvent::Connecting);
            match ClusterManager::connect_default().await {
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
                match mgr.switch_context(&context).await {
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
                match mgr.set_namespace(namespace).await {
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
            if let Some(mgr) = manager.as_ref() {
                refresh_on_demand_list(mgr, kind, event_tx).await;
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
                match mgr.refresh_watch().await {
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
        BackendCommand::FetchContainers { pod_name } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.pod_containers(&pod_name).await {
                    Ok(containers) => {
                        let _ = event_tx.send(BackendEvent::ContainersLoaded(containers));
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
        BackendCommand::DeleteResource { kind, name } => {
            if let Some(mgr) = manager.as_ref() {
                match mgr.delete_resource(kind, &name).await {
                    Ok(()) => {
                        let _ = event_tx.send(BackendEvent::ResourceDeleted { kind, name });
                    }
                    Err(err) => {
                        let _ = event_tx.send(BackendEvent::Error(err.user_message()));
                    }
                }
            }
        }
        BackendCommand::StartLogs { pod_name, container } => {
            if let Some(mgr) = manager.as_ref() {
                if let Some(task) = log_task.take() {
                    task.abort();
                }
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let (err_tx, err_rx) = tokio::sync::mpsc::unbounded_channel();
                *log_rx = Some(rx);
                *log_err_rx = Some(err_rx);
                *log_task = Some(mgr.spawn_log_stream(pod_name, container, tx, err_tx));
            }
        }
        BackendCommand::StopLogs => {
            if let Some(task) = log_task.take() {
                task.abort();
            }
            *log_rx = None;
            *log_err_rx = None;
            let _ = event_tx.send(BackendEvent::LogsStopped);
        }
        BackendCommand::PersistSettings { kind, container } => {
            if let Some(mgr) = manager.as_ref() {
                mgr.persist_settings(kind, container.as_deref());
            }
        }
    }

    forward_log_lines(log_rx, event_tx);
    forward_log_errors(log_err_rx, event_tx);
    true
}

fn forward_log_lines(
    log_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    if let Some(rx) = log_rx.as_mut() {
        while let Ok(line) = rx.try_recv() {
            let _ = event_tx.send(BackendEvent::LogLine(line));
        }
    }
}

fn forward_log_errors(
    err_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) {
    if let Some(rx) = err_rx.as_mut() {
        while let Ok(message) = rx.try_recv() {
            let _ = event_tx.send(BackendEvent::LogError(message));
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
