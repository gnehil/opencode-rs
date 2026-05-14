//! Wire protocol for the external-plugin subprocess bridge.
//!
//! The Rust side and the JS host (`host.mjs`) exchange newline-delimited JSON.
//! Rust sends [`HostRequest`] values; the host replies with [`HostEvent`]
//! values. `trigger` requests carry a correlation `id` so concurrent hook
//! invocations can be matched to their results.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The embedded JS host script, written to disk and run by node/bun.
pub const HOST_SCRIPT: &str = include_str!("host.mjs");

/// A resolved plugin the host should import and initialize.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginToLoad {
    /// The original configured spec, used for diagnostics.
    pub spec: String,
    /// The concrete import target (a `file://` URL or an npm package path).
    pub entry: String,
    /// Inline options passed to the plugin function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

/// The `PluginInput` payload handed to every plugin function on init.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInputData {
    pub directory: String,
    pub worktree: String,
    pub project: Value,
    pub server_url: String,
}

/// A request sent from Rust to the JS host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostRequest {
    /// Import and initialize the given plugins.
    Init {
        plugins: Vec<PluginToLoad>,
        input: PluginInputData,
    },
    /// Invoke a trigger-style hook `(input, output) -> output`.
    Trigger {
        id: u64,
        hook: String,
        input: Value,
        output: Value,
    },
    /// Invoke a one-way notification hook (`event`, `config`).
    Notify { hook: String, input: Value },
    /// Ask the host to exit.
    Shutdown,
}

/// A plugin that initialized successfully, with the hook names it registered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadedPlugin {
    pub spec: String,
    pub hooks: Vec<String>,
}

/// A plugin that failed to import or initialize.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginLoadError {
    pub spec: String,
    pub error: String,
}

/// A message sent from the JS host back to Rust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostEvent {
    /// Response to [`HostRequest::Init`].
    Ready {
        plugins: Vec<LoadedPlugin>,
        errors: Vec<PluginLoadError>,
    },
    /// Successful response to [`HostRequest::Trigger`], carrying the mutated
    /// `output`.
    TriggerResult { id: u64, output: Value },
    /// A [`HostRequest::Trigger`] that threw inside a plugin hook.
    TriggerError { id: u64, error: String },
    /// Out-of-band log line from the host or a plugin.
    Log { level: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_script_is_embedded() {
        assert!(HOST_SCRIPT.contains("opencode external plugin bridge host"));
        assert!(HOST_SCRIPT.contains("createInterface"));
    }

    #[test]
    fn init_request_round_trips() {
        let request = HostRequest::Init {
            plugins: vec![PluginToLoad {
                spec: "./plugin.ts".to_string(),
                entry: "file:///abs/plugin.ts".to_string(),
                options: Some(json!({ "key": "value" })),
            }],
            input: PluginInputData {
                directory: "/work".to_string(),
                worktree: "/work".to_string(),
                project: json!({ "id": "p1" }),
                server_url: "http://localhost:4096".to_string(),
            },
        };
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<HostRequest>(&text).unwrap(),
            request
        );
    }

    #[test]
    fn trigger_request_serializes_with_tag() {
        let request = HostRequest::Trigger {
            id: 7,
            hook: "tool.execute.before".to_string(),
            input: json!({ "tool": "bash" }),
            output: json!({ "args": {} }),
        };
        let value: Value = serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(value["type"], "trigger");
        assert_eq!(value["id"], 7);
        assert_eq!(value["hook"], "tool.execute.before");
    }

    #[test]
    fn host_events_round_trip() {
        let events = vec![
            HostEvent::Ready {
                plugins: vec![LoadedPlugin {
                    spec: "./plugin.ts".to_string(),
                    hooks: vec!["event".to_string(), "tool.execute.before".to_string()],
                }],
                errors: vec![PluginLoadError {
                    spec: "broken".to_string(),
                    error: "boom".to_string(),
                }],
            },
            HostEvent::TriggerResult {
                id: 3,
                output: json!({ "args": { "x": 1 } }),
            },
            HostEvent::TriggerError {
                id: 4,
                error: "hook threw".to_string(),
            },
            HostEvent::Log {
                level: "warn".to_string(),
                message: "something".to_string(),
            },
        ];
        for event in events {
            let text = serde_json::to_string(&event).unwrap();
            assert_eq!(serde_json::from_str::<HostEvent>(&text).unwrap(), event);
        }
    }

    #[test]
    fn host_event_parses_from_host_style_json() {
        // Exactly the shape host.mjs writes for a ready response.
        let line = r#"{"type":"ready","plugins":[{"spec":"a","hooks":["config"]}],"errors":[]}"#;
        match serde_json::from_str::<HostEvent>(line).unwrap() {
            HostEvent::Ready { plugins, errors } => {
                assert_eq!(plugins.len(), 1);
                assert_eq!(plugins[0].hooks, vec!["config".to_string()]);
                assert!(errors.is_empty());
            }
            other => panic!("expected ready, got {other:?}"),
        }
    }
}
