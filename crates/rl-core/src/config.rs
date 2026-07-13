use std::path::{Path, PathBuf};

use kube::config::Kubeconfig;
use kube::Config;

use crate::error::{Error, Result};
use crate::settings::load_settings;

/// Load kubeconfig from the default path or `KUBECONFIG`, merged with saved extra paths.
pub fn load_kubeconfig() -> Result<Kubeconfig> {
    let mut config = kube::config::Kubeconfig::read().map_err(Error::from)?;
    for path in &load_settings().extra_kubeconfig_paths {
        let path = Path::new(path);
        if path.exists() {
            let extra = Kubeconfig::read_from(path).map_err(Error::from)?;
            config = Kubeconfig::merge(config, extra).map_err(Error::from)?;
        }
    }
    Ok(config)
}

/// Resolve the kubeconfig file path.
pub fn kubeconfig_path() -> PathBuf {
    if let Ok(path) = std::env::var("KUBECONFIG") {
        let first = path.split(':').next().unwrap_or(&path);
        return PathBuf::from(first);
    }
    dirs_home().join(".kube").join("config")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Build a client config for a named context.
pub async fn config_for_context(context: &str) -> Result<Config> {
    let mut kubeconfig = load_kubeconfig()?;
    kubeconfig.current_context = Some(context.to_string());
    Config::from_custom_kubeconfig(
        kubeconfig,
        &kube::config::KubeConfigOptions {
            context: Some(context.to_string()),
            ..Default::default()
        },
    )
    .await
    .map_err(Error::from)
}

/// List all context names from kubeconfig.
pub fn list_contexts() -> Result<Vec<String>> {
    let kubeconfig = load_kubeconfig()?;
    Ok(kubeconfig
        .contexts
        .iter()
        .map(|ctx| ctx.name.clone())
        .collect())
}

/// Return the current context from kubeconfig, if set.
pub fn current_context_name() -> Result<Option<String>> {
    let kubeconfig = load_kubeconfig()?;
    Ok(kubeconfig.current_context)
}

/// Whether a context's cluster and user entries exist in kubeconfig.
pub fn context_is_usable(context: &str) -> bool {
    let Ok(kubeconfig) = load_kubeconfig() else {
        return false;
    };
    kubeconfig_context_is_usable(&kubeconfig, context)
}

fn kubeconfig_context_is_usable(kubeconfig: &kube::config::Kubeconfig, context: &str) -> bool {
    let Some(ctx) = kubeconfig.contexts.iter().find(|c| c.name == context) else {
        return false;
    };
    let Some(inner) = &ctx.context else {
        return false;
    };
    let cluster_ok = kubeconfig
        .clusters
        .iter()
        .any(|c| c.name == inner.cluster);
    let user_ok = inner.user.as_ref().is_some_and(|user| {
        kubeconfig.auth_infos.iter().any(|u| u.name == *user)
    });
    cluster_ok && user_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kubeconfig_path_has_default() {
        let path = kubeconfig_path();
        assert!(
            path.to_string_lossy().contains("config") || path.to_string_lossy().contains("kube")
        );
    }
}
