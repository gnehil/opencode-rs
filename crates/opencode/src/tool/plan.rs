use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

#[derive(Debug, Deserialize)]
pub struct PlanToolParams {
    pub goal: String,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub constraints: Option<Vec<String>>,
}

pub struct PlanTool;

impl Tool for PlanTool {
    fn name(&self) -> &str {
        "plan"
    }

    fn description(&self) -> &str {
        "Create a structured plan for achieving a goal. Disallows all edit tools - planning only."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "The goal or objective to plan for"
                },
                "context": {
                    "type": "string",
                    "description": "Additional context about the situation"
                },
                "constraints": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Constraints or limitations to consider"
                }
            },
            "required": ["goal"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: PlanToolParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid plan parameters: {}", e))?;

            let plan = format!(
                "# Plan: {}\n\n## Context\n{}\n\n## Steps\n1. Analyze the current situation\n2. Identify key requirements\n3. Break down into actionable tasks\n4. Determine dependencies\n5. Estimate effort for each task\n6. Create timeline\n\n## Constraints\n{}\n\n## Next Actions\n- Review plan with stakeholders\n- Identify blockers\n- Start with highest priority task",
                params.goal,
                params.context.unwrap_or_else(|| "No additional context provided".to_string()),
                params.constraints.unwrap_or_default().join(", ")
            );

            Ok(ToolResult::with_metadata(
                plan,
                json!({
                    "goal": params.goal,
                    "plan_created": true,
                })
            ))
        })
    }
}