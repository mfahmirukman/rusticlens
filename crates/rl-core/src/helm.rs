use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ListParams};
use kube::Client;

use crate::error::Result;
use crate::resources::{format_age, ResourceRow};

pub async fn list_helm_releases(client: &Client, namespace: &str) -> Result<Vec<ResourceRow>> {
    let api: Api<Secret> = Api::namespaced(client.clone(), namespace);
    let lp = ListParams::default().labels("owner=helm");
    let list = api.list(&lp).await?;

    let rows = list
        .items
        .into_iter()
        .filter_map(|secret| {
            let name = secret.metadata.labels.as_ref()?.get("name")?.clone();
            let status = secret
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("status"))
                .cloned()
                .unwrap_or_else(|| "unknown".into());
            let version = secret
                .metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("version"))
                .map(|v| format!("v{v}"))
                .unwrap_or_else(|| "-".into());
            Some(ResourceRow::new(
                name,
                namespace.to_string(),
                version,
                status,
                "-".to_string(),
                format_age(secret.metadata.creation_timestamp.as_ref()),
            ))
        })
        .collect();

    Ok(rows)
}
