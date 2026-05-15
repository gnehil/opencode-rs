use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;
use crate::agent::{AgentInfo, AgentMode};
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
                    "description": "The type of specialized agent to use"
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
            "required": ["description", "prompt", "subagent_type"]
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

            ctx.check_permission("task", &params.subagent_type).await?;

            let Some(agent) = resolve_task_agent(&params.subagent_type, ctx.config.as_ref()) else {
                return Ok(ToolResult::with_metadata(
                    format!(
                        "task_id: unavailable\n\n<task_result>\nUnknown subagent type: {}\nAvailable types: {}\n</task_result>",
                        params.subagent_type,
                        available_task_agents(ctx.config.as_ref()).join(", ")
                    ),
                    json!({
                        "task_id": serde_json::Value::Null,
                        "status": "error",
                        "error": "unknown_subagent_type",
                    }),
                ));
            };

            let provider: Arc<dyn Provider> = if let Some(provider) = ctx.provider.clone() {
                provider
            } else {
                Arc::new(AnthropicProvider::from_env().map_err(|e| {
                    anyhow::anyhow!("Failed to initialize Anthropic provider: {}", e)
                })?)
            };

            let store = if let Some(store) = ctx.store.clone() {
                store
            } else {
                let data_dir = std::env::var("OPENCODE_DATA_DIR")
                    .ok()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        dirs::data_local_dir()
                            .map(|p| p.join("opencode"))
                            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/opencode"))
                    });
                Arc::new(SessionStore::new(data_dir).await?)
            };

            let resume_session_id = params.session_id.as_deref().or(params.task_id.as_deref());
            let child_session = if let Some(session_id) = resume_session_id {
                if let Ok(parsed) = SessionID::parse(session_id) {
                    if let Some(session) = store.get(&parsed).await? {
                        session
                    } else {
                        store
                            .create(
                                &format!("{} (@{} subagent)", params.description, agent.name),
                                "default",
                                &ctx.working_dir.clone(),
                            )
                            .await?
                    }
                } else {
                    store
                        .create(
                            &format!("{} (@{} subagent)", params.description, agent.name),
                            "default",
                            &ctx.working_dir.clone(),
                        )
                        .await?
                }
            } else {
                store
                    .create(
                        &format!("{} (@{} subagent)", params.description, agent.name),
                        "default",
                        &ctx.working_dir.clone(),
                    )
                    .await?
            };

            let child_session_id = SessionID::parse(&child_session.id).unwrap_or_default();
            let _ = store.set_agent(&child_session_id, &agent.name).await;
            if let Some(model) = &agent.model {
                let _ = store
                    .set_model(
                        &child_session_id,
                        &format!("{}/{}", model.provider_id, model.model_id),
                    )
                    .await;
            }

            let mut processor = PromptProcessor::new(store.clone(), provider.clone())
                .with_agent(agent.name.clone())
                .with_session_permission_rules(derive_subagent_session_permission(
                    &ctx.permission_rules,
                    &agent,
                ));
            if let Some(config) = ctx.config.clone() {
                processor = processor.with_config(config);
            }
            if let Some(model_id) = agent
                .model
                .as_ref()
                .map(|model| model.model_id.clone())
                .or_else(|| ctx.model_id.clone())
            {
                processor = processor.with_model(model_id);
            }

            let result_text = match processor.process(&child_session_id, &params.prompt).await {
                Ok(text) => text,
                Err(e) => format!("Error: {}", e),
            };

            Ok(ToolResult::with_metadata(
                format!(
                    "task_id: {} (session_id: {})\n\n<task_result>\n{}\n</task_result>",
                    child_session.id, child_session.id, result_text
                ),
                json!({
                    "task_id": child_session.id,
                    "session_id": child_session.id,
                    "subagent_type": params.subagent_type,
                    "description": params.description,
                    "status": "completed",
                }),
            ))
        })
    }
}

fn resolve_task_agent(name: &str, config: Option<&crate::config::Config>) -> Option<AgentInfo> {
    crate::agent::resolve_agent(name, config).or_else(|| legacy_task_agent(name))
}

fn legacy_task_agent(name: &str) -> Option<AgentInfo> {
    match name {
        "oracle" => Some(read_only_agent(
            "oracle",
            "Read-only high-IQ consultant for debugging and architecture",
        )),
        "librarian" => Some(read_only_agent(
            "librarian",
            "Multi-repository code search and documentation retrieval",
        )),
        _ => None,
    }
}

