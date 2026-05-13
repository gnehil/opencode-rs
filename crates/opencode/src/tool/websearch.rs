use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Deserialize)]
pub struct WebSearchParams {
    pub query: String,
    #[serde(default = "default_num_results")]
    pub numResults: usize,
    #[serde(default)]
    pub livecrawl: Option<String>,
    #[serde(default, rename = "type")]
    pub search_type: Option<String>,
}

fn default_num_results() -> usize {
    8
}

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "websearch"
    }

    fn description(&self) -> &str {
        "Search the web for any topic and get clean, ready-to-use content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query"
                },
                "numResults": {
                    "type": "integer",
                    "default": 8,
                    "description": "Number of search results to return"
                },
                "livecrawl": {
                    "type": "string",
                    "enum": ["fallback", "preferred"],
                    "description": "Live crawl mode"
                },
                "type": {
                    "type": "string",
                    "enum": ["auto", "fast", "deep"],
                    "description": "Search type"
                }
            },
            "required": ["query"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: WebSearchParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid websearch parameters: {}", e))?;

            let api_key = std::env::var("EXA_API_KEY").ok();

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(25))
                .build()?;

            let mut body = json!({
                "query": params.query,
                "type": params.search_type.unwrap_or_else(|| "auto".to_string()),
                "numResults": params.numResults,
            });

            if let Some(livecrawl) = &params.livecrawl {
                body["livecrawl"] = json!(livecrawl);
            }

            let response = if let Some(key) = api_key {
                client
                    .post("https://api.exa.ai/search")
                    .header("Authorization", format!("Bearer {}", key))
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await?
            } else {
                return Ok(ToolResult::text("Web search requires EXA_API_KEY environment variable. Please set it to enable web search."));
            };

            let results: serde_json::Value = response.json().await?;

            let output =
                if let Some(results_arr) = results.get("results").and_then(|r| r.as_array()) {
                    let mut text = String::new();
                    for (i, result) in results_arr.iter().enumerate() {
                        let title = result.get("title").and_then(|t| t.as_str()).unwrap_or("");
                        let url = result.get("url").and_then(|u| u.as_str()).unwrap_or("");
                        let content = result.get("text").and_then(|c| c.as_str()).unwrap_or("");

                        text.push_str(&format!("{}. {}\n{}\n{}\n\n", i + 1, title, url, content));
                    }
                    if text.is_empty() {
                        "No search results found.".to_string()
                    } else {
                        text
                    }
                } else {
                    "No search results found. Please try a different query.".to_string()
                };

            Ok(ToolResult::with_metadata(
                output,
                json!({
                    "query": params.query,
                    "numResults": params.numResults,
                    "provider": "exa",
                }),
            ))
        })
    }
}
