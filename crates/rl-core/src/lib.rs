pub mod cluster;
pub mod config;
pub mod containers;
pub mod crd;
pub mod dashboard;
pub mod error;
pub mod events;
pub mod exec_session;
pub mod helm;
pub mod kinds_ext;
pub mod metrics;
pub mod native_portforward;
pub mod ops;
pub mod plugin_loader;
pub mod plugins;
pub mod portforward;
pub mod resources;
pub mod settings;
pub mod store;

pub use cluster::ClusterManager;
pub use containers::ContainerInfo;
pub use crd::{list_crd_instances, list_crds};
pub use dashboard::{ClusterDashboard, NodeDashboardRow};
pub use error::{Error, Result};
pub use events::{format_events_text, EventRow};
pub use exec_session::run_pod_exec;
pub use metrics::{format_metrics_text, PodMetricSummary};
pub use native_portforward::{
    probe_native_port_forward, start_port_forward, PortForwardHandle, PortForwardInfo,
};
pub use ops::{
    kubectl_attach_command, kubectl_edit_command, kubectl_exec_command,
    kubectl_rollout_restart_command, LOG_BUFFER_MAX_LINES,
};
pub use plugin_loader::{
    ensure_plugins_dir, example_manifest_path, list_installed_plugins, plugins_dir,
    write_example_manifest_if_missing,
};
pub use portforward::{
    kubectl_port_forward_command, spawn_kubectl_attach_terminal, spawn_kubectl_exec_terminal,
    spawn_kubectl_port_forward,
};
pub use resources::CrdTarget;
pub use resources::{ResourceCategory, ResourceKind, ResourceRow};
pub use settings::{load_settings, save_settings, AppSettings, FavoriteResource};
pub use store::{ResourceSnapshot, WatchController};
