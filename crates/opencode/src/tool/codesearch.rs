use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Deserialize)]
pub struct CodeSearchParams {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub file_pattern: Option<String>,
    #[serde(default)]
    pub case_sensitive: Option<bool>,
}

pub struct CodeSearchTool;

impl Tool for CodeSearchTool {
    fn name(&self) -> &str {
        "codesearch"
    }

    fn description(&self) -> &str {
        "Search for code patterns across the codebase or external repositories."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The code pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Path to search (defaults to current directory)"
                },
                "language": {
                    "type": "string",
                    "description": "Filter by programming language"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "File pattern to match (e.g., '*.ts')"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Case sensitive search"
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
            let params: CodeSearchParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid codesearch parameters: {}", e))?;

            let search_path = params
                .path
                .unwrap_or_else(|| ctx.working_dir.to_string_lossy().to_string());

            let mut args = vec!["-r", "-n"];

            if !params.case_sensitive.unwrap_or(false) {
                args.push("-i");
            }

            args.push(&params.pattern);
            args.push(&search_path);

            if let Some(lang) = &params.language {
                let ext = match lang.as_str() {
                    "typescript" => "--include=*.ts",
                    "tsx" => "--include=*.tsx",
                    "javascript" => "--include=*.js",
                    "python" => "--include=*.py",
                    "rust" => "--include=*.rs",
                    "go" => "--include=*.go",
                    "java" => "--include=*.java",
                    "c" => "--include=*.c",
                    "cpp" => "--include=*.cpp",
                    "ruby" => "--include=*.rb",
                    "php" => "--include=*.php",
                    _ => "",
                };
                if !ext.is_empty() {
                    args.push(ext);
                }
            }

            let include_arg = params
                .file_pattern
                .as_ref()
                .map(|fp| format!("--include={}", fp));
            if let Some(ia) = include_arg.as_deref() {
                args.push(ia);
            }

            let output = std::process::Command::new("grep").args(&args).output();

            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    if stdout.is_empty() {
                        Ok(ToolResult::new("No matches found"))
                    } else {
                        let matches: Vec<String> = stdout
                            .lines()
                            .take(50)
                            .map(|line| line.to_string())
                            .collect();
                        Ok(ToolResult::with_metadata(
                            matches.join("\n"),
                            json!({ "pattern": params.pattern, "matches": matches.len() }),
                        ))
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Ok(ToolResult::new(
                            "grep not available. Install ripgrep for better search.",
                        ))
                    } else {
                        Err(anyhow::anyhow!("Search failed: {}", e))
                    }
                }
            }
        })
    }
}
