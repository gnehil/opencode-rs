use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;
use crate::agent::get_agent;
use crate::id::SessionID;
use crate::provider::{AnthropicProvider, Provider};
use crate::session::{PromptProcessor, SessionStore};

#[derive(Debug, Deserialize)]
pub struct TaskParams {
    pub description: String,
    pub prompt: String,
    pub subagent_type: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub load_skills: Vec<String>,
    #[serde(default)]
    pub category: Option<String>,
}

pub struct TaskTool;

impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Spawn agent task with category-based or direct agent selection."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "A short (3-5 words) description of the task"
                },
                "prompt": {
                    "type": "string",
                    "description": "The task for the agent to perform"
                },
                "subagent_type": {
                    "type": "string",
                    "description": "The type of specialized agent to use",
                    "enum": ["build", "explore", "general", "plan", "oracle", "librarian", "scout"]
                },
                "task_id": {
                    "type": "string",
                    "description": "Resume a previous task by passing its task_id"
                },
                "command": {
                    "type": "string",
                    "description": "The command that triggered this task"
                },
                "session_id": {
                    "type": "string",
                    "description": "Existing session to continue"
                },
                "load_skills": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Skills to load for this task"
                },
                "category": {
                    "type": "string",
                    "description": "Task category for model selection"
                }
            },
            "required": ["description", "prompt"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: TaskParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid task parameters: {}", e))?;

            let task_id = params
                .task_id
                .unwrap_or_else(|| ulid::Ulid::new().to_string());

            let agent_info = get_agent(&params.subagent_type).or_else(|| {
                if params.subagent_type == "oracle" {
                    Some(crate::agent::AgentInfo {
                        name: "oracle".to_string(),
                        description: Some(
                            "Read-only high-IQ consultant for debugging and architecture"
                                .to_string(),
                        ),
                        mode: crate::agent::AgentMode::Subagent,
                        native: Some(true),
                        hidden: None,
                        top_p: None,
                        temperature: None,
                        color: None,
                        permission: vec![
                            crate::permission::PermissionRule::allow_tool("grep"),
                            crate::permission::PermissionRule::allow_tool("glob"),
                            crate::permission::PermissionRule::allow_tool("read"),
                            crate::permission::PermissionRule::allow_tool("webfetch"),
                            crate::permission::PermissionRule::allow_tool("websearch"),
                        ],
                        model: None,
                        variant: None,
                        prompt: None,
                        options: std::collections::HashMap::new(),
                        steps: None,
                    })
                } else if params.subagent_type == "librarian" {
                    Some(crate::agent::AgentInfo {
                        name: "librarian".to_string(),
                        description: Some(
                            "Multi-repository code search and documentation retrieval".to_string(),
                        ),
                        mode: crate::agent::AgentMode::Subagent,
                        native: Some(true),
                        hidden: None,
                        top_p: None,
                        temperature: None,
                        color: None,
                        permission: vec![
                            crate::permission::PermissionRule::allow_tool("grep"),
                            crate::permission::PermissionRule::allow_tool("glob"),
                            crate::permission::PermissionRule::allow_tool("read"),
                            crate::permission::PermissionRule::allow_tool("webfetch"),
                            crate::permission::PermissionRule::allow_tool("websearch"),
                        ],
                        model: None,
                        variant: None,
                        prompt: None,
                        options: std::collections::HashMap::new(),
                        steps: None,
                    })
                } else {
                    None
                }
            });

            if agent_info.is_none() {
                return Ok(ToolResult::with_metadata(
                    format!(
                        "task_id: {}\n\n<task_result>\nUnknown subagent type: {}\nAvailable types: build, explore, general, plan, oracle, librarian, scout\n</task_result>",
                        task_id,
                        params.subagent_type
                    ),
                    json!({
                        "task_id": task_id,
                        "status": "error",
                        "error": "unknown_subagent_type",
                    }),
                ));
            }

            let agent = agent_info.unwrap();

            let provider: Arc<dyn Provider> =
                Arc::new(AnthropicProvider::from_env().map_err(|e| {
                    anyhow::anyhow!("Failed to initialize Anthropic provider: {}", e)
                })?);

            let data_dir = std::env::var("OPENCODE_DATA_DIR")
                .ok()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    dirs::data_local_dir()
                        .map(|p| p.join("opencode"))
                        .unwrap_or_else(|| std::path::PathBuf::from("/tmp/opencode"))
                });

            let store = Arc::new(SessionStore::new(data_dir).await?);

            let child_session = store
                .create(
                    &format!("{} (@{} subagent)", params.description, agent.name),
                    "default",
                    &ctx.working_dir.clone(),
                )
                .await?;

            let processor = PromptProcessor::new(store.clone(), provider.clone());

            let result_text = match processor
                .process(
                    &SessionID::parse(&child_session.id).unwrap_or_default(),
                    &params.prompt,
                )
                .await
            {
                Ok(text) => text,
                Err(e) => format!("Error: {}", e),
            };

            Ok(ToolResult::with_metadata(
                format!(
                    "task_id: {} (session_id: {})\n\n<task_result>\n{}\n</task_result>",
                    task_id, child_session.id, result_text
                ),
                json!({
                    "task_id": task_id,
                    "session_id": child_session.id,
                    "subagent_type": params.subagent_type,
                    "description": params.description,
                    "status": "completed",
                }),
            ))
        })
    }
}
