use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use tracing::{debug, warn};

use crate::config::{Config, McpConfigEntry, McpServerConfig};
use crate::mcp::client::McpClient;
use crate::mcp::tool::{McpRuntimeTool, McpTool};
use crate::tool::Tool;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpServerStatus {
    Connected,
    Disabled,
    Failed { error: String },
}

pub struct McpManager {
    clients: HashMap<String, Arc<McpClient>>,
    configs: HashMap<String, McpServerConfig>,
    status: HashMap<String, McpServerStatus>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            configs: HashMap::new(),
            status: HashMap::new(),
        }
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
                        self.status.insert(
                            name.clone(),
                            McpServerStatus::Failed {
                                error: e.to_string(),
                            },
                        );
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
            McpClient::connect_http(url.clone()).await
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

    fn remove_client(&mut self, name: &str) {
        if let Some(_client) = self.clients.remove(name) {
            debug!("Removed MCP client '{}'", name);
        }
    }
}
