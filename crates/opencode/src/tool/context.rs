use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::id::{MessageID, PartID, SessionID};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: SessionID,
    pub working_dir: std::path::PathBuf,
    pub permission_rules: crate::permission::Ruleset,
    pub event_bus: Option<crate::bus::EventBus>,
    pub permission_broker: Option<crate::permission::PermissionBroker>,
    pub provider: Option<Arc<dyn crate::provider::Provider>>,
    pub store: Option<Arc<crate::session::SessionStore>>,
    pub config: Option<crate::config::Config>,
    pub agent_name: Option<String>,
    pub model_id: Option<String>,
    pub plugin_manager: Option<Arc<crate::plugin::PluginManager>>,
}

impl ToolContext {
    /// Resolve whether `permission` should be granted for `pattern`. Empty
    /// rulesets allow by default so that callers (CLI, tests) that haven't
    /// loaded any rules don't break. Matching uses the last applicable rule,
    /// mirroring the TypeScript implementation's override order.
    /// An `Ask` decision is routed through the permission broker when the
    /// caller provided one. Without a broker, Ask remains an error so
    /// non-interactive contexts fail closed instead of silently executing.
    pub async fn check_permission(&self, permission: &str, pattern: &str) -> anyhow::Result<()> {
        use crate::permission::Action;
        if self.permission_rules.is_empty() {
            return Ok(());
        }
        let decision = crate::permission::evaluate(
            permission,
            pattern,
            std::slice::from_ref(&self.permission_rules),
        );
        match decision.action {
            Action::Allow => Ok(()),
            Action::Deny => Err(anyhow::anyhow!(
                "Tool '{}' denied by permission rule (pattern '{}')",
                permission,
                decision.pattern
            )),
            Action::Ask => self.ask_permission(permission, pattern).await,
        }
    }

