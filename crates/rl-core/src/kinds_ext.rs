use std::collections::HashMap;

use futures::StreamExt;
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use k8s_openapi::api::networking::v1::NetworkPolicy;
use k8s_openapi::api::rbac::v1::{
    ClusterRole, ClusterRoleBinding, Role, RoleBinding,
};
use k8s_openapi::api::storage::v1::StorageClass;
use kube::api::{Api, ListParams};
use kube::runtime::watcher::{watcher, Config as WatchConfig, Event};
use kube::Client;

use crate::resources::{format_age, ResourceRow};
use crate::store::{apply_delta, WatchDelta};

pub fn networkpolicy_to_row(np: &NetworkPolicy, namespace: &str) -> ResourceRow {
    let name = np.metadata.name.clone().unwrap_or_default();
    let policy_types = np
        .spec
        .as_ref()
        .and_then(|s| s.policy_types.as_ref())
        .map(|t| t.join(", "))
        .unwrap_or_else(|| "-".into());
    let ingress = np
        .spec
        .as_ref()
        .and_then(|s| s.ingress.as_ref())
        .map(|r| r.len())
        .unwrap_or(0);
    let egress = np
        .spec
        .as_ref()
        .and_then(|s| s.egress.as_ref())
        .map(|r| r.len())
        .unwrap_or(0);

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        policy_types,
        format!("in:{ingress} out:{egress}"),
        "-".into(),
        format_age(np.metadata.creation_timestamp.as_ref()),
    );
    row.controlled_by = np
        .spec
        .as_ref()
        .map(|s| {
            s.pod_selector
                .match_labels
                .as_ref()
                .map(|m| format!("{} labels", m.len()))
                .unwrap_or_else(|| "all pods".into())
        })
        .unwrap_or_else(|| "-".into());
    row
}

pub fn pvc_to_row(pvc: &PersistentVolumeClaim, namespace: &str) -> ResourceRow {
    let name = pvc.metadata.name.clone().unwrap_or_default();
    let phase = pvc
        .status
        .as_ref()
        .and_then(|s| s.phase.clone())
        .unwrap_or_else(|| "Pending".into());
    let capacity = pvc
        .status
        .as_ref()
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("storage"))
        .map(|q| q.0.clone())
        .unwrap_or_else(|| "-".into());
    let request = pvc
        .spec
        .as_ref()
        .and_then(|s| s.resources.as_ref())
        .and_then(|r| r.requests.as_ref())
        .and_then(|req| req.get("storage"))
        .map(|q| q.0.clone())
        .unwrap_or_else(|| "-".into());
    let storage_class = pvc
        .spec
        .as_ref()
        .and_then(|s| s.storage_class_name.clone())
        .unwrap_or_else(|| "-".into());

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        phase.clone(),
        capacity,
        "-".into(),
        format_age(pvc.metadata.creation_timestamp.as_ref()),
    );
    row.memory = request;
    row.controlled_by = storage_class;
    row.status = phase;
    row
}

pub fn storageclass_to_row(sc: &StorageClass) -> ResourceRow {
    let name = sc.metadata.name.clone().unwrap_or_default();
    let provisioner = sc.provisioner.clone();
    let binding = sc
        .volume_binding_mode
        .clone()
        .unwrap_or_else(|| "Immediate".into());
    let reclaim = sc
        .reclaim_policy
        .clone()
        .unwrap_or_else(|| "Delete".into());

    let mut row = ResourceRow::new(
        name,
        "-".into(),
        provisioner,
        binding,
        "-".into(),
        format_age(sc.metadata.creation_timestamp.as_ref()),
    );
    row.memory = reclaim;
    row
}

pub fn role_to_row(role: &Role, namespace: &str) -> ResourceRow {
    let name = role.metadata.name.clone().unwrap_or_default();
    let rules = role
        .rules
        .as_ref()
        .map(|r| r.len())
        .unwrap_or(0);

    ResourceRow::new(
        name,
        namespace.to_string(),
        rules.to_string(),
        "Role".into(),
        "-".into(),
        format_age(role.metadata.creation_timestamp.as_ref()),
    )
}

pub fn rolebinding_to_row(rb: &RoleBinding, namespace: &str) -> ResourceRow {
    let name = rb.metadata.name.clone().unwrap_or_default();
    let subjects = rb.subjects.as_ref().map(|s| s.len()).unwrap_or(0);
    let role_ref = rb
        .role_ref
        .name
        .clone();

    let mut row = ResourceRow::new(
        name,
        namespace.to_string(),
        subjects.to_string(),
        "Binding".into(),
        "-".into(),
        format_age(rb.metadata.creation_timestamp.as_ref()),
    );
    row.controlled_by = role_ref;
    row
}

pub fn clusterrole_to_row(cr: &ClusterRole) -> ResourceRow {
    let name = cr.metadata.name.clone().unwrap_or_default();
    let rules = cr.rules.as_ref().map(|r| r.len()).unwrap_or(0);

    ResourceRow::new(
        name,
        "-".into(),
        rules.to_string(),
        "ClusterRole".into(),
        "-".into(),
        format_age(cr.metadata.creation_timestamp.as_ref()),
    )
}

pub fn clusterrolebinding_to_row(crb: &ClusterRoleBinding) -> ResourceRow {
    let name = crb.metadata.name.clone().unwrap_or_default();
    let subjects = crb.subjects.as_ref().map(|s| s.len()).unwrap_or(0);
    let role_ref = crb.role_ref.name.clone();

    let mut row = ResourceRow::new(
        name,
        "-".into(),
        subjects.to_string(),
        "Binding".into(),
        "-".into(),
        format_age(crb.metadata.creation_timestamp.as_ref()),
    );
    row.controlled_by = role_ref;
    row
}

