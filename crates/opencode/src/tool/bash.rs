use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::{BashParams, Tool};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute shell commands with timeout. Use for terminal operations like git, npm, docker, etc."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory for the command (default: current working directory)"
                }
            },
            "required": ["command"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: BashParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid bash parameters: {}", e))?;

            let cwd = params.workdir.unwrap_or_else(|| ctx.working_dir.clone());
            let shell = detect_shell();

            let mut cmd = Command::new(&shell);
            cmd.arg("-c")
                .arg(&params.command)
                .current_dir(&cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let mut child = cmd.spawn().context("Failed to spawn shell")?;

let stdout_handle = child.stdout.take();
            let stderr_handle = child.stderr.take();

            let timeout = tokio::time::Duration::from_millis(DEFAULT_TIMEOUT_MS);

            let output = tokio::time::timeout(timeout, async {
                let mut stdout_buf = Vec::new();
                let mut stderr_buf = Vec::new();

                if let Some(mut stdout) = stdout_handle {
                    let _ = stdout.read_to_end(&mut stdout_buf).await;
                }
                if let Some(mut stderr) = stderr_handle {
                    let _ = stderr.read_to_end(&mut stderr_buf).await;
                }

                let status = child.wait().await;
                (status, stdout_buf, stderr_buf)
            })
            .await;

            let (exit_status, stdout_buf, stderr_buf) = match output {
                Ok((Ok(status), stdout, stderr)) => (status, stdout, stderr),
                Ok((Err(e), _, _)) => {
                    let _ = child.kill().await;
                    return Err(anyhow::anyhow!("Command failed: {}", e));
                }
                Err(_) => {
                    let _ = child.kill().await;
                    return Ok(ToolResult::text(format!(
                        "Command timed out after {}ms and was killed.",
                        DEFAULT_TIMEOUT_MS
                    )));
                }
            };

            let stdout_truncated = truncate_output(&stdout_buf);
            let stderr_truncated = truncate_output(&stderr_buf);
            let stdout = String::from_utf8_lossy(&stdout_truncated);
            let stderr = String::from_utf8_lossy(&stderr_truncated);
            let exit_code = exit_status.code().unwrap_or(-1);

            let mut output_text = String::new();
            if !stdout.is_empty() {
                output_text.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !output_text.is_empty() {
                    output_text.push_str("\n\n");
                }
                output_text.push_str("Stderr:\n");
                output_text.push_str(&stderr);
            }

            if output_text.is_empty() {
                output_text.push_str("(no output)");
            }

            Ok(ToolResult::with_metadata(
                output_text,
                json!({
                    "exit_code": exit_code,
                    "command": params.command,
                    "cwd": cwd.display().to_string(),
                }),
            ))
        })
    }
}

fn detect_shell() -> PathBuf {
    if let Ok(shell) = std::env::var("SHELL") {
        return PathBuf::from(shell);
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(comspec) = std::env::var("COMSPEC") {
            return PathBuf::from(comspec);
        }
        return PathBuf::from("cmd.exe");
    }
    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from("/bin/sh")
    }
}

fn truncate_output(data: &[u8]) -> Vec<u8> {
    if data.len() <= MAX_OUTPUT_BYTES {
        return data.to_vec();
    }
    let tail = &data[data.len() - MAX_OUTPUT_BYTES..];
    let mut result = b"...output truncated...\n".to_vec();
    let mut start = 0;
    for (i, &byte) in tail.iter().enumerate() {
        if byte & 0xC0 != 0x80 {
            start = i;
            break;
        }
    }
    result.extend_from_slice(&tail[start..]);
    result
}
