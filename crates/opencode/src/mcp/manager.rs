use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use serde::Serialize;
use tracing::{debug, warn};

use crate::config::{Config, McpConfigEntry, McpOAuthConfig, McpServerConfig};
use crate::mcp::client::McpClient;
use crate::mcp::oauth::McpAuthStore;
use crate::mcp::tool::{McpRuntimeTool, McpTool};
use crate::tool::Tool;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpServerStatus {
    Connected,
    Disabled,
    Failed { error: String },
    NeedsAuth,
    NeedsClientRegistration { error: String },
}

pub struct McpManager {
    clients: HashMap<String, Arc<McpClient>>,
    configs: HashMap<String, McpServerConfig>,
    status: HashMap<String, McpServerStatus>,
    auth_store: Option<Arc<McpAuthStore>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            configs: HashMap::new(),
            status: HashMap::new(),
            auth_store: None,
        }
    }

    pub fn with_auth_store(mut self, auth_store: Arc<McpAuthStore>) -> Self {
        self.auth_store = Some(auth_store);
        self
    }

    pub async fn start_configured(&mut self, config: &Config) {
        let Some(mcp) = &config.mcp else {
            return;
        };

        for (name, entry) in mcp {
            match entry {
                McpConfigEntry::Disabled { enabled } if !enabled => {
                    self.remove_client(name);
                    self.status.insert(name.clone(), McpServerStatus::Disabled);
                }
                McpConfigEntry::Disabled { .. } => {
                    self.status.insert(
                        name.clone(),
                        McpServerStatus::Failed {
                            error:
                                "MCP entry only contains enabled=true and no server configuration"
                                    .to_string(),
                        },
                    );
                }
                McpConfigEntry::Full(server) if !server.is_enabled() => {
                    self.configs.insert(name.clone(), server.clone());
                    self.remove_client(name);
                    self.status.insert(name.clone(), McpServerStatus::Disabled);
                }
                McpConfigEntry::Full(server) => {
                    self.configs.insert(name.clone(), server.clone());
                    if let Err(e) = self.start_server(name, server).await {
                        warn!("Failed to start MCP server '{}': {}", name, e);
                        self.remove_client(name);
                        let status = if supports_oauth(server) && is_unauthorized_error(&e) {
                            McpServerStatus::NeedsAuth
                        } else {
                            McpServerStatus::Failed {
                                error: e.to_string(),
                            }
                        };
                        self.status.insert(name.clone(), status);
                    }
                }
            }
        }
    }

    pub async fn start_server(&mut self, name: &str, config: &McpServerConfig) -> Result<()> {
        if self.clients.contains_key(name) {
            self.remove_client(name);
        }
        self.configs.insert(name.to_string(), config.clone());

        let client = if let Some(url) = &config.url {
            let headers = http_headers_for_config(name, config, self.auth_store.clone()).await?;
            McpClient::connect_http_with_headers(url.clone(), headers).await
        } else if let Some((command, args)) = config.command_and_args() {
            let env = config.env.clone().unwrap_or_default();
            McpClient::connect_stdio(command, args, env).await
        } else {
            anyhow::bail!("MCP server '{}' has no valid configuration", name);
        };

        self.clients.insert(name.to_string(), Arc::new(client?));
        self.status
            .insert(name.to_string(), McpServerStatus::Connected);
        debug!("MCP server '{}' started successfully", name);
        Ok(())
    }

    pub async fn connect_server(&mut self, name: &str) -> Result<()> {
        let config =
            self.configs.get(name).cloned().ok_or_else(|| {
                anyhow::anyhow!("MCP server '{}' has no known configuration", name)
            })?;
        self.start_server(name, &config).await
    }

    pub fn stop_server(&mut self, name: &str) -> Result<()> {
        self.remove_client(name);
        self.status
            .insert(name.to_string(), McpServerStatus::Disabled);
        debug!("MCP server '{}' stopped", name);
        Ok(())
    }

    pub fn get_client(&self, name: &str) -> Option<&Arc<McpClient>> {
        self.clients.get(name)
    }

    pub fn status(&self) -> HashMap<String, McpServerStatus> {
        self.status.clone()
    }

    pub async fn list_all_tools(&self) -> Vec<(String, McpTool)> {
        let mut all_tools = Vec::new();

        for (name, client) in &self.clients {
            match client.list_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        all_tools.push((name.clone(), tool));
                    }
                }
                Err(e) => {
                    warn!("Failed to list tools from '{}': {}", name, e);
                }
            }
        }

        all_tools
    }

    pub async fn runtime_tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut result: Vec<Arc<dyn Tool>> = Vec::new();

        for (name, client) in &self.clients {
            match client.list_tools().await {
                Ok(tools) => {
                    for tool in tools {
                        result.push(Arc::new(McpRuntimeTool::new(
                            name.clone(),
                            tool,
                            client.clone(),
                        )));
                    }
                }
                Err(e) => {
                    warn!("Failed to list tools from '{}': {}", name, e);
                }
            }
        }

        result
    }

    pub async fn list_all_resources(&self) -> Vec<serde_json::Value> {
        let mut result = Vec::new();

        for (name, client) in &self.clients {
            match client.list_resources().await {
                Ok(resources) => {
                    for resource in resources {
                        result.push(serde_json::json!({
                            "client": name,
                            "name": resource.name,
                            "uri": resource.uri,
                            "description": resource.description,
                            "mimeType": resource.mime_type,
                        }));
                    }
                }
                Err(e) => {
                    warn!("Failed to list resources from '{}': {}", name, e);
                }
            }
        }

        result
    }

    pub async fn list_all_prompts(&self) -> HashMap<String, serde_json::Value> {
        let mut result = HashMap::new();

        for (name, client) in &self.clients {
            match client.list_prompts().await {
                Ok(prompts) => {
                    for prompt in prompts {
                        let key = format!(
                            "{}:{}",
                            sanitize_mcp_name(name),
                            sanitize_mcp_name(&prompt.name)
                        );
                        let mut value = serde_json::to_value(&prompt).unwrap_or_else(|_| {
                            serde_json::json!({
                                "name": prompt.name,
                                "description": prompt.description,
                                "arguments": prompt.arguments,
                            })
                        });
                        if let serde_json::Value::Object(map) = &mut value {
                            map.insert(
                                "client".to_string(),
                                serde_json::Value::String(name.clone()),
                            );
                        }
                        result.insert(key, value);
                    }
                }
                Err(e) => {
                    warn!("Failed to list prompts from '{}': {}", name, e);
                }
            }
        }

        result
    }

    pub async fn get_prompt(
        &self,
        client_name: &str,
        name: &str,
        args: Option<HashMap<String, String>>,
    ) -> Result<Option<rmcp::model::GetPromptResult>> {
        let Some(client) = self.clients.get(client_name) else {
            warn!("client not found for get_prompt: {}", client_name);
            return Ok(None);
        };

        match client.get_prompt(name, args).await {
            Ok(prompt) => Ok(Some(prompt)),
            Err(e) => {
                warn!(
                    "Failed to get prompt '{}' from '{}': {}",
                    name, client_name, e
                );
                Ok(None)
            }
        }
    }

    fn remove_client(&mut self, name: &str) {
        if let Some(_client) = self.clients.remove(name) {
            debug!("Removed MCP client '{}'", name);
        }
    }
}

