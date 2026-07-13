use std::fs;
use std::process::Command;

use serde::Deserialize;

use crate::plugins::{PluginRegistry, RusticlensPlugin};

#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Shell command invoked on cluster connect. Env: `RUSTICLENS_CONTEXT`.
    #[serde(default)]
    pub on_connect: Option<String>,
}

struct ManifestPlugin {
    manifest: PluginManifest,
}

impl RusticlensPlugin for ManifestPlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn on_cluster_connected(&self, context: &str) -> Result<(), String> {
        let Some(cmd) = &self.manifest.on_connect else {
            return Ok(());
        };
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .env("RUSTICLENS_CONTEXT", context)
            .spawn()
            .map_err(|err| err.to_string())?;
        Ok(())
    }
}

pub fn plugins_dir() -> std::path::PathBuf {
    crate::settings::settings_path()
        .parent()
        .map(|p| p.join("plugins"))
        .unwrap_or_else(|| std::path::PathBuf::from(".config/rusticlens/plugins"))
}

pub fn load_manifest_plugins(registry: &mut PluginRegistry) {
    let dir = plugins_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            tracing::warn!("plugin: failed to read {}", path.display());
            continue;
        };
        match toml::from_str::<PluginManifest>(&content) {
            Ok(manifest) => {
                tracing::info!("loaded plugin {}", manifest.name);
                registry.register(Box::new(ManifestPlugin { manifest }));
            }
            Err(err) => {
                tracing::warn!("plugin: invalid {}: {err}", path.display());
            }
        }
    }
}

pub fn example_manifest_path() -> std::path::PathBuf {
    plugins_dir().join("example.toml")
}

pub fn write_example_manifest_if_missing() {
    let dir = plugins_dir();
    let _ = fs::create_dir_all(&dir);
    let path = example_manifest_path();
    if path.exists() {
        return;
    }
    let example = r#"# rusticlens plugin manifest (TOML)
# Place .toml files in this directory to extend rusticlens.

name = "example"
description = "Logs when a cluster connection succeeds"

# Optional shell hook (runs in background via `sh -c`)
on_connect = "echo \"rusticlens connected to $RUSTICLENS_CONTEXT\""
"#;
    let _ = fs::write(path, example);
}

pub fn list_installed_plugins() -> Vec<PluginManifest> {
    let dir = plugins_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(manifest) = toml::from_str(&content) {
                out.push(manifest);
            }
        }
    }
    out
}

pub fn ensure_plugins_dir() {
    let _ = fs::create_dir_all(plugins_dir());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest() {
        let raw = r#"
name = "demo"
on_connect = "echo hi"
"#;
        let m: PluginManifest = toml::from_str(raw).unwrap();
        assert_eq!(m.name, "demo");
        assert_eq!(m.on_connect.as_deref(), Some("echo hi"));
    }
}
