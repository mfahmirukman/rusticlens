use std::collections::HashMap;

use k8s_openapi::api::core::v1::{Node, Pod};
use kube::api::{Api, ListParams};
use kube::Client;

use crate::error::Result;

#[derive(Debug, Clone, Default)]
pub struct NodeDashboardRow {
    pub name: String,
    pub ready: String,
    pub pods: usize,
    pub cpu_allocatable: String,
    pub memory_allocatable: String,
    pub cpu_capacity: String,
    pub memory_capacity: String,
}

#[derive(Debug, Clone, Default)]
pub struct ClusterDashboard {
    pub nodes: Vec<NodeDashboardRow>,
    pub total_pods: usize,
    pub running_pods: usize,
    pub pending_pods: usize,
    pub failed_pods: usize,
    pub succeeded_pods: usize,
    pub unknown_pods: usize,
}

pub async fn fetch_cluster_dashboard(client: &Client) -> Result<ClusterDashboard> {
    let nodes_api: Api<Node> = Api::all(client.clone());
    let pods_api: Api<Pod> = Api::all(client.clone());

    let node_list = nodes_api.list(&ListParams::default()).await?;
    let pod_list = pods_api.list(&ListParams::default()).await?;

    let mut pods_by_node: HashMap<String, usize> = HashMap::new();
    let mut running_pods = 0usize;
    let mut pending_pods = 0usize;
    let mut failed_pods = 0usize;
    let mut succeeded_pods = 0usize;
    let mut unknown_pods = 0usize;

    for pod in &pod_list.items {
        let phase = pod
            .status
            .as_ref()
            .and_then(|s| s.phase.as_deref())
            .unwrap_or("Unknown");
        match phase {
            "Running" => running_pods += 1,
            "Pending" => pending_pods += 1,
            "Failed" => failed_pods += 1,
            "Succeeded" => succeeded_pods += 1,
            _ => unknown_pods += 1,
        }
        if let Some(node) = pod.spec.as_ref().and_then(|s| s.node_name.as_deref()) {
            *pods_by_node.entry(node.to_string()).or_default() += 1;
        }
    }

    let nodes = node_list
        .items
        .iter()
        .map(|node| {
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

            let (cpu_capacity, memory_capacity, cpu_allocatable, memory_allocatable) = node
                .status
                .as_ref()
                .map(|s| {
                    (
                        quantity_str(s.capacity.as_ref().and_then(|m| m.get("cpu"))),
                        quantity_str(s.capacity.as_ref().and_then(|m| m.get("memory"))),
                        quantity_str(s.allocatable.as_ref().and_then(|m| m.get("cpu"))),
                        quantity_str(s.allocatable.as_ref().and_then(|m| m.get("memory"))),
                    )
                })
                .unwrap_or_else(|| ("-".into(), "-".into(), "-".into(), "-".into()));

            NodeDashboardRow {
                name: name.clone(),
                ready,
                pods: pods_by_node.get(&name).copied().unwrap_or(0),
                cpu_allocatable,
                memory_allocatable,
                cpu_capacity,
                memory_capacity,
            }
        })
        .collect();

    Ok(ClusterDashboard {
        nodes,
        total_pods: pod_list.items.len(),
        running_pods,
        pending_pods,
        failed_pods,
        succeeded_pods,
        unknown_pods,
    })
}

fn quantity_str(
    q: Option<&k8s_openapi::apimachinery::pkg::api::resource::Quantity>,
) -> String {
    q.map(|q| q.0.clone()).unwrap_or_else(|| "-".into())
}
