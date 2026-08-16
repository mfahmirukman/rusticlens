use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::resources::ResourceKind;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct FavoriteResource {
    /// `ResourceKind::api_kind()` value (e.g. `Pod`, `Deployment`).
    pub kind: String,
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub last_context: Option<String>,
    pub last_namespace: Option<String>,
    pub last_kind: Option<String>,
    pub last_container: Option<String>,
    #[serde(default)]
    pub pinned_contexts: Vec<String>,
    /// Last selected namespace per kubeconfig context name.
    #[serde(default)]
    pub context_namespaces: HashMap<String, String>,
    pub bottom_panel_height: Option<f32>,
    pub detail_panel_width: Option<f32>,
    /// `dark` (default) or `light`.
    #[serde(default)]
    pub theme: Option<String>,
    /// Additional kubeconfig files merged after the primary config.
    #[serde(default)]
    pub extra_kubeconfig_paths: Vec<String>,
    #[serde(default)]
    pub favorites: Vec<FavoriteResource>,
    /// Context names shown as multi-cluster tabs in the GUI.
    #[serde(default)]
    pub open_cluster_tabs: Vec<String>,
    /// Prefer kube-rs port-forward over spawning kubectl (default true).
    #[serde(default = "default_true")]
    pub use_native_port_forward: bool,
    /// Open logs in a new terminal emulator running `kubectl logs -f` (default true).
    /// Falls back to the built-in log view when no emulator can be launched.
    #[serde(default = "default_true")]
    pub external_logs: bool,
    /// Editor command for apply/edit YAML (e.g. `zed --wait`). Overrides `$VISUAL`/
    /// `$EDITOR`; `None` = use env vars / `vi` fallback.
    #[serde(default)]
    pub editor: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_context: None,
            last_namespace: None,
            last_kind: Some(ResourceKind::Pod.label().to_string()),
            last_container: None,
            pinned_contexts: Vec::new(),
            context_namespaces: HashMap::new(),
            bottom_panel_height: None,
            detail_panel_width: None,
            theme: None,
            extra_kubeconfig_paths: Vec::new(),
            favorites: Vec::new(),
            open_cluster_tabs: Vec::new(),
            use_native_port_forward: true,
            external_logs: true,
            editor: None,
        }
    }
}

pub fn settings_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("rusticlens").join("settings.json");
    }
    dirs_home()
        .join(".config")
        .join("rusticlens")
        .join("settings.json")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn load_settings() -> AppSettings {
    let path = settings_path();
    let Ok(content) = fs::read_to_string(&path) else {
        return AppSettings::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) -> std::io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(settings)?;
    fs::write(path, content)
}

/// Pick the namespace to use for `context`: saved choice if still valid, else first alphabetically.
pub fn pick_namespace_for_context(context: &str, available: &[String]) -> String {
    if let Some(saved) = saved_namespace_for_context(context) {
        if available.is_empty() || available.iter().any(|n| n == &saved) {
            return saved;
        }
    }

    if !available.is_empty() {
        return available
            .first()
            .cloned()
            .unwrap_or_else(|| String::from("default"));
    }

    String::from("default")
}

pub fn saved_namespace_for_context(context: &str) -> Option<String> {
    let settings = load_settings();
    if let Some(saved) = settings.context_namespaces.get(context) {
        return Some(saved.clone());
    }

    if settings.last_context.as_deref() == Some(context) {
        return settings.last_namespace.clone();
    }

    None
}

pub fn remember_namespace_for_context(context: &str, namespace: &str) {
    let mut settings = load_settings();
    settings.last_context = Some(context.to_string());
    settings.last_namespace = Some(namespace.to_string());
    settings
        .context_namespaces
        .insert(context.to_string(), namespace.to_string());
    let _ = save_settings(&settings);
}
