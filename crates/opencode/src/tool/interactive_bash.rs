use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

#[derive(Debug, Deserialize)]
pub struct InteractiveBashParams {
    pub tmux_command: String,
}

pub struct InteractiveBashTool;

impl Tool for InteractiveBashTool {
    fn name(&self) -> &str {
        "interactive_bash"
    }

    fn description(&self) -> &str {
        "Execute tmux subcommands directly for TUI apps. Pass tmux subcommands without 'tmux' prefix."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "tmux_command": {
                    "type": "string",
                    "description": "The tmux command to execute (without 'tmux' prefix). Examples: 'new-session -d -s my-dev', 'send-keys -t my-dev \"vim\" Enter'"
                }
            },
            "required": ["tmux_command"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: InteractiveBashParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid interactive_bash parameters: {}", e))?;

            let output = std::process::Command::new("tmux")
                .args(params.tmux_command.split_whitespace())
                .output();

            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    
                    if !stderr.is_empty() {
                        Ok(ToolResult::with_metadata(
                            format!("tmux {}\nstderr: {}", params.tmux_command, stderr),
                            json!({ "success": false, "error": stderr.to_string() })
                        ))
                    } else {
                        Ok(ToolResult::with_metadata(
                            format!("tmux {}\n{}", params.tmux_command, stdout),
                            json!({ "success": true, "output": stdout.to_string() })
                        ))
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Ok(ToolResult::new("tmux not installed. Install with: apt install tmux or brew install tmux"))
                    } else {
                        Err(anyhow::anyhow!("tmux execution failed: {}", e))
                    }
                }
            }
        })
    }
}