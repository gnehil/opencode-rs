use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Deserialize)]
pub struct RepoSearchParams {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub use_regexp: Option<bool>,
    #[serde(default)]
    pub match_case: Option<bool>,
    #[serde(default)]
    pub match_whole_words: Option<bool>,
}

pub struct RepoSearchTool;

impl Tool for RepoSearchTool {
    fn name(&self) -> &str {
        "grep_app_searchGitHub"
    }

    fn description(&self) -> &str {
        "Find real-world code examples from GitHub repositories to help answer programming questions."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The literal code pattern to search for (like 'useState(', 'async function')"
                },
                "path": {
                    "type": "string",
                    "description": "Filter by file path"
                },
                "language": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Filter by language (TypeScript, Python, etc)"
                },
                "useRegexp": {
                    "type": "boolean",
                    "description": "Use regex patterns"
                },
                "matchCase": {
                    "type": "boolean",
                    "description": "Case sensitive search"
                },
                "matchWholeWords": {
                    "type": "boolean",
                    "description": "Match whole words only"
                }
            },
            "required": ["pattern"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: RepoSearchParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid search parameters: {}", e))?;

            let url = format!(
                "https://api.github.com/search/code?q={}{}{}",
                params.pattern,
                if let Some(lang) = &params.language {
                    format!("+language:{}", lang)
                } else {
                    "".to_string()
                },
                if let Some(path) = &params.path {
                    format!("+path:{}", path)
                } else {
                    "".to_string()
                }
            );

            let client = reqwest::Client::new();
            let response = client
                .get(&url)
                .header("Accept", "application/vnd.github.v3+json")
                .header("User-Agent", "opencode-rs")
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("GitHub API request failed: {}", e))?;

            let data: serde_json::Value = response
                .json()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to parse response: {}", e))?;

            let results: Vec<String> = data["items"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .take(10)
                .map(|item| {
                    format!(
                        "{} - {}",
                        item["repository"]["full_name"]
                            .as_str()
                            .unwrap_or("unknown"),
                        item["path"].as_str().unwrap_or("unknown")
                    )
                })
                .collect();

            Ok(ToolResult::new(results.join("\n")))
        })
    }
}
