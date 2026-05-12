use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

#[derive(Debug, Deserialize)]
pub struct SkillParams {
    pub name: String,
    #[serde(default)]
    pub user_message: Option<String>,
}

pub struct SkillTool;

impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        "Load a skill or execute a slash command to get detailed instructions for a specific task."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "The skill or command name"},
                "user_message": {"type": "string", "description": "Optional arguments"}
            },
            "required": ["name"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: SkillParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid skill parameters: {}", e))?;

            let skill_path = ctx.working_dir
                .join(".opencode")
                .join("skill")
                .join(format!("{}.md", params.name));

            if !skill_path.exists() {
                return Ok(ToolResult::text(format!(
                    "Skill '{}' not found. Check .opencode/skill/ directory.",
                    params.name
                )));
            }

            let content = tokio::fs::read_to_string(&skill_path).await?;

            let output = format!(
                "<skill_content name=\"{}\">\n{}\n</skill_content>",
                params.name,
                content
            );

            Ok(ToolResult::with_metadata(
                output,
                json!({
                    "name": params.name,
                    "path": skill_path.display().to_string(),
                }),
            ))
        })
    }
}