    async fn ask_permission(&self, permission: &str, pattern: &str) -> anyhow::Result<()> {
        // External plugins get first say via the `permission.ask` hook: they
        // can resolve the request to `allow`/`deny` before it ever reaches
        // the interactive broker. A `status` left at `ask` falls through.
        if let Some(plugin_manager) = &self.plugin_manager {
            let decision = plugin_manager
                .trigger_bridge(
                    "permission.ask",
                    serde_json::json!({
                        "type": permission,
                        "pattern": pattern,
                        "sessionID": self.session_id.to_string(),
                    }),
                    serde_json::json!({ "status": "ask" }),
                )
                .await;
            match decision.get("status").and_then(|s| s.as_str()) {
                Some("allow") => return Ok(()),
                Some("deny") => {
                    return Err(anyhow::anyhow!(
                        "Tool '{}' denied by plugin for pattern '{}'",
                        permission,
                        pattern
                    ))
                }
                _ => {}
            }
        }

        let Some(broker) = &self.permission_broker else {
            return Err(anyhow::anyhow!(
                "Tool '{}' requires explicit user approval for pattern '{}'",
                permission,
                pattern
            ));
        };

        let permission_id = crate::permission::PermissionID::new();
        let mut metadata = HashMap::new();
        metadata.insert("pattern".to_string(), serde_json::json!(pattern));
        let request = crate::permission::PermissionRequest {
            id: permission_id,
            session_id: self.session_id,
            permission: permission.to_string(),
            patterns: vec![pattern.to_string()],
            metadata,
            always: vec![pattern.to_string()],
            tool: None,
        };

        let rx = broker.register(request).await;
        if let Some(bus) = &self.event_bus {
            bus.publish(crate::bus::Event::PermissionAsked(
                crate::bus::event::PermissionAskedEvent {
                    session_id: self.session_id.to_string(),
                    permission_id: permission_id.to_string(),
                    permission_type: permission.to_string(),
                    tool_call_id: None,
                    tool_name: Some(permission.to_string()),
                    metadata: serde_json::json!({
                        "permission": permission,
                        "pattern": pattern,
                    }),
                },
            ));
        }

        match tokio::time::timeout(Duration::from_secs(300), rx).await {
            Ok(Ok(crate::permission::Reply::Once | crate::permission::Reply::Always)) => Ok(()),
            Ok(Ok(crate::permission::Reply::Reject)) => Err(anyhow::anyhow!(
                "Tool '{}' rejected by user for pattern '{}'",
                permission,
                pattern
            )),
            Ok(Err(_)) => Err(anyhow::anyhow!(
                "Permission request for tool '{}' was cancelled",
                permission
            )),
            Err(_) => {
                broker.remove(&permission_id.to_string()).await;
                Err(anyhow::anyhow!(
                    "Permission request for tool '{}' timed out",
                    permission
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<FilePart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    pub id: PartID,
    #[serde(rename = "sessionID")]
    pub session_id: SessionID,
    #[serde(rename = "messageID")]
    pub message_id: MessageID,
    pub mime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub url: String,
}

impl ToolResult {
    pub fn text(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            attachments: None,
            metadata: None,
        }
    }

    pub fn with_metadata(output: impl Into<String>, metadata: serde_json::Value) -> Self {
        Self {
            output: output.into(),
            attachments: None,
            metadata: Some(metadata),
        }
    }

    pub fn with_attachments(output: impl Into<String>, attachments: Vec<FilePart>) -> Self {
        Self {
            output: output.into(),
            attachments: Some(attachments),
            metadata: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{Action, PermissionRule};

    fn ctx(rules: Vec<PermissionRule>) -> ToolContext {
        ToolContext {
            session_id: SessionID::new(),
            working_dir: std::path::PathBuf::from("/tmp"),
            permission_rules: rules,
            event_bus: None,
            permission_broker: None,
            provider: None,
            store: None,
            config: None,
            agent_name: None,
            model_id: None,
            plugin_manager: None,
        }
    }

    #[tokio::test]
    async fn empty_ruleset_allows() {
        assert!(ctx(vec![]).check_permission("bash", "ls").await.is_ok());
    }

    #[tokio::test]
    async fn explicit_deny_blocks() {
        let rule = PermissionRule {
            permission: "bash".to_string(),
            pattern: "rm *".to_string(),
            action: Action::Deny,
        };
        assert!(ctx(vec![rule])
            .check_permission("bash", "rm -rf /")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn explicit_allow_passes() {
        let rule = PermissionRule {
            permission: "bash".to_string(),
            pattern: "ls *".to_string(),
            action: Action::Allow,
        };
        assert!(ctx(vec![rule])
            .check_permission("bash", "ls -la")
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn no_match_with_nonempty_ruleset_asks() {
        // A non-empty ruleset that doesn't match the pattern should fall
        // back to Ask, which surfaces as an error to the tool.
        let rule = PermissionRule {
            permission: "bash".to_string(),
            pattern: "rm *".to_string(),
            action: Action::Deny,
        };
        let err = ctx(vec![rule])
            .check_permission("bash", "ls -la")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("explicit user approval"), "got: {err}");
    }

    #[tokio::test]
    async fn ask_waits_for_permission_reply_when_broker_is_available() {
        let broker = crate::permission::PermissionBroker::new();
        let mut ctx = ctx(vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "git *".to_string(),
            action: Action::Ask,
        }]);
        ctx.permission_broker = Some(broker.clone());

        let session_id = ctx.session_id.to_string();
        let task = tokio::spawn(async move { ctx.check_permission("bash", "git status").await });

        let request_id = loop {
            let pending = broker.pending(Some(&session_id)).await;
            if let Some(request) = pending.first() {
                break request.id.to_string();
            }
            tokio::task::yield_now().await;
        };

        assert!(
            broker
                .reply(&request_id, crate::permission::Reply::Once)
                .await
        );
        assert!(task.await.unwrap().is_ok());
    }

    async fn ctx_with_permission_plugin(status: &str) -> Option<ToolContext> {
        use crate::plugin::bridge;
        let (runtime, _) = bridge::detect_js_runtime()?;
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("plugin.mjs");
        std::fs::write(
            &plugin,
            format!(
                r#"export default async function () {{
                    return {{
                        "permission.ask": async (input, output) => {{
                            output.status = "{status}"
                        }},
                    }}
                }}"#
            ),
        )
        .unwrap();
        let plugin_bridge = bridge::PluginBridge::spawn(
            runtime,
            vec![bridge::PluginToLoad {
                spec: "./plugin.mjs".to_string(),
                entry: format!("file://{}", plugin.to_string_lossy()),
                options: None,
            }],
            bridge::PluginInputData {
                directory: "/tmp".to_string(),
                worktree: "/tmp".to_string(),
                project: serde_json::json!({}),
                server_url: "http://localhost:4096".to_string(),
            },
        )
        .await
        .unwrap();
        let manager = crate::plugin::PluginManager::new();
        manager.set_bridge(std::sync::Arc::new(plugin_bridge));

        // A non-empty ruleset with no matching rule resolves to Ask, which
        // is where the `permission.ask` plugin hook gets consulted.
        let mut ctx = ctx(vec![PermissionRule {
            permission: "bash".to_string(),
            pattern: "git *".to_string(),
            action: Action::Ask,
        }]);
        ctx.plugin_manager = Some(std::sync::Arc::new(manager));
        Some(ctx)
    }

    #[tokio::test]
    async fn permission_ask_plugin_can_allow() {
        let Some(ctx) = ctx_with_permission_plugin("allow").await else {
            eprintln!("skipping: no JS runtime available");
            return;
        };
        // No broker is set; only the plugin's `allow` lets this through.
        assert!(ctx.check_permission("bash", "rm -rf /").await.is_ok());
    }

    #[tokio::test]
    async fn permission_ask_plugin_can_deny() {
        let Some(ctx) = ctx_with_permission_plugin("deny").await else {
            eprintln!("skipping: no JS runtime available");
            return;
        };
        let err = ctx
            .check_permission("bash", "ls")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("denied by plugin"), "got: {err}");
    }
}
