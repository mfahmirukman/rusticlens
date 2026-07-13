use std::time::Duration;

use chrono::Utc;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

/// A row displayed in the resource table.
#[derive(Debug, Clone)]
pub struct ResourceRow {
    pub name: String,
    pub namespace: String,
    pub ready: String,
    pub status: String,
    pub restarts: String,
    pub age: String,
    pub controlled_by: String,
    pub cpu: String,
    pub memory: String,
    /// CronJob: cron schedule expression
    pub schedule: String,
    /// CronJob: time zone (e.g. UTC)
    pub timezone: String,
    /// CronJob: True when not suspended
    pub resumed: String,
    /// CronJob: active child jobs; Deployment: available replicas
    pub active: String,
    /// CronJob: time since last schedule
    pub last_schedule: String,
    /// Deployment: up-to-date replica count
    pub up_to_date: String,
    /// Job: CronJob owner name
    pub owner: String,
    /// Service: ClusterIP, NodePort, LoadBalancer, etc.
    pub service_type: String,
    /// Service: cluster IP (or `None` for headless)
    pub cluster_ip: String,
    /// Service: external / load-balancer addresses
    pub external_ip: String,
    /// Service: display e.g. `8080/TCP, 8180/TCP`
    pub ports: String,
    /// Service: numeric ports from spec (for port-forward)
    pub service_ports: Vec<u16>,
}

impl ResourceRow {
    pub fn new(
        name: String,
        namespace: String,
        ready: String,
        status: String,
        restarts: String,
        age: String,
    ) -> Self {
        Self {
            name,
            namespace,
            ready,
            status,
            restarts,
            age,
            controlled_by: "-".into(),
            cpu: "N/A".into(),
            memory: "N/A".into(),
            schedule: "-".into(),
            timezone: "-".into(),
            resumed: "-".into(),
            active: "-".into(),
            last_schedule: "-".into(),
            up_to_date: "-".into(),
            owner: "-".into(),
            service_type: "-".into(),
            cluster_ip: "-".into(),
            external_ip: "-".into(),
            ports: "-".into(),
            service_ports: Vec::new(),
        }
    }
}

/// Sidebar category for grouping resource kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceCategory {
    Workloads,
    Network,
    Storage,
    Access,
    Config,
    Cluster,
    Custom,
}

/// Supported resource kinds in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Pod,
    Deployment,
    StatefulSet,
    Job,
    CronJob,
    Service,
    Ingress,
    ConfigMap,
    Secret,
    Namespace,
    Node,
    NetworkPolicy,
    PersistentVolumeClaim,
    StorageClass,
    Role,
    RoleBinding,
    ClusterRole,
    ClusterRoleBinding,
    HelmRelease,
    Crd,
}

impl ResourceKind {
    pub const ALL: [ResourceKind; 20] = [
        ResourceKind::Pod,
        ResourceKind::Deployment,
        ResourceKind::StatefulSet,
        ResourceKind::Job,
        ResourceKind::CronJob,
        ResourceKind::Service,
        ResourceKind::Ingress,
        ResourceKind::NetworkPolicy,
        ResourceKind::PersistentVolumeClaim,
        ResourceKind::StorageClass,
        ResourceKind::Role,
        ResourceKind::RoleBinding,
        ResourceKind::ClusterRole,
        ResourceKind::ClusterRoleBinding,
        ResourceKind::ConfigMap,
        ResourceKind::Secret,
        ResourceKind::Namespace,
        ResourceKind::Node,
        ResourceKind::HelmRelease,
        ResourceKind::Crd,
    ];

