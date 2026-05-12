use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

#[derive(Debug, Deserialize)]
pub struct RepoCloneParams {
    pub url: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
}

pub struct RepoCloneTool;

impl Tool for RepoCloneTool {
    fn name(&self) -> &str { "repo_clone" }

    fn description(&self) -> &str {
        "Clone a repository from URL into the managed cache for dependency inspection."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Repository URL to clone" },
                "path": { "type": "string", "description": "Target directory path" },
                "branch": { "type": "string", "description": "Branch to clone" },
                "depth": { "type": "integer", "description": "Clone depth (shallow clone)" }
            },
            "required": ["url"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: RepoCloneParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid repo_clone parameters: {}", e))?;

            let target_path = params.path.unwrap_or_else(|| {
                let repo_name = params.url.split('/').last().unwrap_or("repo");
                format!("{}/{}", ctx.working_dir, repo_name.replace(".git", ""))
            });

            let mut args = vec!["clone", &params.url, &target_path];
            if let Some(branch) = &params.branch {
                args.extend(["--branch", branch]);
            }
            if let Some(depth) = params.depth {
                args.extend(["--depth", &depth.to_string()]);
            }

            let output = std::process::Command::new("git")
                .args(&args)
                .output();

            match output {
                Ok(o) => {
                    if o.status.success() {
                        Ok(ToolResult::with_metadata(
                            format!("Cloned {} to {}", params.url, target_path),
                            json!({ "url": params.url, "path": target_path, "success": true })
                        ))
                    } else {
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        Err(anyhow::anyhow!("Git clone failed: {}", stderr))
                    }
                }
                Err(e) => Err(anyhow::anyhow!("Failed to execute git: {}", e)),
            }
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct RepoOverviewParams {
    pub path: String,
    #[serde(default)]
    pub max_files: Option<usize>,
}

pub struct RepoOverviewTool;

impl Tool for RepoOverviewTool {
    fn name(&self) -> &str { "repo_overview" }

    fn description(&self) -> &str {
        "Generate an overview analysis of a repository structure and content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Repository path to analyze" },
                "max_files": { "type": "integer", "description": "Maximum files to include in overview" }
            },
            "required": ["path"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: RepoOverviewParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid repo_overview parameters: {}", e))?;

            let path = std::path::PathBuf::from(&params.path);
            if !path.exists() {
                return Err(anyhow::anyhow!("Path not found: {}", params.path));
            }

            let mut overview = format!("# Repository Overview: {}\n\n", params.path);

            if let Ok(entries) = std::fs::read_dir(&path) {
                let mut dirs = Vec::new();
                let mut files = Vec::new();

                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') { continue; }
                    if entry.path().is_dir() {
                        dirs.push(name);
                    } else {
                        files.push(name);
                    }
                }

                overview.push_str(&format!("## Directories ({})\n{}\n\n", dirs.len(), dirs.join(", ")));
                overview.push_str(&format!("## Root Files ({})\n{}\n\n", files.len(), files.join(", ")));
            }

            if let Ok(output) = std::process::Command::new("git")
                .args(["log", "--oneline", "-10"])
                .current_dir(&path)
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                overview.push_str(&format!("## Recent Commits\n{}\n", stdout));
            }

            Ok(ToolResult::new(overview))
        })
    }
}