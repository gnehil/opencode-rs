use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use super::context::ToolContext;
use itertools::Itertools;
use super::r#trait::Tool;
use crate::session::SessionStore;

#[derive(Debug, Deserialize)]
pub struct SessionListParams {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub from_date: Option<String>,
    #[serde(default)]
    pub to_date: Option<String>,
}

pub struct SessionListTool;

impl Tool for SessionListTool {
    fn name(&self) -> &str {
        "session_list"
    }

    fn description(&self) -> &str {
        "List all OpenCode sessions with optional filtering."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of sessions to return"
                },
                "from_date": {
                    "type": "string",
                    "description": "Filter sessions from this date (ISO 8601)"
                },
                "to_date": {
                    "type": "string",
                    "description": "Filter sessions until this date (ISO 8601)"
                }
            }
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: SessionListParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid session list parameters: {}", e))?;

            let data_dir = std::path::PathBuf::from(
                std::env::var("OPENCODE_DATA_DIR")
                    .unwrap_or_else(|_| dirs::data_local_dir()
                        .map(|p| p.join("opencode"))
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/opencode")))
            );

            let store = Arc::new(SessionStore::new(data_dir).await?);
            let sessions = store.list(None).await?;
            
            let limit = params.limit.unwrap_or(50);
            let session_list: Vec<String> = sessions
                .iter()
                .take(limit)
                .map(|s| format!("{} | {} | {}", s.id, s.title, s.time_created))
                .collect();

            Ok(ToolResult::with_metadata(
                session_list.join("\n"),
                json!({ "count": sessions.len(), "limit": limit })
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct SessionInfoParams {
    pub session_id: String,
}

pub struct SessionInfoTool;

impl Tool for SessionInfoTool {
    fn name(&self) -> &str {
        "session_info"
    }

    fn description(&self) -> &str {
        "Get metadata and statistics about an OpenCode session."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID to inspect"
                }
            },
            "required": ["session_id"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: SessionInfoParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid session info parameters: {}", e))?;

            let data_dir = std::path::PathBuf::from(
                std::env::var("OPENCODE_DATA_DIR")
                    .unwrap_or_else(|_| dirs::data_local_dir()
                        .map(|p| p.join("opencode"))
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/opencode")))
            );

            let store = Arc::new(SessionStore::new(data_dir).await?);
            let session_id = crate::id::SessionID::parse(&params.session_id)
                .map_err(|e| anyhow::anyhow!("Invalid session ID: {}", e))?;
            
            let session = store.get(&session_id).await?
                .ok_or_else(|| anyhow::anyhow!("Session not found: {}", params.session_id))?;

            Ok(ToolResult::with_metadata(
                format!("Session: {}\nTitle: {}\nCreated: {}\nMessages: TBD",
                    session.id, session.title, session.time_created),
                json!({
                    "session_id": session.id,
                    "title": session.title,
                    "time_created": session.time_created,
                    "time_updated": session.time_updated,
                    "agent": session.agent,
                    "model": session.model,
                })
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct SessionReadParams {
    pub session_id: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_todos: Option<bool>,
    #[serde(default)]
    pub include_transcript: Option<bool>,
}

pub struct SessionReadTool;

impl Tool for SessionReadTool {
    fn name(&self) -> &str {
        "session_read"
    }

    fn description(&self) -> &str {
        "Read messages and history from an OpenCode session."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID to read"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of messages to return"
                },
                "include_todos": {
                    "type": "boolean",
                    "description": "Include todo list if available"
                },
                "include_transcript": {
                    "type": "boolean",
                    "description": "Include transcript log if available"
                }
            },
            "required": ["session_id"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: SessionReadParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid session read parameters: {}", e))?;

            let data_dir = std::path::PathBuf::from(
                std::env::var("OPENCODE_DATA_DIR")
                    .unwrap_or_else(|_| dirs::data_local_dir()
                        .map(|p| p.join("opencode"))
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/opencode")))
            );

            let store = Arc::new(SessionStore::new(data_dir).await?);
            let session_id = crate::id::SessionID::parse(&params.session_id)
                .map_err(|e| anyhow::anyhow!("Invalid session ID: {}", e))?;
            
            let with_parts = store.get_messages_with_parts(&session_id).await?;
            
            let limit = params.limit.unwrap_or(100);
            let messages: Vec<String> = with_parts
                .iter()
                .take(limit)
                .map(|wp| {
                    let role = match &wp.info {
                        crate::message::Message::User(_) => "user",
                        crate::message::Message::Assistant(_) => "assistant",
                    };
                    let content = wp.parts.iter()
                        .filter_map(|p| match p {
                            crate::message::Part::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .join("\n");
                    format!("[{}] {}", role, content)
                })
                .collect();

            Ok(ToolResult::with_metadata(
                messages.join("\n---\n"),
                json!({ "message_count": with_parts.len(), "limit": limit })
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct SessionSearchParams {
    pub query: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub struct SessionSearchTool;

impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search for content within OpenCode session messages."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "session_id": {
                    "type": "string",
                    "description": "Search within specific session only"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Case-sensitive search"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results"
                }
            },
            "required": ["query"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: SessionSearchParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid session search parameters: {}", e))?;

            let data_dir = std::path::PathBuf::from(
                std::env::var("OPENCODE_DATA_DIR")
                    .unwrap_or_else(|_| dirs::data_local_dir()
                        .map(|p| p.join("opencode"))
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/opencode")))
            );

            let store = Arc::new(SessionStore::new(data_dir).await?);
            let sessions = store.list(None).await?;
            
            let query_lower = params.query.to_lowercase();
            let limit = params.limit.unwrap_or(20);
            let mut matches: Vec<String> = Vec::new();

            for session in sessions.iter() {
                if let Some(sid) = &params.session_id {
                    if session.id != *sid {
                        continue;
                    }
                }

                let session_id = crate::id::SessionID::parse(&session.id)
                    .map_err(|e| anyhow::anyhow!("Invalid session ID: {}", e))?;
                
                let with_parts = store.get_messages_with_parts(&session_id).await?;
                
                for wp in with_parts.iter() {
                    for part in wp.parts.iter() {
                        if let crate::message::Part::Text(t) = part {
                            let text = if params.case_sensitive.unwrap_or(false) {
                                t.text.clone()
                            } else {
                                t.text.to_lowercase()
                            };
                            
                            if text.contains(&query_lower) {
                                matches.push(format!(
                                    "[ses_{}] {}...",
                                    session.id.chars().take(8).collect::<String>(),
                                    t.text.chars().take(100).collect::<String>()
                                ));
                                
                                if matches.len() >= limit {
                                    return Ok(ToolResult::with_metadata(
                                        matches.join("\n"),
                                        json!({ "count": matches.len(), "limit": limit })
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            Ok(ToolResult::with_metadata(
                matches.join("\n"),
                json!({ "count": matches.len(), "limit": limit })
            ))
        })
    }
}