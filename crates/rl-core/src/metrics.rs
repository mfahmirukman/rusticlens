use kube::api::{Api, ListParams};
use kube::discovery::Discovery;
use kube::Client;
use serde_json::Value;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct PodMetricSummary {
    pub pod_name: String,
    pub cpu: String,
    pub memory: String,
}

pub async fn list_pod_metrics(
    client: &Client,
    namespace: &str,
) -> Result<Vec<PodMetricSummary>> {
    let discovery = Discovery::new(client.clone()).run().await?;
    let Some((ar, _)) = discovery
        .get("metrics.k8s.io")
        .and_then(|group| group.recommended_kind("PodMetrics"))
    else {
        return Ok(Vec::new());
    };

    let api: Api<kube::api::DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ar);
    let list = api.list(&ListParams::default()).await?;
    let rows = list
        .items
        .into_iter()
        .filter_map(parse_pod_metrics)
        .collect();
    Ok(rows)
}

fn parse_pod_metrics(obj: kube::api::DynamicObject) -> Option<PodMetricSummary> {
    let name = obj.metadata.name?;
    let containers = obj.data.get("containers")?.as_array()?;
    let mut cpu_nano: i64 = 0;
    let mut mem_bytes: i64 = 0;
    for container in containers {
        let usage = container.get("usage")?;
        if let Some(cpu) = usage.get("cpu").and_then(|v| v.as_str()) {
            cpu_nano += quantity_to_nano(cpu);
        }
        if let Some(mem) = usage.get("memory").and_then(|v| v.as_str()) {
            mem_bytes += quantity_to_bytes(mem);
        }
    }
    Some(PodMetricSummary {
        pod_name: name,
        cpu: format_cpu(cpu_nano),
        memory: format_memory(mem_bytes),
    })
}

fn quantity_to_nano(s: &str) -> i64 {
    if let Some(v) = s.strip_suffix('n') {
        return v.parse().unwrap_or(0);
    }
    if let Some(v) = s.strip_suffix('m') {
        return v.parse::<i64>().unwrap_or(0) * 1_000_000;
    }
    s.parse().unwrap_or(0) * 1_000_000_000
}

fn quantity_to_bytes(s: &str) -> i64 {
    if let Some(v) = s.strip_suffix("Ki") {
        return v.parse::<i64>().unwrap_or(0) * 1024;
    }
    if let Some(v) = s.strip_suffix("Mi") {
        return v.parse::<i64>().unwrap_or(0) * 1024 * 1024;
    }
    if let Some(v) = s.strip_suffix('k') {
        return v.parse::<i64>().unwrap_or(0) * 1000;
    }
    s.parse().unwrap_or(0)
}

fn format_cpu(nano: i64) -> String {
    if nano < 1_000_000 {
        format!("{nano}n")
    } else if nano < 1_000_000_000 {
        format!("{:.1}m", nano as f64 / 1_000_000.0)
    } else {
        format!("{:.2}", nano as f64 / 1_000_000_000.0)
    }
}

fn format_memory(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}Ki", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}Mi", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn format_metrics_text(metrics: &[PodMetricSummary]) -> String {
    if metrics.is_empty() {
        return "No metrics available (is metrics-server installed?)".to_string();
    }
    metrics
        .iter()
        .map(|m| format!("{}  CPU: {}  MEM: {}", m.pod_name, m.cpu, m.memory))
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(dead_code)]
fn _value_helper(v: &Value) -> Option<&str> {
    v.as_str()
}
