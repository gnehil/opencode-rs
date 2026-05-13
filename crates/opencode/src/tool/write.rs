use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::{Tool, WriteParams};

pub struct WriteTool;

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write file contents. Creates the file if it doesn't exist, or overwrites if it does."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "filePath": {
                    "type": "string",
                    "description": "The absolute path to the file to write (must be absolute, not relative)"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["filePath", "content"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: WriteParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid write parameters: {}", e))?;

            ctx.check_permission("edit", &params.file_path).await?;

            let path = Path::new(&params.file_path);

            if let Some(parent) = path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Err(anyhow::anyhow!(
                        "Cannot create directory {}: {}",
                        parent.display(),
                        e
                    ));
                }
            }

            let existed = path.exists();

            fs::write(path, &params.content)
                .with_context(|| format!("Failed to write file: {}", path.display()))?;

            let output = if existed {
                "Wrote file successfully.".to_string()
            } else {
                "Created and wrote file successfully.".to_string()
            };

            Ok(ToolResult::with_metadata(
                output,
                json!({
                    "filepath": params.file_path,
                    "exists": existed,
                }),
            ))
        })
    }
}
