use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
    pub priority: String,
}

#[derive(Debug, Deserialize)]
pub struct TodoWriteParams {
    pub todos: Vec<TodoItem>,
}

pub struct TodoWriteTool;

impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Use this tool to create and manage a structured task list for your current coding session."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string"},
                            "status": {"type": "string", "enum": ["pending", "in_progress", "completed", "cancelled"]},
                            "priority": {"type": "string", "enum": ["high", "medium", "low"]}
                        },
                        "required": ["content", "status", "priority"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: TodoWriteParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid todowrite parameters: {}", e))?;

            let active_count = params
                .todos
                .iter()
                .filter(|t| t.status != "completed")
                .count();

            Ok(ToolResult::with_metadata(
                serde_json::to_string_pretty(&params.todos)?,
                json!({
                    "todos": params.todos,
                    "active_count": active_count,
                }),
            ))
        })
    }
}
