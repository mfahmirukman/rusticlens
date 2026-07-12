use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Node, Pod, Secret, Service};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{Api, DeleteParams, ListParams, LogParams};
use kube::Client;
use tokio::sync::mpsc::UnboundedSender;

use crate::containers;
use crate::crd;
use crate::error::Result;
use crate::plugins::get_dynamic_yaml;
use crate::resources::{CrdTarget, ResourceKind};

pub async fn list_namespaces(client: &Client) -> Result<Vec<String>> {
    let api: Api<Namespace> = Api::all(client.clone());
    let list = api.list(&ListParams::default()).await?;
    let mut names: Vec<String> = list
        .items
        .into_iter()
        .filter_map(|ns| ns.metadata.name)
        .collect();
    names.sort();
    Ok(names)
}

pub async fn get_resource_yaml(
    client: &Client,
    namespace: &str,
    kind: ResourceKind,
    name: &str,
    crd_target: Option<&CrdTarget>,
) -> Result<String> {
    let yaml = match kind {
        ResourceKind::Pod => {
            let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Deployment => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::StatefulSet => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Job => {
            let api: Api<Job> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::CronJob => {
            let api: Api<CronJob> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Service => {
            let api: Api<Service> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Ingress => {
            let api: Api<Ingress> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::ConfigMap => {
            let api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Secret => {
            let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Namespace => {
            let api: Api<Namespace> = Api::all(client.clone());
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Node => {
            let api: Api<Node> = Api::all(client.clone());
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::HelmRelease => {
            let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Crd => {
            let target = crd_target.ok_or_else(|| {
                crate::error::Error::Message("no CRD type selected".into())
            })?;
            get_dynamic_yaml(
                client,
                namespace,
                &target.group,
                &target.display_name,
                name,
            )
            .await?
        }
    };
    Ok(yaml)
}

pub async fn delete_resource(
    client: &Client,
    namespace: &str,
    kind: ResourceKind,
    name: &str,
    crd_target: Option<&CrdTarget>,
) -> Result<()> {
    let params = DeleteParams::default();
    match kind {
        ResourceKind::Pod => {
            Api::<Pod>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Deployment => {
            Api::<Deployment>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::StatefulSet => {
            Api::<StatefulSet>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Job => {
            Api::<Job>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::CronJob => {
            Api::<CronJob>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Service => {
            Api::<Service>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Ingress => {
            Api::<Ingress>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::ConfigMap => {
            Api::<ConfigMap>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Secret | ResourceKind::HelmRelease => {
            Api::<Secret>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Namespace => {
            Api::<Namespace>::all(client.clone())
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Node => {
            Api::<Node>::all(client.clone())
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Crd => {
            let target = crd_target.ok_or_else(|| {
                crate::error::Error::Message("no CRD type selected".into())
            })?;
            delete_dynamic(client, namespace, target, name).await?;
        }
    }
    Ok(())
}

async fn delete_dynamic(
    client: &Client,
    namespace: &str,
    target: &CrdTarget,
    name: &str,
) -> Result<()> {
    use kube::api::Api;
    use kube::discovery::{Discovery, Scope};

    let discovery = Discovery::new(client.clone()).run().await?;
    let (ar, caps) = discovery
        .get(&target.group)
        .and_then(|group| group.recommended_kind(&target.display_name))
        .ok_or_else(|| crate::error::Error::Message("CRD not in discovery".into()))?;

    let api: Api<kube::api::DynamicObject> = if caps.scope == Scope::Cluster {
        Api::all_with(client.clone(), &ar)
    } else {
        Api::namespaced_with(client.clone(), namespace, &ar)
    };
    api.delete(name, &DeleteParams::default()).await?;
    Ok(())
}

const MAX_LOG_LINES: usize = 500;

pub async fn stream_pod_logs(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
    tx: UnboundedSender<String>,
) -> Result<()> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let container =
        containers::resolve_container(client, namespace, pod_name, container).await?;

    let params = LogParams {
        container: Some(container),
        follow: true,
        tail_lines: Some(MAX_LOG_LINES as i64),
        ..Default::default()
    };

    let stream = api.log_stream(pod_name, &params).await?;
    let mut lines = stream.lines();
    let mut line_count = 0usize;

    while let Some(line) = lines
        .next()
        .await
        .transpose()
        .map_err(|err| crate::error::Error::Message(err.to_string()))?
    {
        if tx.send(line).is_err() {
            break;
        }
        line_count += 1;
        if line_count > MAX_LOG_LINES * 2 {
            let _ = tx.send("--- log buffer limit reached ---".to_string());
            break;
        }
    }

    Ok(())
}

pub fn kubectl_exec_command(namespace: &str, pod_name: &str, container: Option<&str>) -> String {
    match container {
        Some(c) => format!("kubectl exec -it -n {namespace} {pod_name} -c {c} -- /bin/sh"),
        None => format!("kubectl exec -it -n {namespace} {pod_name} -- /bin/sh"),
    }
}

pub async fn list_crd_targets(client: &Client) -> Result<Vec<CrdTarget>> {
    crd::list_crds(client).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kubectl_exec_formats_command() {
        let cmd = kubectl_exec_command("default", "api-123", Some("app"));
        assert!(cmd.contains("kubectl exec"));
        assert!(cmd.contains("-c app"));
    }
}
