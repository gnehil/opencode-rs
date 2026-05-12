use std::fs;
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use serde_json::json;
use walkdir::WalkDir;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::{GrepParams, Tool};

const LIMIT: usize = 100;
const MAX_LINE_LENGTH: usize = 2000;

struct MatchEntry {
    path: String,
    line: usize,
    text: String,
    mtime: u64,
}

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Content search with regex. Returns matching lines from files."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for in file contents"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. Defaults to the current working directory."
                },
                "include": {
                    "type": "string",
                    "description": "File pattern to include in the search (e.g. \"*.js\", \"*.{ts,tsx}\")"
                }
            },
            "required": ["pattern"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: GrepParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid grep parameters: {}", e))?;

            if params.pattern.is_empty() {
                return Err(anyhow::anyhow!("pattern is required"));
            }

            let re =
                Regex::new(&params.pattern).map_err(|e| anyhow::anyhow!("Invalid regex pattern: {}", e))?;

            let search_dir = params.path.unwrap_or_else(|| ctx.working_dir.clone());
            let path = Path::new(&search_dir);
            if !path.exists() {
                return Err(anyhow::anyhow!("Search path not found: {}", search_dir.display()));
            }

            let mut matches: Vec<MatchEntry> = Vec::new();

            for entry in WalkDir::new(&search_dir).into_iter().filter_map(|e| e.ok()) {
                if !entry.file_type().is_file() {
                    continue;
                }

                let file_path = entry.path();

                if let Some(include) = &params.include {
                    let file_name = file_path.file_name().unwrap_or_default().to_string_lossy();
                    if !glob_match::glob_match(include, &*file_name) {
                        continue;
                    }
                }

                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let mtime = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                let full_path = file_path
                    .canonicalize()
                    .unwrap_or_else(|_| file_path.to_path_buf());
                let path_str = full_path.to_string_lossy().into_owned();

                for (line_idx, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        let truncated = if line.len() > MAX_LINE_LENGTH {
                            format!("{}...", &line[..MAX_LINE_LENGTH])
                        } else {
                            line.to_string()
                        };
                        matches.push(MatchEntry {
                            path: path_str.clone(),
                            line: line_idx + 1,
                            text: truncated,
                            mtime,
                        });
                    }
                }
            }

            if matches.is_empty() {
                return Ok(ToolResult::text("No files found"));
            }

            matches.sort_by(|a, b| b.mtime.cmp(&a.mtime));

            let truncated = matches.len() > LIMIT;
            if truncated {
                matches.truncate(LIMIT);
            }

            let mut output_lines = Vec::new();
            output_lines.push(format!("Found {} matches", matches.len()));

            let mut current_path = String::new();
            for m in &matches {
                if current_path != m.path {
                    if !current_path.is_empty() {
                        output_lines.push(String::new());
                    }
                    current_path = m.path.clone();
                    output_lines.push(format!("{}:", m.path));
                }
                output_lines.push(format!("  Line {}: {}", m.line, m.text));
            }

            if truncated {
                output_lines.push(String::new());
                output_lines.push(format!(
                    "(Results truncated: showing {} of {} matches. Consider using a more specific path or pattern.)",
                    LIMIT,
                    matches.len()
                ));
            }

            Ok(ToolResult::with_metadata(
                output_lines.join("\n"),
                json!({
                    "matches": matches.len(),
                    "truncated": truncated,
                }),
            ))
        })
    }
}
