use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Deserialize)]
pub struct PatchParams {
    #[serde(rename = "patchText")]
    pub patch_text: String,
}

pub struct ApplyPatchTool;

impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a patch to files. The patch text follows unified diff format."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "patchText": {
                    "type": "string",
                    "description": "The full patch text describing all changes"
                }
            },
            "required": ["patchText"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: PatchParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid apply_patch parameters: {}", e))?;

            if params.patch_text.trim().is_empty() {
                return Err(anyhow::anyhow!("patchText is required and cannot be empty"));
            }

            let hunks = parse_patch(&params.patch_text)?;

            if hunks.is_empty() {
                return Err(anyhow::anyhow!("No hunks found in patch"));
            }

            let mut changes: Vec<(std::path::PathBuf, String)> = Vec::new();

            for hunk in &hunks {
                let file_path = ctx.working_dir.join(&hunk.path);
                super::assert_external_directory(
                    &ctx,
                    &file_path,
                    super::ExternalKind::File,
                    false,
                )
                .await?;

                match hunk.operation.as_str() {
                    "add" => {
                        changes.push((file_path.clone(), hunk.content.clone()));
                        tokio::fs::create_dir_all(file_path.parent().unwrap()).await?;
                        tokio::fs::write(&file_path, &hunk.content).await?;
                    }
                    "delete" => {
                        if file_path.exists() {
                            tokio::fs::remove_file(&file_path).await?;
                        }
                    }
                    "update" => {
                        if file_path.exists() {
                            let original = tokio::fs::read_to_string(&file_path).await?;
                            let updated = apply_hunk(&original, hunk);
                            changes.push((file_path.clone(), updated.clone()));
                            tokio::fs::write(&file_path, updated).await?;
                        } else {
                            return Err(anyhow::anyhow!(
                                "File not found for update: {}",
                                file_path.display()
                            ));
                        }
                    }
                    _ => {}
                }
            }

            let summary = hunks
                .iter()
                .map(|h| match h.operation.as_str() {
                    "add" => format!("A {}", h.path),
                    "delete" => format!("D {}", h.path),
                    "update" => format!("M {}", h.path),
                    _ => h.path.clone(),
                })
                .collect::<Vec<_>>()
                .join("\n");

            Ok(ToolResult::with_metadata(
                format!("Success. Updated the following files:\n{}", summary),
                json!({
                    "hunks_count": hunks.len(),
                    "changes": hunks.iter().map(|h| json!({
                        "path": h.path,
                        "operation": h.operation,
                    })).collect::<Vec<_>>(),
                }),
            ))
        })
    }
}

#[derive(Debug, Clone)]
struct Hunk {
    path: String,
    operation: String,
    content: String,
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
}

fn parse_patch(patch_text: &str) -> Result<Vec<Hunk>> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current_hunk: Option<Hunk> = None;
    let mut current_path: Option<String> = None;
    let mut current_operation: Option<String> = None;

    for line in patch_text.lines() {
        if line.starts_with("*** Begin Patch") {
            continue;
        }
        if line.starts_with("*** End Patch") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            continue;
        }
        if line.starts_with("+++ ") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            current_path = Some(line.trim_start_matches("+++ ").trim().to_string());
            current_operation = Some("add".to_string());
            continue;
        }
        if line.starts_with("--- ") {
            let path = line.trim_start_matches("--- ").trim().to_string();
            current_path = Some(path);
            continue;
        }
        if line.starts_with("@@ ") {
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            let old_range: usize = 1;
            let new_range: usize = 1;

            current_hunk = Some(Hunk {
                path: current_path.clone().unwrap_or_default(),
                operation: current_operation
                    .clone()
                    .unwrap_or_else(|| "update".to_string()),
                content: String::new(),
                old_start: old_range,
                old_count: 0,
                new_start: new_range,
                new_count: 0,
            });
            continue;
        }

        if let Some(ref mut hunk) = current_hunk {
            if line.starts_with('+') {
                hunk.content
                    .push_str(&format!("{}\n", line.trim_start_matches('+')));
                hunk.new_count += 1;
            } else if line.starts_with('-') {
                hunk.old_count += 1;
            } else if !line.starts_with('\\') {
                hunk.content.push_str(&format!("{}\n", line));
            }
        }
    }

    if let Some(hunk) = current_hunk {
        hunks.push(hunk);
    }

    Ok(hunks)
}

fn apply_hunk(original: &str, hunk: &Hunk) -> String {
    let lines: Vec<&str> = original.lines().collect();
    let mut result = lines.clone();

    let start_idx = (hunk.old_start - 1).min(result.len());
    let remove_count = hunk.old_count.min(result.len() - start_idx);

    result.splice(
        start_idx..start_idx + remove_count,
        hunk.content.lines().collect::<Vec<&str>>(),
    );

    result.join("\n")
}
