use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tracing::{debug, warn};

use crate::config::McpServerConfig;
use crate::mcp::client::McpClient;
use crate::mcp::tool::McpTool;

pub struct McpManager {
    clients: HashMap<String, Arc<McpClient>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub async fn start_server(&mut self, name: &str, config: &McpServerConfig) -> Result<()> {
        if self.clients.contains_key(name) {
            self.remove_client(name);
        }

        let client = if let Some(url) = &config.url {
            McpClient::connect_http(url.clone()).await
        } else if let Some(command) = &config.command {
            let args = config.args.clone().unwrap_or_default();
            let env = config.env.clone().unwrap_or_default();
            McpClient::connect_stdio(command.clone(), args, env).await
        } else {
            anyhow::bail!("MCP server '{}' has no valid configuration", name);
        };

        self.clients.insert(name.to_string(), Arc::new(client?));
        debug!("MCP server '{}' started successfully", name);
        Ok(())
    }

    pub fn stop_server(&mut self, name: &str) -> Result<()> {
        self.remove_client(name);
        debug!("MCP server '{}' stopped", name);
        Ok(())
    }

    pub fn get_client(&self, name: &str) -> Option<&Arc<McpClient>> {
        self.clients.get(name)
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

    fn remove_client(&mut self, name: &str) {
        if let Some(_client) = self.clients.remove(name) {
            debug!("Removed MCP client '{}'", name);
        }
    }
}
