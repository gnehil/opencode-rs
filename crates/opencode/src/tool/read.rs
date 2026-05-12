use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::{ReadParams, Tool};

const MAX_LINE_LENGTH: usize = 2000;
const MAX_LINE_SUFFIX: &str = "... (line truncated to 2000 chars)";
const MAX_BYTES: usize = 50 * 1024;

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read file contents. Supports files and directories. Returns content with line numbers."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "filePath": {
                    "type": "string",
                    "description": "The absolute path to the file or directory to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "The line number to start reading from (1-indexed)",
                    "default": 1
                },
                "limit": {
                    "type": "integer",
                    "description": "The maximum number of lines to read (defaults to 2000)",
                    "default": 2000
                }
            },
            "required": ["filePath"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: ReadParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid read parameters: {}", e))?;

            let path = Path::new(&params.file_path);
            if !path.exists() {
                return Err(anyhow::anyhow!("File not found: {}", params.file_path));
            }

            if path.is_dir() {
                return read_directory(path, &params);
            }

            read_file(path, &params)
        })
    }
}

fn read_directory(path: &Path, params: &ReadParams) -> Result<ToolResult> {
    let entries: Vec<String> = fs::read_dir(path)
        .with_context(|| format!("Cannot read directory: {}", path.display()))?
        .filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                format!("{}/", name_str)
            } else {
                name_str.into_owned()
            }
        })
        .collect();

    let start = params.offset.saturating_sub(1);
    let end = (start + params.limit).min(entries.len());
    let sliced = if start < entries.len() {
        &entries[start..end]
    } else {
        &[]
    };
    let truncated = end < entries.len();

    let output = format!(
        "<path>{}</path>\n<type>directory</type>\n<entries>\n{}\n</entries>",
        path.display(),
        sliced.join("\n")
    );

    Ok(ToolResult::with_metadata(
        output,
        json!({
            "truncated": truncated,
            "total_entries": entries.len(),
        }),
    ))
}

fn read_file(path: &Path, params: &ReadParams) -> Result<ToolResult> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Cannot read file: {}", path.display()))?;

    let total_lines = content.lines().count();
    let start = params.offset.saturating_sub(1);
    let limit = params.limit;

    let mut output_lines = Vec::new();
    let mut bytes_read = 0;
    let mut more = false;
    let mut cut = false;

    for (i, line) in content.lines().enumerate() {
        if i < start {
            continue;
        }
        if output_lines.len() >= limit {
            more = true;
            break;
        }

        let truncated = if line.len() > MAX_LINE_LENGTH {
            format!("{}{}", &line[..MAX_LINE_LENGTH], MAX_LINE_SUFFIX)
        } else {
            line.to_string()
        };
        let line_size = truncated.len() + 1;

        if bytes_read + line_size > MAX_BYTES && !output_lines.is_empty() {
            cut = true;
            more = true;
            break;
        }

        output_lines.push((i + 1, truncated));
        bytes_read += line_size;
    }

    let last_line = output_lines.last().map(|(l, _)| *l).unwrap_or(start + 1);
    let next_offset = last_line + 1;

    let mut output = format!(
        "<path>{}</path>\n<type>file</type>\n<content>\n",
        path.display()
    );

    for (line_num, line) in &output_lines {
        output.push_str(&format!("{}: {}\n", line_num, line));
    }

    if cut {
        output.push_str(&format!(
            "\n(Output capped at {}KB. Showing lines {}-{}. Use offset={} to continue.)",
            MAX_BYTES / 1024,
            params.offset,
            last_line,
            next_offset
        ));
    } else if more {
        output.push_str(&format!(
            "\n(Showing lines {}-{} of {}. Use offset={} to continue.)",
            params.offset,
            last_line,
            total_lines,
            next_offset
        ));
    } else {
        output.push_str(&format!("\n(End of file - total {} lines)", total_lines));
    }
    output.push_str("\n</content>");

    if start >= total_lines && total_lines > 0 {
        return Err(anyhow::anyhow!(
            "Offset {} is out of range for this file ({} lines)",
            params.offset,
            total_lines
        ));
    }

    Ok(ToolResult::with_metadata(
        output,
        json!({
            "preview": output_lines.iter().take(20).map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n"),
            "truncated": more,
            "total_lines": total_lines,
        }),
    ))
}
