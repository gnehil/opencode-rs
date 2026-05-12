use rmcp::model::Tool;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl McpTool {
    pub fn from_rmcp(tool: &Tool) -> Self {
        Self {
            name: tool.name.to_string(),
            description: tool.description.to_string(),
            input_schema: tool.schema_as_json_value(),
        }
    }
}
