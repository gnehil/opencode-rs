use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::{EditParams, Tool};
use super::result::ToolResult;

pub struct EditTool;

impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Exact string replacement in files. Provide oldString and newString for replacement."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "filePath": {
                    "type": "string",
                    "description": "The absolute path to the file to modify"
                },
                "oldString": {
                    "type": "string",
                    "description": "The text to replace"
                },
                "newString": {
                    "type": "string",
                    "description": "The text to replace it with (must be different from oldString)"
                },
                "replaceAll": {
                    "type": "boolean",
                    "description": "Replace all occurrences of oldString (default false)",
                    "default": false
                }
            },
            "required": ["filePath", "oldString", "newString"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: EditParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid edit parameters: {}", e))?;

            ctx.check_permission("edit", &params.file_path).await?;

            if params.old_string == params.new_string {
                return Err(anyhow::anyhow!(
                    "No changes to apply: oldString and newString are identical."
                ));
            }

            let path = Path::new(&params.file_path);
            if !path.exists() {
                return Err(anyhow::anyhow!("File not found: {}", params.file_path));
            }
            if path.is_dir() {
                return Err(anyhow::anyhow!(
                    "Path is a directory, not a file: {}",
                    params.file_path
                ));
            }

            let content = fs::read_to_string(path)
                .with_context(|| format!("Cannot read file: {}", path.display()))?;

            let count = count_matches(&content, &params.old_string);
            if count == 0 {
                return Err(anyhow::anyhow!(
                    "Could not find oldString in the file. It must match exactly, including whitespace, indentation, and line endings."
                ));
            }
            if count > 1 && !params.replace_all {
                return Err(anyhow::anyhow!(
                    "Found {} matches for oldString. Provide more surrounding context to make the match unique, or use replaceAll=true.",
                    count
                ));
            }

            let new_content = if params.replace_all {
                content.replace(&params.old_string, &params.new_string)
            } else {
                replace_first(&content, &params.old_string, &params.new_string)
            };

            let diff = build_diff(&content, &new_content, &params.file_path);

            fs::write(path, &new_content)
                .with_context(|| format!("Failed to write file: {}", path.display()))?;

            let additions = count_lines_added(&content, &new_content);
            let deletions = count_lines_removed(&content, &new_content);

            Ok(ToolResult::with_metadata(
                "Edit applied successfully.".to_string(),
                json!({
                    "diff": diff,
                    "additions": additions,
                    "deletions": deletions,
                    "filepath": params.file_path,
                }),
            ))
        })
    }
}

fn count_matches(content: &str, old: &str) -> usize {
    if old.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = content[start..].find(old) {
        count += 1;
        start += pos + old.len();
    }
    count
}

fn replace_first(content: &str, old: &str, new: &str) -> String {
    if let Some(pos) = content.find(old) {
        let mut result = String::with_capacity(content.len() + new.len() - old.len());
        result.push_str(&content[..pos]);
        result.push_str(new);
        result.push_str(&content[pos + old.len()..]);
        result
    } else {
        content.to_string()
    }
}

fn build_diff(old: &str, new: &str, path: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let max_len = old_lines.len().max(new_lines.len());

    let mut diff = format!("--- {}\n+++ {}\n", path, path);
    let mut hunk_lines = Vec::new();
    let mut in_hunk = false;
    let mut hunk_start = 0;

    for i in 0..max_len {
        let old_line = old_lines.get(i);
        let new_line = new_lines.get(i);

        match (old_line, new_line) {
            (Some(a), Some(b)) if a == b => {
                if in_hunk {
                    hunk_lines.push(format!(" {}", a));
                    if hunk_lines.len() > 3 {
                        flush_hunk(&mut diff, hunk_start, &hunk_lines);
                        hunk_lines.clear();
                        in_hunk = false;
                    }
                }
            }
            _ => {
                if !in_hunk {
                    hunk_start = i + 1;
                    in_hunk = true;
                }
                if let Some(a) = old_line {
                    hunk_lines.push(format!("-{}", a));
                }
                if let Some(b) = new_line {
                    hunk_lines.push(format!("+{}", b));
                }
            }
        }
    }

    if in_hunk && !hunk_lines.is_empty() {
        flush_hunk(&mut diff, hunk_start, &hunk_lines);
    }

    if diff.lines().count() <= 2 {
        return "(no changes)".to_string();
    }
    diff
}

fn flush_hunk(diff: &mut String, start: usize, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    diff.push_str(&format!("@@ -{},0 +{},{} @@\n", start, start, lines.len()));
    for line in lines {
        diff.push_str(line);
        diff.push('\n');
    }
}

fn count_lines_added(old: &str, new: &str) -> usize {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut count = 0;
    let mut old_idx = 0;
    for new_line in &new_lines {
        if old_idx < old_lines.len() && old_lines[old_idx] == *new_line {
            old_idx += 1;
        } else {
            count += 1;
        }
    }
    count
}

fn count_lines_removed(old: &str, new: &str) -> usize {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut count = 0;
    let mut new_idx = 0;
    for old_line in &old_lines {
        if new_idx < new_lines.len() && new_lines[new_idx] == *old_line {
            new_idx += 1;
        } else {
            count += 1;
        }
    }
    count
}
