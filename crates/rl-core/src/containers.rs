use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use kube::Client;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub name: String,
    pub ready: bool,
    pub image: String,
}

pub async fn list_pod_containers(
    client: &Client,
    namespace: &str,
    pod_name: &str,
) -> Result<Vec<ContainerInfo>> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pod = api.get(pod_name).await?;
    Ok(extract_containers(&pod))
}

pub fn extract_containers(pod: &Pod) -> Vec<ContainerInfo> {
    let statuses: std::collections::HashMap<String, bool> = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .map(|list| list.iter().map(|s| (s.name.clone(), s.ready)).collect())
        .unwrap_or_default();

    pod.spec
        .as_ref()
        .map(|spec| &spec.containers)
        .map(|containers| {
            containers
                .iter()
                .map(|c| ContainerInfo {
                    name: c.name.clone(),
                    ready: statuses.get(&c.name).copied().unwrap_or(false),
                    image: c.image.clone().unwrap_or_else(|| "-".into()),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub async fn resolve_container(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    preferred: Option<&str>,
) -> Result<String> {
    let containers = list_pod_containers(client, namespace, pod_name).await?;
    if let Some(name) = preferred {
        if containers.iter().any(|c| c.name == name) {
            return Ok(name.to_string());
        }
    }

    containers
        .iter()
        .find(|c| c.ready)
        .map(|c| c.name.clone())
        .or_else(|| containers.first().map(|c| c.name.clone()))
        .ok_or_else(|| Error::Message(format!("no container found for pod {pod_name}")))
}
