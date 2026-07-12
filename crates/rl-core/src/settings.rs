use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::resources::ResourceKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub last_context: Option<String>,
    pub last_namespace: Option<String>,
    pub last_kind: Option<String>,
    pub last_container: Option<String>,
    #[serde(default)]
    pub pinned_contexts: Vec<String>,
    pub bottom_panel_height: Option<f32>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_context: None,
            last_namespace: None,
            last_kind: Some(ResourceKind::Pod.label().to_string()),
            last_container: None,
            pinned_contexts: Vec::new(),
            bottom_panel_height: None,
        }
    }
}

pub fn settings_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("rusticlens").join("settings.json");
    }
    dirs_home().join(".config").join("rusticlens").join("settings.json")
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
