use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Deserialize)]
pub struct TruncateParams {
    pub file_path: String,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub max_lines: Option<usize>,
}

pub struct TruncateTool;

impl Tool for TruncateTool {
    fn name(&self) -> &str {
        "truncate"
    }

    fn description(&self) -> &str {
        "Truncate a file to specified size or number of lines."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to truncate"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum bytes to keep"
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Maximum lines to keep"
                }
            },
            "required": ["file_path"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: TruncateParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid truncate parameters: {}", e))?;

            let path = std::path::PathBuf::from(&params.file_path);

            if !path.exists() {
                return Err(anyhow::anyhow!("File not found: {}", params.file_path));
            }

            if params.max_lines.is_some() {
                let content = std::fs::read_to_string(&path)?;
                let lines: Vec<&str> = content.lines().collect();
                let max_lines = params.max_lines.unwrap();

                if lines.len() > max_lines {
                    let truncated = lines[..max_lines].join("\n");
                    std::fs::write(&path, truncated)?;
                    return Ok(ToolResult::new(format!(
                        "Truncated {} from {} to {} lines",
                        params.file_path,
                        lines.len(),
                        max_lines
                    )));
                }
                return Ok(ToolResult::new(format!(
                    "File {} has {} lines, no truncation needed",
                    params.file_path,
                    lines.len()
                )));
            }

            if params.max_bytes.is_some() {
                let content = std::fs::read(&path)?;
                let max_bytes = params.max_bytes.unwrap() as usize;

                if content.len() > max_bytes {
                    let truncated = &content[..max_bytes];
                    std::fs::write(&path, truncated)?;
                    return Ok(ToolResult::new(format!(
                        "Truncated {} from {} to {} bytes",
                        params.file_path,
                        content.len(),
                        max_bytes
                    )));
                }
                return Ok(ToolResult::new(format!(
                    "File {} has {} bytes, no truncation needed",
                    params.file_path,
                    content.len()
                )));
            }

            Ok(ToolResult::new(format!(
                "No truncation performed for {}",
                params.file_path
            )))
        })
    }
}
