use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use rmcp::model::Tool as RmcpTool;
use serde_json::Value;

use crate::mcp::client::McpClient;
use crate::mcp::result::{McpContent, McpToolResult};
use crate::tool::{Tool, ToolContext, ToolResult};

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl McpTool {
    pub fn from_rmcp(tool: &RmcpTool) -> Self {
        Self {
            name: tool.name.to_string(),
            description: tool.description.to_string(),
            input_schema: tool.schema_as_json_value(),
        }
    }
}

pub fn sanitize_tool_name(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub struct McpRuntimeTool {
    name: String,
    server_name: String,
    tool_name: String,
    description: String,
    input_schema: Value,
    client: Arc<McpClient>,
}

impl McpRuntimeTool {
    pub fn new(server_name: impl Into<String>, tool: McpTool, client: Arc<McpClient>) -> Self {
        let server_name = server_name.into();
        let name = format!(
            "{}_{}",
            sanitize_tool_name(&server_name),
            sanitize_tool_name(&tool.name)
        );
        Self {
            name,
            server_name,
            tool_name: tool.name,
            description: tool.description,
            input_schema: normalize_input_schema(tool.input_schema),
            client,
        }
    }
}

impl Tool for McpRuntimeTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn execute(
        &self,
        params: Value,
        ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + '_>> {
        let client = self.client.clone();
        let server_name = self.server_name.clone();
        let tool_name = self.tool_name.clone();
        let exposed_name = self.name.clone();
        Box::pin(async move {
            ctx.check_permission(&exposed_name, &format!("{}:{}", server_name, tool_name))
                .await?;

            let result = client.call_tool(&tool_name, params).await?;
            let output = format_tool_result(&result);
            if result.is_error {
                anyhow::bail!("MCP tool '{}' returned error: {}", exposed_name, output);
            }
            Ok(ToolResult::with_metadata(
                output,
                serde_json::json!({
                    "mcp": {
                        "server": server_name,
                        "tool": tool_name,
                        "isError": result.is_error
                    }
                }),
            ))
        })
    }
}

fn normalize_input_schema(schema: Value) -> Value {
    let mut schema = match schema {
        Value::Object(map) => Value::Object(map),
        _ => serde_json::json!({}),
    };
    if let Value::Object(map) = &mut schema {
        map.entry("type".to_string())
            .or_insert_with(|| Value::String("object".to_string()));
        map.entry("properties".to_string())
            .or_insert_with(|| serde_json::json!({}));
        map.entry("additionalProperties".to_string())
            .or_insert_with(|| Value::Bool(false));
    }
    schema
}

fn format_tool_result(result: &McpToolResult) -> String {
    result
        .content
        .iter()
        .map(|content| match content {
            McpContent::Text { text } => text.clone(),
            McpContent::Image { data, mime_type } => {
                format!("[image: {}, {} base64 chars]", mime_type, data.len())
            }
            McpContent::Resource { resource } => {
                if let Some(text) = &resource.text {
                    format!("[resource: {}]\n{}", resource.uri, text)
                } else if let Some(blob) = &resource.blob {
                    format!("[resource: {}, {} base64 chars]", resource.uri, blob.len())
                } else {
                    format!("[resource: {}]", resource.uri)
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_tool_name_matches_ts_mcp_naming() {
        assert_eq!(sanitize_tool_name("browser tools"), "browser_tools");
        assert_eq!(sanitize_tool_name("repo:search"), "repo_search");
        assert_eq!(sanitize_tool_name("ok-name_1"), "ok-name_1");
    }

    #[test]
    fn normalize_input_schema_forces_object_shape() {
        let schema = normalize_input_schema(serde_json::json!({
            "properties": {
                "query": { "type": "string" }
            }
        }));
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
    }
}
