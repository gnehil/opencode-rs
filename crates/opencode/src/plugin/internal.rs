use std::sync::Arc;
use async_trait::async_trait;
use super::{Plugin, PluginMeta, PluginConfig, Hooks, HookFn, PermissionInput, PermissionOutput, ProviderRequestInput, ProviderRequestOutput};

pub struct CodexAuthPlugin;

impl CodexAuthPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for CodexAuthPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "codex-auth".to_string(),
            name: "Codex Auth Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Handles authentication for OpenAI Codex provider".to_string()),
            author: Some("OpenCode".to_string()),
        }
    }

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks> {
        Ok(Hooks {
            on_provider_request: Some(Arc::new(|input: ProviderRequestInput| {
                Box::pin(async move {
                    if input.provider == "openai" || input.provider == "codex" {
                        let mut modified = input.request.clone();
                        if let Some(obj) = modified.as_object_mut() {
                            if !obj.contains_key("api_key") {
                                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                                    obj.insert("api_key".to_string(), serde_json::json!(key));
                                }
                            }
                        }
                        Ok(ProviderRequestOutput {
                            modified_request: Some(modified),
                        })
                    } else {
                        Ok(ProviderRequestOutput {
                            modified_request: None,
                        })
                    }
                })
            })),
            on_permission_asked: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_message_create: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        })
    }
}

pub struct CopilotAuthPlugin;

impl CopilotAuthPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for CopilotAuthPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "copilot-auth".to_string(),
            name: "GitHub Copilot Auth Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Handles authentication for GitHub Copilot provider".to_string()),
            author: Some("OpenCode".to_string()),
        }
    }

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks> {
        Ok(Hooks {
            on_provider_request: Some(Arc::new(|input: ProviderRequestInput| {
                Box::pin(async move {
                    if input.provider == "copilot" || input.provider == "github-copilot" {
                        let mut modified = input.request.clone();
                        if let Ok(token) = std::env::var("GITHUB_COPILOT_TOKEN") {
                            if let Some(obj) = modified.as_object_mut() {
                                obj.insert("authorization".to_string(), serde_json::json!(format!("Bearer {}", token)));
                            }
                        }
                        Ok(ProviderRequestOutput {
                            modified_request: Some(modified),
                        })
                    } else {
                        Ok(ProviderRequestOutput {
                            modified_request: None,
                        })
                    }
                })
            })),
            on_permission_asked: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_message_create: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        })
    }
}

pub struct GitlabAuthPlugin;

impl GitlabAuthPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for GitlabAuthPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "gitlab-auth".to_string(),
            name: "GitLab Auth Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Handles authentication for GitLab provider".to_string()),
            author: Some("OpenCode".to_string()),
        }
    }

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks> {
        Ok(Hooks {
            on_provider_request: Some(Arc::new(|input: ProviderRequestInput| {
                Box::pin(async move {
                    if input.provider == "gitlab" {
                        let mut modified = input.request.clone();
                        if let Ok(token) = std::env::var("GITLAB_TOKEN") {
                            if let Some(obj) = modified.as_object_mut() {
                                obj.insert("private_token".to_string(), serde_json::json!(token));
                            }
                        }
                        Ok(ProviderRequestOutput {
                            modified_request: Some(modified),
                        })
                    } else {
                        Ok(ProviderRequestOutput {
                            modified_request: None,
                        })
                    }
                })
            })),
            on_permission_asked: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_message_create: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        })
    }
}

pub struct PoeAuthPlugin;

impl PoeAuthPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for PoeAuthPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "poe-auth".to_string(),
            name: "Poe Auth Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Handles authentication for Poe provider".to_string()),
            author: Some("OpenCode".to_string()),
        }
    }

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks> {
        Ok(Hooks {
            on_provider_request: Some(Arc::new(|input: ProviderRequestInput| {
                Box::pin(async move {
                    if input.provider == "poe" {
                        let mut modified = input.request.clone();
                        if let Ok(key) = std::env::var("POE_API_KEY") {
                            if let Some(obj) = modified.as_object_mut() {
                                obj.insert("api_key".to_string(), serde_json::json!(key));
                            }
                        }
                        Ok(ProviderRequestOutput {
                            modified_request: Some(modified),
                        })
                    } else {
                        Ok(ProviderRequestOutput {
                            modified_request: None,
                        })
                    }
                })
            })),
            on_permission_asked: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_message_create: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        })
    }
}

