//! Config-driven external plugin loading.
//!
//! Resolves the `plugin` entries from opencode config into concrete import
//! targets and spawns a [`PluginBridge`] for them. Local file plugins are
//! resolved directly; npm-package plugins are not yet supported (they need an
//! on-demand install step) and are skipped with a warning.

use serde_json::Value;

use crate::config::PluginSpec;
use crate::plugin::npm::install_npm_plugin;
use crate::plugin::spec::{
    is_deprecated_plugin, plugin_source, resolve_path_plugin_target, PluginSource,
};

use super::process::{detect_js_runtime, PluginBridge};
use super::protocol::{PluginInputData, PluginToLoad};

/// Resolve configured plugin specs and spawn a bridge for them.
///
/// Returns `Ok(None)` when there is nothing to load or no JS runtime is
/// available. Individual plugin import failures do not fail the whole load —
/// they are reported by [`PluginBridge::load_errors`].
pub async fn load_external_plugins(
    specs: &[PluginSpec],
    input: PluginInputData,
) -> anyhow::Result<Option<PluginBridge>> {
    let mut to_load = Vec::new();
    for spec in specs {
        let specifier = spec.specifier();
        if is_deprecated_plugin(specifier) {
            tracing::debug!("skipping deprecated built-in plugin spec: {specifier}");
            continue;
        }
        let options = spec
            .options()
            .map(|opts| Value::Object(opts.clone().into_iter().collect()));
        match plugin_source(specifier) {
            PluginSource::File => match resolve_path_plugin_target(specifier) {
                Ok(entry) => to_load.push(PluginToLoad {
                    spec: specifier.to_string(),
                    entry,
                    options,
                }),
                Err(error) => {
                    tracing::warn!("failed to resolve plugin {specifier}: {error}");
                }
            },
            PluginSource::Npm => match install_npm_plugin(specifier).await {
                Ok(entry) => to_load.push(PluginToLoad {
                    spec: specifier.to_string(),
                    entry,
                    options,
                }),
                Err(error) => {
                    tracing::warn!("failed to install npm plugin {specifier}: {error}");
                }
            },
        }
    }

    if to_load.is_empty() {
        return Ok(None);
    }

    let Some((runtime, _)) = detect_js_runtime() else {
        tracing::warn!(
            "{} external plugin(s) configured but no JS runtime (bun/node) found; skipping",
            to_load.len()
        );
        return Ok(None);
    };

    let bridge = PluginBridge::spawn(runtime, to_load, input).await?;
    for plugin in bridge.loaded_plugins() {
        tracing::info!(
            "loaded external plugin {} ({} hook(s))",
            plugin.spec,
            plugin.hooks.len()
        );
    }
    for error in bridge.load_errors() {
        tracing::error!("failed to load plugin {}: {}", error.spec, error.error);
    }
    Ok(Some(bridge))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> PluginInputData {
        PluginInputData {
            directory: "/work".to_string(),
            worktree: "/work".to_string(),
            project: serde_json::json!({}),
            server_url: "http://localhost:4096".to_string(),
        }
    }

    #[tokio::test]
    async fn empty_specs_load_nothing() {
        assert!(load_external_plugins(&[], sample_input())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn deprecated_specs_are_skipped() {
        let specs = vec![PluginSpec::Bare("opencode-copilot-auth".to_string())];
        assert!(load_external_plugins(&specs, sample_input())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn file_plugin_is_resolved_and_loaded() {
        if detect_js_runtime().is_none() {
            eprintln!("skipping: no JS runtime available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("plugin.mjs");
        std::fs::write(
            &plugin,
            r#"export default async function () {
                return { config: async () => {} }
            }"#,
        )
        .unwrap();

        let specs = vec![PluginSpec::Bare(plugin.to_string_lossy().to_string())];
        let bridge = load_external_plugins(&specs, sample_input())
            .await
            .unwrap()
            .expect("bridge expected for a resolvable file plugin");

        assert_eq!(bridge.loaded_plugins().len(), 1);
        assert!(bridge.has_hook("config"));
        bridge.shutdown().await.unwrap();
    }
}
