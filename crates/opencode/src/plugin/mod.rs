use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use async_trait::async_trait;

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

pub type HookFn<I, O> = Arc<dyn Fn(I) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<O>> + Send>> + Send + Sync>;

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

    pub async fn register(&mut self, plugin: Arc<dyn Plugin>, config: PluginConfig) -> anyhow::Result<()> {
        let hooks = plugin.initialize(config).await?;
        self.plugins.push(plugin);
        self.hooks.push(hooks);
        Ok(())
    }

    pub async fn trigger_session_create(&self, input: SessionCreateInput) -> anyhow::Result<SessionCreateOutput> {
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

    pub async fn trigger_session_prompt(&self, input: SessionPromptInput) -> anyhow::Result<SessionPromptOutput> {
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

    pub async fn trigger_tool_start(&self, input: ToolStartInput) -> anyhow::Result<ToolStartOutput> {
        let mut output = ToolStartOutput {
            approved: true,
            modified_input: None,
        };
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_tool_start {
                output = hook(input.clone()).await?;
                if !output.approved {
                    break;
                }
            }
        }
        Ok(output)
    }

    pub async fn trigger_tool_complete(&self, input: ToolCompleteInput) -> anyhow::Result<ToolCompleteOutput> {
        let mut output = ToolCompleteOutput {
            modified_output: None,
        };
        for hooks in &self.hooks {
            if let Some(hook) = &hooks.on_tool_complete {
                output = hook(input.clone()).await?;
            }
        }
        Ok(output)
    }

    pub async fn trigger_permission(&self, input: PermissionInput) -> anyhow::Result<PermissionOutput> {
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