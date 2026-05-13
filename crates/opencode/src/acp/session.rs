use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::acp::types::{ACPSessionState, McpServer, McpServerConfig, ModelSelection};
use crate::id::SessionID;
use crate::session::SessionStore;
use crate::storage::SessionRow;
use anyhow::Result;

pub struct ACPSessionManager {
    sessions: Arc<RwLock<HashMap<String, ACPSessionState>>>,
    store: Arc<SessionStore>,
}

impl ACPSessionManager {
    pub fn new(store: Arc<SessionStore>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            store,
        }
    }

    pub async fn try_get(&self, session_id: &str) -> Option<ACPSessionState> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    pub async fn create(
        &self,
        cwd: String,
        mcp_servers: Vec<McpServer>,
        model: Option<ModelSelection>,
    ) -> Result<ACPSessionState> {
        let directory = std::path::PathBuf::from(&cwd);
        let title = format!("ACP Session: {}", cwd);
        let session_row = self.store.create(&title, "acp", &directory).await?;
        let session_id = session_row.id;

        let mcp_configs = mcp_servers
            .iter()
            .map(|s| match s {
                McpServer::Sse { name, url, headers } => McpServerConfig::Remote {
                    name: name.clone(),
                    url: url.clone(),
                    headers: headers
                        .iter()
                        .map(|h| (h.name.clone(), h.value.clone()))
                        .collect(),
                },
                McpServer::Stdio {
                    name,
                    command,
                    args,
                    env,
                } => McpServerConfig::Local {
                    name: name.clone(),
                    command: command.clone(),
                    args: args.clone(),
                    env: env
                        .iter()
                        .map(|e| (e.name.clone(), e.value.clone()))
                        .collect(),
                },
            })
            .collect();

        let state = ACPSessionState {
            id: session_id.clone(),
            cwd,
            mcp_servers: mcp_configs,
            created_at: chrono::Utc::now(),
            model,
            variant: None,
            mode_id: None,
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id, state.clone());

        Ok(state)
    }

    pub async fn load(
        &self,
        session_id: &str,
        cwd: String,
        mcp_servers: Vec<McpServer>,
        model: Option<ModelSelection>,
    ) -> Result<ACPSessionState> {
        let parsed_id = SessionID::parse(session_id)?;
        let _session_row = self
            .store
            .get(&parsed_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

        let mcp_configs = mcp_servers
            .iter()
            .map(|s| match s {
                McpServer::Sse { name, url, headers } => McpServerConfig::Remote {
                    name: name.clone(),
                    url: url.clone(),
                    headers: headers
                        .iter()
                        .map(|h| (h.name.clone(), h.value.clone()))
                        .collect(),
                },
                McpServer::Stdio {
                    name,
                    command,
                    args,
                    env,
                } => McpServerConfig::Local {
                    name: name.clone(),
                    command: command.clone(),
                    args: args.clone(),
                    env: env
                        .iter()
                        .map(|e| (e.name.clone(), e.value.clone()))
                        .collect(),
                },
            })
            .collect();

        let state = ACPSessionState {
            id: session_id.to_string(),
            cwd,
            mcp_servers: mcp_configs,
            created_at: chrono::Utc::now(),
            model,
            variant: None,
            mode_id: None,
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.to_string(), state.clone());

        Ok(state)
    }

    pub async fn get(&self, session_id: &str) -> Result<ACPSessionState> {
        let sessions = self.sessions.read().await;
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))
    }

    pub async fn get_model(&self, session_id: &str) -> Result<Option<ModelSelection>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(session_id).and_then(|s| s.model.clone()))
    }

    pub async fn set_model(&self, session_id: &str, model: Option<ModelSelection>) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get_mut(session_id) {
            state.model = model;
        }
        Ok(())
    }

    pub async fn get_variant(&self, session_id: &str) -> Result<Option<String>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(session_id).and_then(|s| s.variant.clone()))
    }

    pub async fn set_variant(&self, session_id: &str, variant: Option<String>) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get_mut(session_id) {
            state.variant = variant;
        }
        Ok(())
    }

    pub async fn set_mode(&self, session_id: &str, mode_id: String) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(state) = sessions.get_mut(session_id) {
            state.mode_id = Some(mode_id);
        }
        Ok(())
    }

    pub async fn remove(&self, session_id: &str) -> Option<ACPSessionState> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id)
    }

    pub async fn list(&self, cwd: Option<&str>) -> Result<Vec<SessionRow>> {
        self.store.list(None).await
    }
}
