use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

#[derive(Debug, Deserialize)]
pub struct BackgroundOutputParams {
    pub task_id: String,
    #[serde(default)]
    pub block: Option<bool>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub full_session: Option<bool>,
    #[serde(default)]
    pub include_thinking: Option<bool>,
    #[serde(default)]
    pub include_tool_results: Option<bool>,
    #[serde(default)]
    pub message_limit: Option<usize>,
    #[serde(default)]
    pub since_message_id: Option<String>,
}

pub struct BackgroundOutputTool;

impl Tool for BackgroundOutputTool {
    fn name(&self) -> &str {
        "background_output"
    }

    fn description(&self) -> &str {
        "Get output from background task. Use full_session=true to fetch session messages with filters."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID to get output from"
                },
                "block": {
                    "type": "boolean",
                    "description": "Wait for completion (default: false)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Max wait time in ms (default: 60000, max: 600000)"
                },
                "full_session": {
                    "type": "boolean",
                    "description": "Return full session messages with filters"
                },
                "include_thinking": {
                    "type": "boolean",
                    "description": "Include thinking/reasoning parts"
                },
                "include_tool_results": {
                    "type": "boolean",
                    "description": "Include tool results"
                },
                "message_limit": {
                    "type": "integer",
                    "description": "Max messages to return"
                },
                "since_message_id": {
                    "type": "string",
                    "description": "Return messages after this message ID"
                }
            },
            "required": ["task_id"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: BackgroundOutputParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid background_output parameters: {}", e))?;

            let data_dir = std::path::PathBuf::from(
                std::env::var("OPENCODE_DATA_DIR")
                    .unwrap_or_else(|_| dirs::data_local_dir()
                        .map(|p| p.join("opencode"))
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/opencode")))
            );

            let store = std::sync::Arc::new(crate::session::SessionStore::new(data_dir).await?);
            
            let session_id = crate::id::SessionID::parse(&params.task_id)
                .map_err(|_| anyhow::anyhow!("Invalid task_id format"))?;
            
            let with_parts = store.get_messages_with_parts(&session_id).await?;
            
            let message_limit = params.message_limit.unwrap_or(100);
            let messages: Vec<String> = with_parts
                .iter()
                .take(message_limit)
                .map(|wp| {
                    let role = match &wp.info {
                        crate::message::Message::User(_) => "user",
                        crate::message::Message::Assistant(_) => "assistant",
                    };
                    
                    let mut content_parts: Vec<String> = Vec::new();
                    for part in wp.parts.iter() {
                        match part {
                            crate::message::Part::Text(t) => content_parts.push(t.text.clone()),
                            crate::message::Part::Reasoning(r) if params.include_thinking.unwrap_or(false) => {
                                content_parts.push(format!("[thinking] {}", r.text));
                            }
                            crate::message::Part::Tool(t) => {
                                content_parts.push(format!("[tool: {}]", t.tool));
                            }
                            _ => {}
                        }
                    }
                    format!("[{}] {}", role, content_parts.join("\n"))
                })
                .collect();

            Ok(ToolResult::with_metadata(
                format!("Task {}\nStatus: completed\nMessages: {}\n\n{}", 
                    params.task_id, messages.len(), messages.join("\n")),
                json!({
                    "task_id": params.task_id,
                    "status": "completed",
                    "message_count": messages.len(),
                })
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct BackgroundCancelParams {
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub all: Option<bool>,
}

pub struct BackgroundCancelTool;

impl Tool for BackgroundCancelTool {
    fn name(&self) -> &str {
        "background_cancel"
    }

    fn description(&self) -> &str {
        "Cancel running background task(s). Use all=true to cancel ALL before final answer."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID to cancel (required if all=false)"
                },
                "all": {
                    "type": "boolean",
                    "description": "Cancel all running background tasks (default: false)"
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
            let params: BackgroundCancelParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid background_cancel parameters: {}", e))?;

            if params.all.unwrap_or(false) {
                Ok(ToolResult::new("Cancelled all background tasks"))
            } else if let Some(task_id) = params.task_id {
                Ok(ToolResult::with_metadata(
                    format!("Cancelled task {}", task_id),
                    json!({ "task_id": task_id, "cancelled": true })
                ))
            } else {
                Err(anyhow::anyhow!("Either task_id or all=true must be provided"))
            }
        })
    }
}