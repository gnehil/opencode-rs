use std::path::Path;

use anyhow::Result;
use glob_match::glob_match;
use serde_json::json;
use walkdir::WalkDir;

use super::context::ToolContext;
use super::r#trait::{GlobParams, Tool};
use super::result::ToolResult;

const LIMIT: usize = 100;

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Pattern-based file search using glob patterns. Returns matching file paths."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The glob pattern to match files against"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in. If not specified, the current working directory will be used."
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
            let params: GlobParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid glob parameters: {}", e))?;

            let search_dir = params.path.unwrap_or_else(|| ctx.working_dir.clone());
            super::assert_external_directory(
                &ctx,
                &search_dir,
                super::ExternalKind::Directory,
                false,
            )
            .await?;
            let path = Path::new(&search_dir);
            if !path.exists() {
                return Err(anyhow::anyhow!(
                    "Search directory not found: {}",
                    search_dir.display()
                ));
            }
            if !path.is_dir() {
                return Err(anyhow::anyhow!(
                    "Glob path must be a directory: {}",
                    search_dir.display()
                ));
            }

            let mut files: Vec<(String, u64)> = Vec::new();

            for entry in WalkDir::new(&search_dir).into_iter().filter_map(|e| e.ok()) {
                if !entry.file_type().is_file() {
                    continue;
                }

                let file_path = entry.path();
                let relative = file_path.strip_prefix(&search_dir).unwrap_or(file_path);
                let relative_str = relative.to_string_lossy().replace('\\', "/");

                if glob_match(&params.pattern, &relative_str)
                    || glob_match(&params.pattern, &file_path.to_string_lossy())
                {
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
                    files.push((full_path.to_string_lossy().into_owned(), mtime));
                }
            }

            files.sort_by(|a, b| b.1.cmp(&a.1));

            let total_count = files.len();
            let truncated = total_count > LIMIT;
            if truncated {
                files.truncate(LIMIT);
            }

            let paths: Vec<String> = files.into_iter().map(|(p, _)| p).collect();

            let mut output_lines = Vec::new();
            if paths.is_empty() {
                output_lines.push("No files found".to_string());
            } else {
                output_lines.extend(paths);
                if truncated {
                    output_lines.push(String::new());
                    output_lines.push(format!(
                        "(Results are truncated: showing first {} results. Consider using a more specific path or pattern.)",
                        LIMIT
                    ));
                }
            }

            Ok(ToolResult::with_metadata(
                output_lines.join("\n"),
                json!({
                    "count": total_count,
                    "truncated": truncated,
                }),
            ))
        })
    }
}
