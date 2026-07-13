use futures::{AsyncBufReadExt, StreamExt};
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Namespace, Node, PersistentVolumeClaim, Pod, Secret, Service,
};
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::{
    Api, DeleteParams, DynamicObject, ListParams, LogParams, Patch, PatchParams, PostParams,
};
use kube::discovery::{Discovery, Scope};
use kube::Client;
use kube_core::gvk::GroupVersionKind;
use serde::Deserialize;
use tokio::sync::mpsc::Sender;

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
        ResourceKind::NetworkPolicy => {
            let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::PersistentVolumeClaim => {
            let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::StorageClass => {
            let api: Api<StorageClass> = Api::all(client.clone());
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::Role => {
            let api: Api<Role> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::RoleBinding => {
            let api: Api<RoleBinding> = Api::namespaced(client.clone(), namespace);
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::ClusterRole => {
            let api: Api<ClusterRole> = Api::all(client.clone());
            serde_yaml::to_string(&api.get(name).await?)?
        }
        ResourceKind::ClusterRoleBinding => {
            let api: Api<ClusterRoleBinding> = Api::all(client.clone());
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
            let target = crd_target
                .ok_or_else(|| crate::error::Error::Message("no CRD type selected".into()))?;
            get_dynamic_yaml(client, namespace, &target.group, &target.display_name, name).await?
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
    force: bool,
) -> Result<()> {
    let params = if force {
        DeleteParams {
            grace_period_seconds: Some(0),
            ..Default::default()
        }
    } else {
        DeleteParams::default()
    };
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
        ResourceKind::NetworkPolicy => {
            Api::<NetworkPolicy>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::PersistentVolumeClaim => {
            Api::<PersistentVolumeClaim>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::StorageClass => {
            Api::<StorageClass>::all(client.clone())
                .delete(name, &params)
                .await?;
        }
        ResourceKind::Role => {
            Api::<Role>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::RoleBinding => {
            Api::<RoleBinding>::namespaced(client.clone(), namespace)
                .delete(name, &params)
                .await?;
        }
        ResourceKind::ClusterRole => {
            Api::<ClusterRole>::all(client.clone())
                .delete(name, &params)
                .await?;
        }
        ResourceKind::ClusterRoleBinding => {
            Api::<ClusterRoleBinding>::all(client.clone())
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
            let target = crd_target
                .ok_or_else(|| crate::error::Error::Message("no CRD type selected".into()))?;
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
pub const LOG_CHUNK_LINES: i64 = 500;
/// Initial tail when opening a log tab (Freelens uses 500).
pub const LOG_INITIAL_TAIL_LINES: i64 = 500;
/// Poll interval for incremental log fetches via `sinceTime`.
pub const LOG_POLL_INTERVAL_SECS: u64 = 10;
/// Must match `rl-app` log tab line cap (used to bound older-log API requests).
pub const LOG_BUFFER_MAX_LINES: usize = 2_000;

pub async fn fetch_pod_logs_tail(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
    timestamps: bool,
    tail_lines: i64,
) -> Result<Vec<String>> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let container = containers::resolve_container(client, namespace, pod_name, container).await?;

    let params = LogParams {
        container: Some(container),
        follow: false,
        tail_lines: Some(tail_lines),
        timestamps,
        ..Default::default()
    };

    read_log_lines(api, pod_name, &params).await
}

/// Fetch log lines newer than `since_time` (Kubernetes `sinceTime` query param).
pub async fn fetch_pod_logs_since(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
    since_time: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<String>> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let container = containers::resolve_container(client, namespace, pod_name, container).await?;

    let params = LogParams {
        container: Some(container),
        follow: false,
        since_time: Some(since_time),
        timestamps: true,
        ..Default::default()
    };

    read_log_lines(api, pod_name, &params).await
}

/// Parse an RFC3339 `sinceTime` value stored between polls.
pub fn parse_log_since_time(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// When no log lines exist yet, poll from "now".
pub fn fallback_log_since_time() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

/// RFC3339 timestamp prefix on Kubernetes log lines (when `timestamps=true`).
pub fn parse_log_line_timestamp(line: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let prefix = line.split_whitespace().next()?;
    chrono::DateTime::parse_from_rfc3339(prefix)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// `sinceTime` for the next poll: last line timestamp + 1s (avoids duplicates).
pub fn since_time_after_lines(lines: &[String]) -> Option<chrono::DateTime<chrono::Utc>> {
    let last = lines.last()?;
    let mut stamp = parse_log_line_timestamp(last)?;
    stamp += chrono::Duration::seconds(1);
    Some(stamp)
}

/// Strip the leading RFC3339 timestamp for display when the user hides timestamps.
pub fn strip_log_timestamp(line: &str) -> &str {
    match line.find(' ') {
        Some(idx) if parse_log_line_timestamp(line).is_some() => &line[idx + 1..],
        _ => line,
    }
}

/// Remove ANSI SGR/CSI escape sequences (colors, bold, etc.) from terminal output.
pub fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\x1b' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() {
                let b = bytes[i];
                i += 1;
                if b.is_ascii_alphabetic() || b == b'@' {
                    break;
                }
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod ansi_tests {
    use super::strip_ansi_codes;

    #[test]
    fn strips_sgr_codes() {
        let raw = "\x1b[1;4mhttp://example.com\x1b[0m";
        assert_eq!(strip_ansi_codes(raw), "http://example.com");
    }
}

async fn read_log_lines(api: Api<Pod>, pod_name: &str, params: &LogParams) -> Result<Vec<String>> {
    let stream = api.log_stream(pod_name, params).await?;
    let mut lines = stream.lines();
    let mut out = Vec::new();

    while let Some(line) = lines
        .next()
        .await
        .transpose()
        .map_err(|err| crate::error::Error::Message(err.to_string()))?
    {
        if !line.is_empty() {
            out.push(line);
        }
    }

    Ok(out)
}

pub async fn stream_pod_logs(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
    timestamps: bool,
    tx: Sender<String>,
) -> Result<()> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let container = containers::resolve_container(client, namespace, pod_name, container).await?;

    let params = LogParams {
        container: Some(container),
        follow: true,
        tail_lines: Some(MAX_LOG_LINES as i64),
        timestamps,
        ..Default::default()
    };

    let stream = api.log_stream(pod_name, &params).await?;
    let mut lines = stream.lines();

    while let Some(line) = lines
        .next()
        .await
        .transpose()
        .map_err(|err| crate::error::Error::Message(err.to_string()))?
    {
        if tx.send(line).await.is_err() {
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

pub fn kubectl_attach_command(namespace: &str, pod_name: &str, container: Option<&str>) -> String {
    match container {
        Some(c) => format!("kubectl attach -it -n {namespace} {pod_name} -c {c}"),
        None => format!("kubectl attach -it -n {namespace} {pod_name}"),
    }
}

pub fn kubectl_edit_command(namespace: &str, resource_kind: &str, name: &str) -> String {
    format!(
        "kubectl edit -n {namespace} {} {name}",
        resource_kind.to_lowercase()
    )
}

pub fn kubectl_rollout_restart_command(namespace: &str, name: &str) -> String {
    format!("kubectl rollout restart deployment/{name} -n {namespace}")
}

pub async fn trigger_cronjob(client: &Client, namespace: &str, name: &str) -> Result<()> {
    let api: Api<CronJob> = Api::namespaced(client.clone(), namespace);
    let cj = api.get(name).await?;
    let template = cj
        .spec
        .as_ref()
        .map(|s| s.job_template.clone())
        .ok_or_else(|| crate::error::Error::Message("cronjob has no job template".into()))?;

    let suffix = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
    let job_name = format!("{name}-manual-{suffix}");
    let owner_refs = cj.metadata.uid.as_ref().map(|uid| {
        vec![OwnerReference {
            api_version: "batch/v1".into(),
            kind: "CronJob".into(),
            name: name.to_string(),
            uid: uid.clone(),
            controller: Some(true),
            block_owner_deletion: Some(true),
        }]
    });

    let job = Job {
        metadata: kube::api::ObjectMeta {
            name: Some(job_name),
            namespace: Some(namespace.to_string()),
            owner_references: owner_refs,
            ..Default::default()
        },
        spec: template.spec,
        ..Default::default()
    };

    Api::<Job>::namespaced(client.clone(), namespace)
        .create(&PostParams::default(), &job)
        .await?;
    Ok(())
}

pub async fn set_cronjob_suspended(
    client: &Client,
    namespace: &str,
    name: &str,
    suspend: bool,
) -> Result<()> {
    let api: Api<CronJob> = Api::namespaced(client.clone(), namespace);
    let patch = serde_json::json!({ "spec": { "suspend": suspend } });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

pub async fn restart_deployment(client: &Client, namespace: &str, name: &str) -> Result<()> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    rollout_restart_patch(&api, name).await
}

pub async fn restart_statefulset(client: &Client, namespace: &str, name: &str) -> Result<()> {
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
    rollout_restart_patch(&api, name).await
}

async fn rollout_restart_patch<K>(api: &Api<K>, name: &str) -> Result<()>
where
    K: kube::Resource<DynamicType = ()>
        + Clone
        + std::fmt::Debug
        + serde::Serialize
        + serde::de::DeserializeOwned,
{
    let now = chrono::Utc::now().to_rfc3339();
    let patch = serde_json::json!({
        "spec": {
            "template": {
                "metadata": {
                    "annotations": {
                        "kubectl.kubernetes.io/restartedAt": now
                    }
                }
            }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Strategic(&patch))
        .await?;
    Ok(())
}

pub async fn scale_deployment(
    client: &Client,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> Result<()> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    scale_replicas_patch(&api, name, replicas).await
}

pub async fn scale_statefulset(
    client: &Client,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> Result<()> {
    let api: Api<StatefulSet> = Api::namespaced(client.clone(), namespace);
    scale_replicas_patch(&api, name, replicas).await
}

async fn scale_replicas_patch<K>(api: &Api<K>, name: &str, replicas: i32) -> Result<()>
where
    K: kube::Resource<DynamicType = ()>
        + Clone
        + std::fmt::Debug
        + serde::Serialize
        + serde::de::DeserializeOwned,
{
    let patch = serde_json::json!({ "spec": { "replicas": replicas } });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

/// Apply one or more YAML documents via server-side apply.
pub async fn apply_yaml(
    client: &Client,
    default_namespace: &str,
    yaml: &str,
) -> Result<Vec<String>> {
    let discovery = Discovery::new(client.clone()).run().await?;
    let mut applied = Vec::new();

    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let yaml_value: serde_yaml::Value = Deserialize::deserialize(doc)?;
        let json: serde_json::Value = serde_json::to_value(yaml_value)?;
        let api_version = json
            .get("apiVersion")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::error::Error::Message("missing apiVersion".into()))?;
        let kind_str = json
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::error::Error::Message("missing kind".into()))?
            .to_string();
        let metadata = json
            .get("metadata")
            .and_then(|m| m.as_object())
            .ok_or_else(|| crate::error::Error::Message("missing metadata".into()))?;
        let name = metadata
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::error::Error::Message("missing metadata.name".into()))?
            .to_string();
        let namespace = metadata
            .get("namespace")
            .and_then(|v| v.as_str())
            .unwrap_or(default_namespace)
            .to_string();

        let (group, version) = parse_api_version(api_version)?;
        let gvk = GroupVersionKind::gvk(&group, &version, &kind_str);
        let (ar, caps) = discovery.resolve_gvk(&gvk).ok_or_else(|| {
            crate::error::Error::Message(format!(
                "unknown apiVersion/kind: {api_version} {kind_str}"
            ))
        })?;

        let api = if caps.scope == Scope::Cluster {
            Api::<DynamicObject>::all_with(client.clone(), &ar)
        } else {
            Api::<DynamicObject>::namespaced_with(client.clone(), &namespace, &ar)
        };

        let pp = PatchParams::apply("rusticlens").force();
        api.patch(&name, &pp, &Patch::Apply(json))
            .await
            .map_err(crate::error::Error::from)?;
        applied.push(format!("{kind_str}/{name}"));
    }

    Ok(applied)
}

fn parse_api_version(api_version: &str) -> Result<(String, String)> {
    if let Some((group, version)) = api_version.split_once('/') {
        Ok((group.to_string(), version.to_string()))
    } else {
        Ok((String::new(), api_version.to_string()))
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
