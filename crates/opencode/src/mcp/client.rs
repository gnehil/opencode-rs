use std::collections::HashMap;
use std::process::Stdio;

use anyhow::{Context, Result};
use reqwest::header::HeaderMap;
use rmcp::handler::client::ClientHandler;
use rmcp::model::{CallToolRequestParam, ClientInfo, ReadResourceRequestParam, ServerInfo};
use rmcp::service::{Peer, RoleClient, ServiceExt};
use rmcp::transport::sse::SseTransport;
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::mcp::result::McpToolResult;
use crate::mcp::tool::McpTool;

#[derive(Clone)]
struct McpClientHandler {
    peer: Option<Peer<RoleClient>>,
    client_info: ClientInfo,
}

impl McpClientHandler {
    fn new(client_info: ClientInfo) -> Self {
        Self {
            peer: None,
            client_info,
        }
    }
}

impl ClientHandler for McpClientHandler {
    fn get_peer(&self) -> Option<Peer<RoleClient>> {
        self.peer.clone()
    }

    fn set_peer(&mut self, peer: Peer<RoleClient>) {
        self.peer = Some(peer);
    }

    fn get_info(&self) -> ClientInfo {
        self.client_info.clone()
    }
}

pub struct McpClient {
    peer: Peer<RoleClient>,
    cancel_token: CancellationToken,
    server_info: ServerInfo,
}

impl McpClient {
    pub async fn connect_stdio(
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<Self> {
        debug!("Connecting to MCP server via stdio: {} {:?}", command, args);

        let mut cmd = Command::new(&command);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let current_env: HashMap<String, String> = std::env::vars().collect();
        let mut final_env = current_env;
        final_env.extend(env);
        cmd.envs(&final_env);

        let transport =
            TokioChildProcess::new(&mut cmd).context("Failed to spawn MCP child process")?;

        let client_info = ClientInfo {
            protocol_version: Default::default(),
            capabilities: Default::default(),
            client_info: rmcp::model::Implementation {
                name: "opencode".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let handler = McpClientHandler::new(client_info);
        let cancel_token = CancellationToken::new();

        let running_service = handler
            .serve_with_ct(transport, cancel_token.clone())
            .await
            .context("Failed to start MCP client via stdio")?;

        let server_info = running_service.peer().peer_info().clone();
        let peer = running_service.peer().clone();

        tokio::spawn(async move {
            let _ = running_service.waiting().await;
        });

        debug!(
            "Connected to MCP server: {} v{}",
            server_info.server_info.name, server_info.server_info.version
        );

        Ok(Self {
            peer,
            cancel_token,
            server_info,
        })
    }

    pub async fn connect_http(url: String) -> Result<Self> {
        Self::connect_http_with_headers(url, HeaderMap::new()).await
    }

    pub async fn connect_http_with_headers(url: String, headers: HeaderMap) -> Result<Self> {
        debug!("Connecting to MCP server via HTTP: {}", url);

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("Failed to build MCP HTTP client")?;
        let transport = SseTransport::start_with_client(&url, client)
            .await
            .context("Failed to connect to MCP server via SSE")?;

        let client_info = ClientInfo {
            protocol_version: Default::default(),
            capabilities: Default::default(),
            client_info: rmcp::model::Implementation {
                name: "opencode".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let handler = McpClientHandler::new(client_info);
        let cancel_token = CancellationToken::new();

        let running_service = handler
            .serve_with_ct(transport, cancel_token.clone())
            .await
            .context("Failed to start MCP client via HTTP")?;

        let server_info = running_service.peer().peer_info().clone();
        let peer = running_service.peer().clone();

        tokio::spawn(async move {
            let _ = running_service.waiting().await;
        });

        debug!(
            "Connected to MCP server: {} v{}",
            server_info.server_info.name, server_info.server_info.version
        );

        Ok(Self {
            peer,
            cancel_token,
            server_info,
        })
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let tools = self
            .peer
            .list_all_tools()
            .await
            .context("Failed to list tools from MCP server")?;

        Ok(tools.iter().map(McpTool::from_rmcp).collect())
    }

    pub async fn call_tool(&self, name: &str, args: serde_json::Value) -> Result<McpToolResult> {
        let arguments = if args.is_null() || args.as_object().map_or(true, |m| m.is_empty()) {
            None
        } else {
            let obj = args
                .as_object()
                .context("Tool arguments must be a JSON object")?
                .clone();
            Some(obj)
        };

        let params = CallToolRequestParam {
            name: std::borrow::Cow::Owned(name.to_string()),
            arguments,
        };

        let result = self
            .peer
            .call_tool(params)
            .await
            .with_context(|| format!("Failed to call tool '{}' on MCP server", name))?;

        Ok(McpToolResult::from_rmcp(&result))
    }

    pub async fn list_resources(&self) -> Result<Vec<rmcp::model::Resource>> {
        self.peer
            .list_all_resources()
            .await
            .context("Failed to list resources from MCP server")
    }

    pub async fn read_resource(&self, uri: &str) -> Result<String> {
        use rmcp::model::ResourceContents;
        let result = self
            .peer
            .read_resource(ReadResourceRequestParam {
                uri: uri.to_string(),
            })
            .await
            .with_context(|| format!("Failed to read resource '{}' from MCP server", uri))?;

        let mut parts = Vec::new();
        for content in &result.contents {
            match content {
                ResourceContents::TextResourceContents { text, .. } => {
                    parts.push(text.clone());
                }
                ResourceContents::BlobResourceContents { blob, .. } => {
                    if let Ok(decoded) =
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, blob)
                    {
                        if let Ok(t) = String::from_utf8(decoded) {
                            parts.push(t);
                        } else {
                            parts.push(format!("<binary data: {} bytes>", blob.len()));
                        }
                    }
                }
            }
        }

        Ok(parts.join("\n"))
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    pub fn disconnect(self) {
        self.cancel_token.cancel();
    }
}