pub struct CloudflareWorkersAuthPlugin;

impl CloudflareWorkersAuthPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for CloudflareWorkersAuthPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "cloudflare-workers-auth".to_string(),
            name: "Cloudflare Workers Auth Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Handles authentication for Cloudflare Workers".to_string()),
            author: Some("OpenCode".to_string()),
        }
    }

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks> {
        Ok(Hooks {
            on_provider_request: Some(Arc::new(|input: ProviderRequestInput| {
                Box::pin(async move {
                    if input.provider == "cloudflare-workers" {
                        let mut modified = input.request.clone();
                        if let Ok(token) = std::env::var("CF_API_TOKEN") {
                            if let Some(obj) = modified.as_object_mut() {
                                obj.insert("authorization".to_string(), serde_json::json!(format!("Bearer {}", token)));
                            }
                        }
                        Ok(ProviderRequestOutput {
                            modified_request: Some(modified),
                        })
                    } else {
                        Ok(ProviderRequestOutput {
                            modified_request: None,
                        })
                    }
                })
            })),
            on_permission_asked: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_message_create: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        })
    }
}

pub struct CloudflareAIGatewayAuthPlugin;

impl CloudflareAIGatewayAuthPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for CloudflareAIGatewayAuthPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "cloudflare-ai-gateway-auth".to_string(),
            name: "Cloudflare AI Gateway Auth Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Handles authentication for Cloudflare AI Gateway".to_string()),
            author: Some("OpenCode".to_string()),
        }
    }

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks> {
        Ok(Hooks {
            on_provider_request: Some(Arc::new(|input: ProviderRequestInput| {
                Box::pin(async move {
                    if input.provider == "cloudflare-ai-gateway" {
                        let mut modified = input.request.clone();
                        if let Ok(token) = std::env::var("CF_AI_GATEWAY_TOKEN") {
                            if let Some(obj) = modified.as_object_mut() {
                                obj.insert("cf-aigateway-token".to_string(), serde_json::json!(token));
                            }
                        }
                        Ok(ProviderRequestOutput {
                            modified_request: Some(modified),
                        })
                    } else {
                        Ok(ProviderRequestOutput {
                            modified_request: None,
                        })
                    }
                })
            })),
            on_permission_asked: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_message_create: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        })
    }
}

pub struct AzureAuthPlugin;

impl AzureAuthPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for AzureAuthPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            id: "azure-auth".to_string(),
            name: "Azure Auth Plugin".to_string(),
            version: "1.0.0".to_string(),
            description: Some("Handles authentication for Azure OpenAI provider".to_string()),
            author: Some("OpenCode".to_string()),
        }
    }

    async fn initialize(&self, config: PluginConfig) -> anyhow::Result<Hooks> {
        Ok(Hooks {
            on_provider_request: Some(Arc::new(|input: ProviderRequestInput| {
                Box::pin(async move {
                    if input.provider == "azure" {
                        let mut modified = input.request.clone();
                        if let Ok(key) = std::env::var("AZURE_OPENAI_API_KEY") {
                            if let Some(obj) = modified.as_object_mut() {
                                obj.insert("api_key".to_string(), serde_json::json!(key));
                            }
                        }
                        if let Ok(endpoint) = std::env::var("AZURE_OPENAI_ENDPOINT") {
                            if let Some(obj) = modified.as_object_mut() {
                                obj.insert("endpoint".to_string(), serde_json::json!(endpoint));
                            }
                        }
                        Ok(ProviderRequestOutput {
                            modified_request: Some(modified),
                        })
                    } else {
                        Ok(ProviderRequestOutput {
                            modified_request: None,
                        })
                    }
                })
            })),
            on_permission_asked: None,
            on_session_create: None,
            on_session_prompt: None,
            on_tool_start: None,
            on_tool_complete: None,
            on_message_create: None,
            on_provider_response: None,
            on_config_change: None,
            on_event: None,
        })
    }
}

pub fn internal_plugins() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(CodexAuthPlugin::new()),
        Arc::new(CopilotAuthPlugin::new()),
        Arc::new(GitlabAuthPlugin::new()),
        Arc::new(PoeAuthPlugin::new()),
        Arc::new(CloudflareWorkersAuthPlugin::new()),
        Arc::new(CloudflareAIGatewayAuthPlugin::new()),
        Arc::new(AzureAuthPlugin::new()),
    ]
}