fn read_only_agent(name: &str, description: &str) -> AgentInfo {
    AgentInfo {
        name: name.to_string(),
        description: Some(description.to_string()),
        mode: AgentMode::Subagent,
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
    }
}

fn available_task_agents(config: Option<&crate::config::Config>) -> Vec<String> {
    let mut names = crate::agent::list_agents(config)
        .into_iter()
        .filter(|agent| agent.mode != AgentMode::Primary)
        .map(|agent| agent.name)
        .collect::<Vec<_>>();
    for legacy in ["oracle", "librarian"] {
        if !names.iter().any(|name| name == legacy) {
            names.push(legacy.to_string());
        }
    }
    names.sort();
    names
}

fn derive_subagent_session_permission(
    parent_rules: &[crate::permission::PermissionRule],
    subagent: &AgentInfo,
) -> crate::permission::Ruleset {
    let mut rules = parent_rules
        .iter()
        .filter_map(|rule| {
            if rule.action != crate::permission::Action::Deny {
                return None;
            }
            if rule.permission == "edit" || rule.permission == "external_directory" {
                return Some(rule.clone());
            }
            if rule.permission == "*" {
                return Some(crate::permission::PermissionRule {
                    permission: "edit".to_string(),
                    pattern: rule.pattern.clone(),
                    action: crate::permission::Action::Deny,
                });
            }
            None
        })
        .collect::<Vec<_>>();

    if !subagent
        .permission
        .iter()
        .any(|rule| rule.permission == "todowrite")
    {
        rules.push(crate::permission::PermissionRule::deny_tool("todowrite"));
    }
    if !subagent
        .permission
        .iter()
        .any(|rule| rule.permission == "task")
    {
        rules.push(crate::permission::PermissionRule::deny_tool("task"));
    }

    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{Action, PermissionRule};

    fn ctx_with_rules(rules: Vec<PermissionRule>) -> ToolContext {
        ToolContext {
            session_id: SessionID::new(),
            working_dir: std::path::PathBuf::from("/tmp"),
            permission_rules: rules,
            event_bus: None,
            permission_broker: None,
            provider: None,
            store: None,
            config: None,
            agent_name: None,
            model_id: None,
            plugin_manager: None,
            question_broker: None,
            skip_permissions: false,
        }
    }

    #[test]
    fn task_schema_accepts_dynamic_subagent_names() {
        let schema = TaskTool.parameters_schema();
        assert!(schema["properties"]["subagent_type"].get("enum").is_none());
        assert_eq!(
            schema["required"],
            serde_json::json!(["description", "prompt", "subagent_type"])
        );
    }

    #[test]
    fn resolves_configured_subagent() {
        let config: crate::config::Config = serde_json::from_value(serde_json::json!({
            "agent": {
                "reviewer": {
                    "description": "Project review agent",
                    "mode": "subagent",
                    "prompt": "Review this project.",
                    "permission": {
                        "bash": "deny"
                    }
                }
            }
        }))
        .unwrap();

        let agent = resolve_task_agent("reviewer", Some(&config)).expect("configured agent");
        assert_eq!(agent.name, "reviewer");
        assert_eq!(agent.mode, AgentMode::Subagent);
        assert_eq!(agent.prompt.as_deref(), Some("Review this project."));
        assert!(agent
            .permission
            .iter()
            .any(|rule| rule.permission == "bash" && rule.action == Action::Deny));
    }

    #[test]
    fn derives_subagent_permissions_from_parent_ceiling() {
        let parent = vec![PermissionRule {
            permission: "*".to_string(),
            pattern: "*".to_string(),
            action: Action::Deny,
        }];
        let subagent = crate::agent::get_agent("general").unwrap();

        let rules = derive_subagent_session_permission(&parent, &subagent);

        assert!(rules
            .iter()
            .any(|rule| rule.permission == "edit" && rule.action == Action::Deny));
        assert!(!rules.iter().any(|rule| rule.permission == "todowrite"));
        assert!(rules
            .iter()
            .any(|rule| rule.permission == "task" && rule.action == Action::Deny));
    }

    #[tokio::test]
    async fn task_permission_deny_blocks_subagent_launch() {
        let rule = PermissionRule {
            permission: "task".to_string(),
            pattern: "general".to_string(),
            action: Action::Deny,
        };

        let result = TaskTool
            .execute(
                serde_json::json!({
                    "description": "Review code",
                    "prompt": "Review this code.",
                    "subagent_type": "general"
                }),
                ctx_with_rules(vec![rule]),
            )
            .await;

        let err = result.expect_err("task permission should block execution");
        assert!(err.to_string().contains("Tool 'task' denied"));
    }
}
