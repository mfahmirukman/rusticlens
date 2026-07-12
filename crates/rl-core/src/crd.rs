use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{Api, DynamicObject, ListParams};
use kube::discovery::{Discovery, Scope};
use kube::Client;
use serde_json::Value;

use crate::error::Result;
use crate::resources::{format_age, CrdTarget, ResourceRow};

pub async fn list_crds(client: &Client) -> Result<Vec<CrdTarget>> {
    let api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let list = api.list(&ListParams::default()).await?;
    let mut targets: Vec<CrdTarget> = list
        .items
        .into_iter()
        .filter_map(|crd| {
            let spec = crd.spec;
            let names = spec.names;
            Some(CrdTarget {
                group: spec.group,
                version: spec
                    .versions
                    .iter()
                    .find(|v| v.storage)
                    .map(|v| v.name.clone())
                    .or_else(|| spec.versions.first().map(|v| v.name.clone()))?,
                plural: names.plural,
                display_name: names.kind,
            })
        })
        .collect();
    targets.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(targets)
}

pub async fn list_crd_instances(
    client: &Client,
    namespace: &str,
    target: &CrdTarget,
) -> Result<Vec<ResourceRow>> {
    let discovery = Discovery::new(client.clone()).run().await?;
    let (ar, caps) = discovery
        .get(&target.group)
        .and_then(|group| group.recommended_kind(&target.display_name))
        .ok_or_else(|| {
            crate::error::Error::Message(format!(
                "CRD {} not found in discovery",
                target.display_name
            ))
        })?;

    let api = dynamic_api(&ar, caps.scope, client.clone(), namespace);
    let list = api.list(&ListParams::default()).await?;
    let rows = list
        .items
        .into_iter()
        .filter_map(|obj| object_to_row(obj, namespace))
        .collect();
    Ok(rows)
}

fn dynamic_api(
    ar: &kube::discovery::ApiResource,
    scope: Scope,
    client: Client,
    namespace: &str,
) -> Api<DynamicObject> {
    if scope == Scope::Cluster {
        Api::all_with(client, ar)
    } else {
        Api::namespaced_with(client, namespace, ar)
    }
}

fn object_to_row(obj: DynamicObject, namespace: &str) -> Option<ResourceRow> {
    let name = obj.metadata.name?;
    let age = format_age(obj.metadata.creation_timestamp.as_ref());
    let status = obj
        .data
        .get("status")
        .and_then(status_phase)
        .unwrap_or_else(|| "—".into());
    Some(ResourceRow::new(
        name,
        namespace.to_string(),
        "-".to_string(),
        status,
        "-".to_string(),
        age,
    ))
}

fn status_phase(status: &Value) -> Option<String> {
    status
        .get("phase")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}
