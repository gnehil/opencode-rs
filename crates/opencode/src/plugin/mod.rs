use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub enabled: bool,
    pub options: HashMap<String, serde_json::Value>,
}

pub struct Hooks {
    pub on_command_execute_before:
        Option<HookFn<CommandExecuteBeforeInput, CommandExecuteBeforeOutput>>,
    pub on_session_create: Option<HookFn<SessionCreateInput, SessionCreateOutput>>,
    pub on_session_prompt: Option<HookFn<SessionPromptInput, SessionPromptOutput>>,
    pub on_tool_start: Option<HookFn<ToolStartInput, ToolStartOutput>>,
    pub on_tool_complete: Option<HookFn<ToolCompleteInput, ToolCompleteOutput>>,
    pub on_permission_asked: Option<HookFn<PermissionInput, PermissionOutput>>,
    pub on_message_create: Option<HookFn<MessageCreateInput, MessageCreateOutput>>,
    pub on_provider_request: Option<HookFn<ProviderRequestInput, ProviderRequestOutput>>,
    pub on_provider_response: Option<HookFn<ProviderResponseInput, ProviderResponseOutput>>,
    pub on_config_change: Option<HookFn<ConfigChangeInput, ConfigChangeOutput>>,
    pub on_event: Option<HookFn<EventInput, EventOutput>>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            on_command_execute_before: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_permission_asked: None,
            on_message_create: None,
            on_provider_request: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        }
    }
}

pub type HookFn<I, O> = Arc<
    dyn Fn(I) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<O>> + Send>>
        + Send
        + Sync,
>;

