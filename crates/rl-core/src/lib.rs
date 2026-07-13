pub mod cluster;
pub mod config;
pub mod containers;
pub mod crd;
pub mod error;
pub mod events;
pub mod helm;
pub mod metrics;
pub mod ops;
pub mod plugins;
pub mod portforward;
pub mod resources;
pub mod settings;
pub mod store;

pub use cluster::ClusterManager;
pub use containers::ContainerInfo;
pub use crd::{list_crd_instances, list_crds};
pub use error::{Error, Result};
pub use events::{format_events_text, EventRow};
pub use metrics::{format_metrics_text, PodMetricSummary};
pub use ops::{
    kubectl_attach_command, kubectl_edit_command, kubectl_exec_command,
    kubectl_rollout_restart_command, LOG_BUFFER_MAX_LINES,
};
pub use portforward::{
    kubectl_port_forward_command, spawn_kubectl_attach_terminal, spawn_kubectl_exec_terminal,
    spawn_kubectl_port_forward,
};
pub use resources::CrdTarget;
pub use resources::{ResourceCategory, ResourceKind, ResourceRow};
pub use settings::{load_settings, save_settings, AppSettings};
pub use store::{ResourceSnapshot, WatchController};
