use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClusterCache {
    pub contexts: Vec<String>,
    pub namespaces_by_context: HashMap<String, Vec<String>>,
}

pub fn cluster_cache_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("rusticlens").join("cluster-cache.json");
    }
    dirs_home()
        .join(".cache")
        .join("rusticlens")
        .join("cluster-cache.json")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn load_cluster_cache() -> ClusterCache {
    let path = cluster_cache_path();
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => ClusterCache::default(),
    }
}

pub fn save_cluster_cache(cache: &ClusterCache) -> std::io::Result<()> {
    let path = cluster_cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(std::io::Error::other)?;
    fs::write(path, text)
}