#[derive(Debug, Clone)]
pub struct CommandExecuteBeforeInput {
    pub session_id: String,
    pub command: String,
    pub arguments: Option<String>,
    pub parts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CommandExecuteBeforeOutput {
    pub parts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct SessionCreateInput {
    pub session_id: String,
    pub project_path: String,
    pub cwd: String,
}

#[derive(Debug, Clone)]
pub struct SessionCreateOutput {
    pub session_id: String,
}

#[derive(Debug, Clone)]
pub struct SessionPromptInput {
    pub session_id: String,
    pub message: String,
    pub parts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct SessionPromptOutput {
    pub session_id: String,
    pub response: String,
}

#[derive(Debug, Clone)]
pub struct ToolStartInput {
    pub session_id: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub call_id: String,
}

#[derive(Debug, Clone)]
pub struct ToolStartOutput {
    pub approved: bool,
    pub modified_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ToolCompleteInput {
    pub session_id: String,
    pub tool_name: String,
    pub tool_output: serde_json::Value,
    pub call_id: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ToolCompleteOutput {
    pub modified_output: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct PermissionInput {
    pub session_id: String,
    pub permission_id: String,
    pub permission_type: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct PermissionOutput {
    pub approved: bool,
    pub remember: bool,
}

#[derive(Debug, Clone)]
pub struct MessageCreateInput {
    pub session_id: String,
    pub message_id: String,
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct MessageCreateOutput {
    pub modified_content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderRequestInput {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub request: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ProviderRequestOutput {
    pub modified_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ProviderResponseInput {
    pub session_id: String,
    pub provider: String,
    pub model: String,
    pub response: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ProviderResponseOutput {
    pub modified_response: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ConfigChangeInput {
    pub config_type: String,
    pub old_value: serde_json::Value,
    pub new_value: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ConfigChangeOutput {}

#[derive(Debug, Clone)]
pub struct EventInput {
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct EventOutput {}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks>;

    async fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct PluginManager {
    plugins: Vec<Arc<dyn Plugin>>,
    hooks: Vec<Hooks>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            hooks: Vec::new(),
        }
    }

    pub async fn register_internal_plugins(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for plugin in internal_plugins() {
            let meta = plugin.meta();
            if let Err(error) = self
                .register(
                    plugin,
                    PluginConfig {
                        enabled: true,
                        options: HashMap::new(),
                    },
                )
                .await
            {
                errors.push(format!("{}: {}", meta.id, error));
            }
        }
        errors
    }

    pub async fn register(
        &mut self,
        plugin: Arc<dyn Plugin>,
        config: PluginConfig,
    ) -> anyhow::Result<()> {
        if !config.enabled {
            return Ok(());
        }
        let hooks = plugin.initialize(config).await?;
        self.plugins.push(plugin);
        self.hooks.push(hooks);
        Ok(())
    }

    pub fn attach_event_bus(self: &Arc<Self>, bus: &crate::bus::EventBus) -> crate::bus::HandlerId {
        let manager = Arc::clone(self);
        bus.subscribe("*", move |event| {
            let manager = Arc::clone(&manager);
            let event_type = event.type_name().to_string();
            let fallback_event_type = event_type.clone();
            let payload = serde_json::to_value(event).unwrap_or_else(|_| {
                serde_json::json!({
                    "event_type": fallback_event_type,
                    "id": event.id(),
                })
            });
            Box::pin(async move {
                let _ = manager
                    .trigger_event(EventInput {
                        event_type,
                        payload,
                    })
                    .await;
            })
        })
    }

    pub async fn trigger_session_create(
        &self,
        input: SessionCreateInput,
    ) -> anyhow::Result<SessionCreateOutput> {
        let mut output = SessionCreateOutput {
            session_id: input.session_id.clone(),
        };
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_session_create {
                output = hook(input.clone()).await?;
            }
        }
        Ok(output)
    }

    pub async fn trigger_command_execute_before(
        &self,
        input: CommandExecuteBeforeInput,
        output: CommandExecuteBeforeOutput,
    ) -> anyhow::Result<CommandExecuteBeforeOutput> {
        let mut output = output;
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_command_execute_before {
                output = hook(CommandExecuteBeforeInput {
                    session_id: input.session_id.clone(),
                    command: input.command.clone(),
                    arguments: input.arguments.clone(),
                    parts: output.parts.clone(),
                })
                .await?;
            }
        }
        Ok(output)
    }

    pub async fn trigger_session_prompt(
        &self,
        input: SessionPromptInput,
    ) -> anyhow::Result<SessionPromptOutput> {
        let mut output = SessionPromptOutput {
            session_id: input.session_id.clone(),
            response: String::new(),
        };
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_session_prompt {
                output = hook(input.clone()).await?;
            }
        }
        Ok(output)
    }

    pub async fn trigger_tool_start(
        &self,
        input: ToolStartInput,
    ) -> anyhow::Result<ToolStartOutput> {
        let mut output = ToolStartOutput {
            approved: true,
            modified_input: None,
        };
        let mut current_input = input;
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_tool_start {
                output = hook(current_input.clone()).await?;
                if !output.approved {
                    break;
                }
                if let Some(modified) = output.modified_input.clone() {
                    current_input.tool_input = modified;
                }
            }
        }
        if output.approved {
            output.modified_input = Some(current_input.tool_input);
        }
        Ok(output)
    }

    pub async fn trigger_tool_complete(
        &self,
        input: ToolCompleteInput,
    ) -> anyhow::Result<ToolCompleteOutput> {
        let mut output = ToolCompleteOutput {
            modified_output: None,
        };
        let mut current_input = input;
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_tool_complete {
                output = hook(current_input.clone()).await?;
                if let Some(modified) = output.modified_output.clone() {
                    current_input.tool_output = modified;
                }
            }
        }
        output.modified_output = Some(current_input.tool_output);
        Ok(output)
    }

    pub async fn trigger_permission(
        &self,
        input: PermissionInput,
    ) -> anyhow::Result<PermissionOutput> {
        let mut output = PermissionOutput {
            approved: true,
            remember: false,
        };
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_permission_asked {
                output = hook(input.clone()).await?;
                if !output.approved {
                    break;
                }
            }
        }
        Ok(output)
    }

    pub async fn trigger_config_change(
        &self,
        input: ConfigChangeInput,
    ) -> anyhow::Result<ConfigChangeOutput> {
        let mut output = ConfigChangeOutput {};
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_config_change {
                output = hook(input.clone()).await?;
            }
        }
        Ok(output)
    }

    pub async fn trigger_event(&self, input: EventInput) -> anyhow::Result<EventOutput> {
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_event {
                let _ = hook(input.clone()).await;
            }
        }
        Ok(EventOutput {})
    }

    pub fn list(&self) -> Vec<PluginMeta> {
        self.plugins.iter().map(|p| p.meta()).collect()
    }

    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

mod internal;

pub use internal::*;

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingPlugin {
        event_tx: tokio::sync::mpsc::UnboundedSender<String>,
        config_tx: tokio::sync::mpsc::UnboundedSender<String>,
    }

    #[async_trait::async_trait]
    impl Plugin for RecordingPlugin {
        fn meta(&self) -> PluginMeta {
            PluginMeta {
                id: "recording".to_string(),
                name: "Recording".to_string(),
                version: "0.0.0".to_string(),
                description: None,
                author: None,
            }
        }

        async fn initialize(&self, _config: PluginConfig) -> anyhow::Result<Hooks> {
            let event_tx = self.event_tx.clone();
            let config_tx = self.config_tx.clone();
            Ok(Hooks {
                on_event: Some(Arc::new(move |input: EventInput| {
                    let event_tx = event_tx.clone();
                    Box::pin(async move {
                        let _ = event_tx.send(input.event_type);
                        Ok(EventOutput {})
                    })
                })),
                on_config_change: Some(Arc::new(move |input: ConfigChangeInput| {
                    let config_tx = config_tx.clone();
                    Box::pin(async move {
                        let _ = config_tx.send(input.config_type);
                        Ok(ConfigChangeOutput {})
                    })
                })),
                ..Hooks::default()
            })
        }
    }

    #[tokio::test]
    async fn plugin_manager_fans_out_bus_events_and_config_changes() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (config_tx, mut config_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut manager = PluginManager::new();
        manager
            .register(
                Arc::new(RecordingPlugin {
                    event_tx,
                    config_tx,
                }),
                PluginConfig {
                    enabled: true,
                    options: HashMap::new(),
                },
            )
            .await
            .unwrap();

        let manager = Arc::new(manager);
        let bus = crate::bus::EventBus::new();
        manager.attach_event_bus(&bus);
        bus.publish(crate::bus::Event::session_create("s1"));
        manager
            .trigger_config_change(ConfigChangeInput {
                config_type: "project".to_string(),
                old_value: serde_json::Value::Null,
                new_value: serde_json::json!({"model": "anthropic/claude"}),
            })
            .await
            .unwrap();

        assert_eq!(event_rx.recv().await.unwrap(), "session.create");
        assert_eq!(config_rx.recv().await.unwrap(), "project");
    }

    #[tokio::test]
    async fn disabled_plugins_are_not_registered() {
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (config_tx, _config_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut manager = PluginManager::new();
        manager
            .register(
                Arc::new(RecordingPlugin {
                    event_tx,
                    config_tx,
                }),
                PluginConfig {
                    enabled: false,
                    options: HashMap::new(),
                },
            )
            .await
            .unwrap();

        assert_eq!(manager.hook_count(), 0);
        assert!(manager.list().is_empty());
    }
}