    pub fn category(self) -> ResourceCategory {
        match self {
            ResourceKind::Pod
            | ResourceKind::Deployment
            | ResourceKind::StatefulSet
            | ResourceKind::Job
            | ResourceKind::CronJob => ResourceCategory::Workloads,
            ResourceKind::Service | ResourceKind::Ingress | ResourceKind::NetworkPolicy => {
                ResourceCategory::Network
            }
            ResourceKind::PersistentVolumeClaim | ResourceKind::StorageClass => {
                ResourceCategory::Storage
            }
            ResourceKind::Role
            | ResourceKind::RoleBinding
            | ResourceKind::ClusterRole
            | ResourceKind::ClusterRoleBinding => ResourceCategory::Access,
            ResourceKind::ConfigMap | ResourceKind::Secret => ResourceCategory::Config,
            ResourceKind::Namespace | ResourceKind::Node | ResourceKind::HelmRelease => {
                ResourceCategory::Cluster
            }
            ResourceKind::Crd => ResourceCategory::Custom,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ResourceKind::Pod => "Pods",
            ResourceKind::Deployment => "Deployments",
            ResourceKind::StatefulSet => "StatefulSets",
            ResourceKind::Job => "Jobs",
            ResourceKind::CronJob => "Cron Jobs",
            ResourceKind::Service => "Services",
            ResourceKind::Ingress => "Ingresses",
            ResourceKind::NetworkPolicy => "Network Policies",
            ResourceKind::PersistentVolumeClaim => "PVCs",
            ResourceKind::StorageClass => "Storage Classes",
            ResourceKind::Role => "Roles",
            ResourceKind::RoleBinding => "Role Bindings",
            ResourceKind::ClusterRole => "Cluster Roles",
            ResourceKind::ClusterRoleBinding => "Cluster Role Bindings",
            ResourceKind::ConfigMap => "ConfigMaps",
            ResourceKind::Secret => "Secrets",
            ResourceKind::Namespace => "Namespaces",
            ResourceKind::Node => "Nodes",
            ResourceKind::HelmRelease => "Helm Releases",
            ResourceKind::Crd => "Custom Resources",
        }
    }

    pub fn api_kind(self) -> &'static str {
        match self {
            ResourceKind::Pod => "Pod",
            ResourceKind::Deployment => "Deployment",
            ResourceKind::StatefulSet => "StatefulSet",
            ResourceKind::Job => "Job",
            ResourceKind::CronJob => "CronJob",
            ResourceKind::Service => "Service",
            ResourceKind::Ingress => "Ingress",
            ResourceKind::NetworkPolicy => "NetworkPolicy",
            ResourceKind::PersistentVolumeClaim => "PersistentVolumeClaim",
            ResourceKind::StorageClass => "StorageClass",
            ResourceKind::Role => "Role",
            ResourceKind::RoleBinding => "RoleBinding",
            ResourceKind::ClusterRole => "ClusterRole",
            ResourceKind::ClusterRoleBinding => "ClusterRoleBinding",
            ResourceKind::ConfigMap => "ConfigMap",
            ResourceKind::Secret => "Secret",
            ResourceKind::Namespace => "Namespace",
            ResourceKind::Node => "Node",
            ResourceKind::HelmRelease => "HelmRelease",
            ResourceKind::Crd => "CustomResource",
        }
    }

    pub fn is_cluster_scoped(self) -> bool {
        matches!(
            self,
            ResourceKind::Namespace
                | ResourceKind::Node
                | ResourceKind::StorageClass
                | ResourceKind::ClusterRole
                | ResourceKind::ClusterRoleBinding
        )
    }

    /// True for kinds backed by a live Kubernetes watch (not Helm/CRD polls).
    pub fn uses_watch(self) -> bool {
        !matches!(self, ResourceKind::HelmRelease | ResourceKind::Crd)
    }

    pub fn supports_logs(self) -> bool {
        self == ResourceKind::Pod
    }

    /// Horizontal tabs shown for the Workloads section (Freelens-style).
    pub const WORKLOAD_TABS: [ResourceKind; 4] = [
        ResourceKind::Pod,
        ResourceKind::Deployment,
        ResourceKind::Job,
        ResourceKind::CronJob,
    ];

    pub fn is_workload_tab(self) -> bool {
        Self::WORKLOAD_TABS.contains(&self)
    }

    pub fn supports_port_forward(self) -> bool {
        self == ResourceKind::Pod || self == ResourceKind::Service
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.label() == label)
    }

    pub fn from_api_kind(api_kind: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.api_kind() == api_kind)
    }
}

/// Identifies a CRD type for dynamic listing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CrdTarget {
    pub group: String,
    pub version: String,
    pub plural: String,
    pub display_name: String,
}

pub fn format_age(timestamp: Option<&Time>) -> String {
    let Some(time) = timestamp else {
        return "-".to_string();
    };

    let duration = Utc::now().signed_duration_since(time.0);
    format_duration(duration.to_std().unwrap_or(Duration::ZERO))
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

pub fn pod_ready_string(ready_count: i32, total: i32) -> String {
    format!("{ready_count}/{total}")
}
