use serde_json::Value;

use crate::error::Result as CoreResult;

/// Plugin hook for future extensions (v0.5+).
pub trait RusticlensPlugin: Send + Sync {
    fn name(&self) -> &str;
    fn on_cluster_connected(&self, context: &str) -> Result<(), String>;
}

pub struct PluginRegistry {
    plugins: Vec<Box<dyn RusticlensPlugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    pub fn register(&mut self, plugin: Box<dyn RusticlensPlugin>) {
        self.plugins.push(plugin);
    }

    pub fn notify_connected(&self, context: &str) {
        for plugin in &self.plugins {
            if let Err(err) = plugin.on_cluster_connected(context) {
                tracing::warn!("plugin {} error: {err}", plugin.name());
            }
        }
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in no-op plugin for testing the registry.
pub struct LoggingPlugin;

impl RusticlensPlugin for LoggingPlugin {
    fn name(&self) -> &str {
        "logging"
    }

    fn on_cluster_connected(&self, context: &str) -> Result<(), String> {
        tracing::info!("connected to {context}");
        Ok(())
    }
}

/// Dynamic resource YAML via API discovery.
pub async fn get_dynamic_yaml(
    client: &kube::Client,
    namespace: &str,
    group: &str,
    kind: &str,
    name: &str,
) -> CoreResult<String> {
    use kube::api::Api;
    use kube::discovery::{Discovery, Scope};

    let discovery = Discovery::new(client.clone()).run().await?;
    let (ar, caps) = discovery
        .get(group)
        .and_then(|api_group| api_group.recommended_kind(kind))
        .ok_or_else(|| crate::error::Error::Message("resource not in discovery".into()))?;

    let api: Api<kube::api::DynamicObject> = if caps.scope == Scope::Cluster {
        Api::all_with(client.clone(), &ar)
    } else {
        Api::namespaced_with(client.clone(), namespace, &ar)
    };

    let obj = api.get(name).await?;
    let value: Value = serde_json::to_value(&obj)
        .map_err(|err| crate::error::Error::Message(err.to_string()))?;
    Ok(serde_yaml::to_string(&value)?)
}
