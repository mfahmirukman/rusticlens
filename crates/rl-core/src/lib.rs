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
pub use resources::CrdTarget;
pub use error::{Error, Result};
pub use events::{EventRow, format_events_text};
pub use metrics::{PodMetricSummary, format_metrics_text};
pub use ops::kubectl_exec_command;
pub use portforward::{
    kubectl_port_forward_command, spawn_kubectl_exec_terminal, spawn_kubectl_port_forward,
};
pub use resources::{ResourceCategory, ResourceKind, ResourceRow};
pub use settings::{AppSettings, load_settings, save_settings};
pub use store::{ResourceSnapshot, WatchController};