async fn http_headers_for_config(
    name: &str,
    config: &McpServerConfig,
    auth_store: Option<Arc<McpAuthStore>>,
) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if let Some(config_headers) = &config.headers {
        for (key, value) in config_headers {
            headers.insert(
                HeaderName::from_bytes(key.as_bytes())
                    .with_context(|| format!("Invalid MCP HTTP header name: {key}"))?,
                HeaderValue::from_str(value)
                    .with_context(|| format!("Invalid MCP HTTP header value for {key}"))?,
            );
        }
    }
    let Some(store) = auth_store else {
        return Ok(headers);
    };
    let Some(url) = config.url.as_deref() else {
        return Ok(headers);
    };
    store.load().await?;
    if let Some(tokens) = store
        .get_for_url(name, url)
        .await
        .and_then(|entry| entry.tokens)
    {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", tokens.access_token))
                .context("Invalid MCP OAuth access token")?,
        );
    }
    Ok(headers)
}

fn supports_oauth(config: &McpServerConfig) -> bool {
    config.url.is_some() && !matches!(config.oauth, Some(McpOAuthConfig::Enabled(false)))
}

fn is_unauthorized_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("401") || message.contains("unauthorized")
    })
}

fn sanitize_mcp_name(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{McpAuthStore, OAuthTokens};
    use serde_json::json;

    #[tokio::test]
    async fn http_headers_include_config_headers_and_saved_bearer_token() {
        let tmp = tempfile::tempdir().unwrap();
        let auth_store = Arc::new(McpAuthStore::new(tmp.path().join("data")));
        auth_store
            .update_tokens(
                "remote",
                OAuthTokens {
                    access_token: "token-123".to_string(),
                    refresh_token: None,
                    expires_at: None,
                    scope: None,
                },
                Some("https://example.com/mcp"),
            )
            .await
            .unwrap();
        let config: McpServerConfig = serde_json::from_value(json!({
            "type": "remote",
            "url": "https://example.com/mcp",
            "headers": {
                "x-api-key": "key-123"
            }
        }))
        .unwrap();

        let headers = http_headers_for_config("remote", &config, Some(auth_store))
            .await
            .unwrap();

        assert_eq!(
            headers.get("x-api-key").unwrap().to_str().unwrap(),
            "key-123"
        );
        assert_eq!(
            headers
                .get(reqwest::header::AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer token-123"
        );
    }

    #[tokio::test]
    async fn prompt_api_handles_empty_manager_and_unknown_client() {
        let manager = McpManager::new();

        assert!(manager.list_all_prompts().await.is_empty());
        assert!(manager
            .get_prompt(
                "missing",
                "review",
                Some(HashMap::from([("topic".to_string(), "auth".to_string(),)]))
            )
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn sanitize_mcp_name_matches_upstream_mcp_prompt_keys() {
        assert_eq!(sanitize_mcp_name("design tools"), "design_tools");
        assert_eq!(sanitize_mcp_name("repo:search"), "repo_search");
        assert_eq!(sanitize_mcp_name("ok-name_1"), "ok-name_1");
    }

    #[tokio::test]
    async fn remote_oauth_unauthorized_status_maps_to_needs_auth() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/mcp",
            axum::routing::get(|| async { axum::http::StatusCode::UNAUTHORIZED }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let config: Config = serde_json::from_value(json!({
            "mcp": {
                "remote": {
                    "type": "remote",
                    "url": format!("http://{addr}/mcp")
                }
            }
        }))
        .unwrap();

        let mut manager = McpManager::new();
        manager.start_configured(&config).await;

        assert!(matches!(
            manager.status().get("remote"),
            Some(McpServerStatus::NeedsAuth)
        ));
        server.abort();
    }
}