pub fn watch_networkpolicies(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<NetworkPolicy> = Api::namespaced(client, &namespace);
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(np)) | Ok(Event::InitApply(np)) => {
                WatchDelta::Upsert(Box::new(networkpolicy_to_row(&np, &namespace)))
            }
            Ok(Event::Delete(np)) => WatchDelta::Remove(np.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

pub fn watch_pvcs(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client, &namespace);
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(pvc)) | Ok(Event::InitApply(pvc)) => {
                WatchDelta::Upsert(Box::new(pvc_to_row(&pvc, &namespace)))
            }
            Ok(Event::Delete(pvc)) => WatchDelta::Remove(pvc.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

pub fn watch_storageclasses(client: Client) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<StorageClass> = Api::all(client);
    watcher(api, WatchConfig::default())
        .map(|event| match event {
            Ok(Event::Apply(sc)) | Ok(Event::InitApply(sc)) => {
                WatchDelta::Upsert(Box::new(storageclass_to_row(&sc)))
            }
            Ok(Event::Delete(sc)) => WatchDelta::Remove(sc.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

pub fn watch_roles(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<Role> = Api::namespaced(client, &namespace);
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(role)) | Ok(Event::InitApply(role)) => {
                WatchDelta::Upsert(Box::new(role_to_row(&role, &namespace)))
            }
            Ok(Event::Delete(role)) => WatchDelta::Remove(role.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

pub fn watch_rolebindings(
    client: Client,
    namespace: String,
) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<RoleBinding> = Api::namespaced(client, &namespace);
    watcher(api, WatchConfig::default())
        .map(move |event| match event {
            Ok(Event::Apply(rb)) | Ok(Event::InitApply(rb)) => {
                WatchDelta::Upsert(Box::new(rolebinding_to_row(&rb, &namespace)))
            }
            Ok(Event::Delete(rb)) => WatchDelta::Remove(rb.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

pub fn watch_clusterroles(client: Client) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<ClusterRole> = Api::all(client);
    watcher(api, WatchConfig::default())
        .map(|event| match event {
            Ok(Event::Apply(cr)) | Ok(Event::InitApply(cr)) => {
                WatchDelta::Upsert(Box::new(clusterrole_to_row(&cr)))
            }
            Ok(Event::Delete(cr)) => WatchDelta::Remove(cr.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

pub fn watch_clusterrolebindings(client: Client) -> impl futures::Stream<Item = Vec<ResourceRow>> {
    let api: Api<ClusterRoleBinding> = Api::all(client);
    watcher(api, WatchConfig::default())
        .map(|event| match event {
            Ok(Event::Apply(crb)) | Ok(Event::InitApply(crb)) => {
                WatchDelta::Upsert(Box::new(clusterrolebinding_to_row(&crb)))
            }
            Ok(Event::Delete(crb)) => WatchDelta::Remove(crb.metadata.name.unwrap_or_default()),
            Ok(Event::Init) | Ok(Event::InitDone) => WatchDelta::Noop,
            Err(_) => WatchDelta::Noop,
        })
        .scan(HashMap::<String, ResourceRow>::new(), |map, delta| {
            apply_delta(map, delta);
            let mut values: Vec<_> = map.values().cloned().collect();
            values.sort_by(|a, b| a.name.cmp(&b.name));
            futures::future::ready(Some(values))
        })
}

pub async fn list_networkpolicies(client: &Client, namespace: &str) -> crate::error::Result<Vec<ResourceRow>> {
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), namespace);
    Ok(api
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(|np| networkpolicy_to_row(np, namespace))
        .collect())
}

pub async fn list_pvcs(client: &Client, namespace: &str) -> crate::error::Result<Vec<ResourceRow>> {
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    Ok(api
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(|pvc| pvc_to_row(pvc, namespace))
        .collect())
}

pub async fn list_storageclasses(client: &Client) -> crate::error::Result<Vec<ResourceRow>> {
    let api: Api<StorageClass> = Api::all(client.clone());
    Ok(api
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(storageclass_to_row)
        .collect())
}

pub async fn list_roles(client: &Client, namespace: &str) -> crate::error::Result<Vec<ResourceRow>> {
    let api: Api<Role> = Api::namespaced(client.clone(), namespace);
    Ok(api
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(|r| role_to_row(r, namespace))
        .collect())
}

pub async fn list_rolebindings(client: &Client, namespace: &str) -> crate::error::Result<Vec<ResourceRow>> {
    let api: Api<RoleBinding> = Api::namespaced(client.clone(), namespace);
    Ok(api
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(|rb| rolebinding_to_row(rb, namespace))
        .collect())
}

pub async fn list_clusterroles(client: &Client) -> crate::error::Result<Vec<ResourceRow>> {
    let api: Api<ClusterRole> = Api::all(client.clone());
    Ok(api
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(clusterrole_to_row)
        .collect())
}

pub async fn list_clusterrolebindings(client: &Client) -> crate::error::Result<Vec<ResourceRow>> {
    let api: Api<ClusterRoleBinding> = Api::all(client.clone());
    Ok(api
        .list(&ListParams::default())
        .await?
        .items
        .iter()
        .map(clusterrolebinding_to_row)
        .collect())
}
