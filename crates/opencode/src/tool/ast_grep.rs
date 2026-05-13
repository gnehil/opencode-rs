use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Deserialize)]
pub struct AstGrepSearchParams {
    pub pattern: String,
    #[serde(rename = "lang")]
    pub language: String,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub globs: Option<Vec<String>>,
    #[serde(default)]
    pub context: Option<usize>,
}

pub struct AstGrepSearchTool;

impl Tool for AstGrepSearchTool {
    fn name(&self) -> &str {
        "ast_grep_search"
    }

    fn description(&self) -> &str {
        "Search code patterns across filesystem using AST-aware matching. Supports 25 languages."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "AST pattern with meta-variables ($VAR, $$$)"
                },
                "lang": {
                    "type": "string",
                    "enum": ["bash", "c", "cpp", "csharp", "css", "elixir", "go", "haskell",
                             "html", "java", "javascript", "json", "kotlin", "lua", "nix",
                             "php", "python", "ruby", "rust", "scala", "solidity", "swift",
                             "typescript", "tsx", "yaml"],
                    "description": "Target language"
                },
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Paths to search"
                },
                "globs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Include/exclude globs"
                },
                "context": {
                    "type": "integer",
                    "description": "Context lines around match"
                }
            },
            "required": ["pattern", "lang"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: AstGrepSearchParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid ast-grep parameters: {}", e))?;

            let search_path = params
                .paths
                .and_then(|p| p.first().cloned())
                .unwrap_or_else(|| ctx.working_dir.to_string_lossy().to_string());

            let sg_path = match params.language.as_str() {
                "typescript" => "ts",
                "tsx" => "tsx",
                "javascript" => "js",
                "rust" => "rust",
                "python" => "py",
                "go" => "go",
                "java" => "java",
                "kotlin" => "kt",
                "c" => "c",
                "cpp" => "cpp",
                "csharp" => "cs",
                "ruby" => "ruby",
                "lua" => "lua",
                "scala" => "scala",
                "swift" => "swift",
                "solidity" => "sol",
                "haskell" => "hs",
                "elixir" => "ex",
                "php" => "php",
                "nix" => "nix",
                "bash" => "sh",
                "json" => "json",
                "yaml" => "yaml",
                "html" => "html",
                "css" => "css",
                _ => params.language.as_str(),
            };

            let output = std::process::Command::new("sg")
                .args([
                    "run",
                    "--pattern",
                    &params.pattern,
                    "--lang",
                    sg_path,
                    "--json",
                    &search_path,
                ])
                .current_dir(&ctx.working_dir)
                .output();

            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    if stdout.is_empty() {
                        Ok(ToolResult::new("No matches found"))
                    } else {
                        let matches: Vec<String> = stdout
                            .lines()
                            .filter_map(|line| {
                                let json: serde_json::Value = serde_json::from_str(line).ok()?;
                                Some(format!(
                                    "{}:{} - {}",
                                    json["file"]["path"].as_str().unwrap_or("?"),
                                    json["range"]["start"]["line"].as_u64().unwrap_or(0),
                                    json["text"].as_str().unwrap_or("?")
                                ))
                            })
                            .collect();
                        Ok(ToolResult::new(matches.join("\n")))
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Ok(ToolResult::new(
                            "ast-grep (sg) not installed. Install with: cargo install ast-grep",
                        ))
                    } else {
                        Err(anyhow::anyhow!("ast-grep execution failed: {}", e))
                    }
                }
            }
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct AstGrepReplaceParams {
    pub pattern: String,
    pub rewrite: String,
    #[serde(rename = "lang")]
    pub language: String,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub globs: Option<Vec<String>>,
    #[serde(default = "default_true")]
    pub dry_run: bool,
}

fn default_true() -> bool {
    true
}

pub struct AstGrepReplaceTool;

impl Tool for AstGrepReplaceTool {
    fn name(&self) -> &str {
        "ast_grep_replace"
    }

    fn description(&self) -> &str {
        "Replace code patterns across filesystem with AST-aware rewriting. Dry-run by default."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "AST pattern to match"
                },
                "rewrite": {
                    "type": "string",
                    "description": "Replacement pattern (can use $VAR from pattern)"
                },
                "lang": {
                    "type": "string",
                    "enum": ["bash", "c", "cpp", "csharp", "css", "elixir", "go", "haskell",
                             "html", "java", "javascript", "json", "kotlin", "lua", "nix",
                             "php", "python", "ruby", "rust", "scala", "solidity", "swift",
                             "typescript", "tsx", "yaml"],
                    "description": "Target language"
                },
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Paths to search"
                },
                "globs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Include/exclude globs"
                },
                "dryRun": {
                    "type": "boolean",
                    "description": "Preview changes without applying (default: true)"
                }
            },
            "required": ["pattern", "rewrite", "lang"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: AstGrepReplaceParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid ast-grep replace parameters: {}", e))?;

            let search_path = params
                .paths
                .and_then(|p| p.first().cloned())
                .unwrap_or_else(|| ctx.working_dir.to_string_lossy().to_string());

            let sg_path = match params.language.as_str() {
                "typescript" => "ts",
                "tsx" => "tsx",
                "javascript" => "js",
                _ => params.language.as_str(),
            };

            let mut args = vec![
                "run",
                "--pattern",
                &params.pattern,
                "--rewrite",
                &params.rewrite,
                "--lang",
                sg_path,
            ];

            if params.dry_run {
                args.push("--json");
            } else {
                args.push("--update");
            }

            args.push(&search_path);

            let output = std::process::Command::new("sg")
                .args(&args)
                .current_dir(&ctx.working_dir)
                .output();

            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    if params.dry_run {
                        let changes: Vec<String> = stdout
                            .lines()
                            .filter_map(|line| {
                                let json: serde_json::Value = serde_json::from_str(line).ok()?;
                                Some(format!(
                                    "{}:{} - would replace with: {}",
                                    json["file"]["path"].as_str().unwrap_or("?"),
                                    json["range"]["start"]["line"].as_u64().unwrap_or(0),
                                    json["replacement"]["text"].as_str().unwrap_or("?")
                                ))
                            })
                            .collect();
                        Ok(ToolResult::with_metadata(
                            changes.join("\n"),
                            json!({ "dry_run": true, "changes_count": changes.len() }),
                        ))
                    } else {
                        Ok(ToolResult::new(format!(
                            "Applied {} replacements",
                            stdout.lines().count()
                        )))
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        Ok(ToolResult::new(
                            "ast-grep (sg) not installed. Install with: cargo install ast-grep",
                        ))
                    } else {
                        Err(anyhow::anyhow!("ast-grep execution failed: {}", e))
                    }
                }
            }
        })
    }
}
