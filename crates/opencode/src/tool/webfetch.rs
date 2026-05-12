use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use crate::id::{PartID, SessionID, MessageID};

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024;
const DEFAULT_TIMEOUT: u64 = 30;
const MAX_TIMEOUT: u64 = 120;

#[derive(Debug, Deserialize)]
pub struct WebFetchParams {
    pub url: String,
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default)]
    pub timeout: Option<u64>,
}

fn default_format() -> String {
    "markdown".to_string()
}

pub struct WebFetchTool;

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        "Fetches content from a URL. Converts to requested format (markdown by default). Use for retrieving web content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "default": "markdown",
                    "description": "The format to return content in"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (max 120)"
                }
            },
            "required": ["url"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: WebFetchParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid webfetch parameters: {}", e))?;

            if !params.url.starts_with("http://") && !params.url.starts_with("https://") {
                return Err(anyhow::anyhow!("URL must start with http:// or https://"));
            }

            let timeout = std::cmp::min(params.timeout.unwrap_or(DEFAULT_TIMEOUT), MAX_TIMEOUT);

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(timeout))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .build()?;

            let accept = match params.format.as_str() {
                "markdown" => "text/markdown;q=1.0, text/html;q=0.9, text/plain;q=0.8, */*;q=0.1",
                "text" => "text/plain;q=1.0, text/html;q=0.9, */*;q=0.1",
                "html" => "text/html;q=1.0, application/xhtml+xml;q=0.9, */*;q=0.1",
                _ => "text/html,application/xhtml+xml,*/*;q=0.8",
            };

            let response = client
                .get(&params.url)
                .header("Accept", accept)
                .header("Accept-Language", "en-US,en;q=0.9")
                .send()
                .await?;

            let content_length = response.content_length();
            if let Some(len) = content_length {
                if len > MAX_RESPONSE_SIZE as u64 {
                    return Err(anyhow::anyhow!("Response too large (exceeds 5MB limit)"));
                }
            }

            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let body = response.bytes().await?;
            if body.len() > MAX_RESPONSE_SIZE {
                return Err(anyhow::anyhow!("Response too large (exceeds 5MB limit)"));
            }

            let mime = content_type.split(';').next().unwrap_or("").trim();
            if mime.starts_with("image/") {
                let base64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body);
                let file_part = crate::message::part::FilePart {
                    id: PartID::new(),
                    session_id: SessionID::new(),
                    message_id: MessageID::new(),
                    mime: mime.to_string(),
                    filename: None,
                    url: format!("data:{};base64,{}", mime, base64),
                    source: None,
                };
                return Ok(ToolResult::with_attachments("Image fetched successfully", vec![file_part]));
            }

            let text = String::from_utf8_lossy(&body).to_string();

            let output = match params.format.as_str() {
                "markdown" if content_type.contains("text/html") => {
                    html_to_markdown(&text)
                }
                "text" if content_type.contains("text/html") => {
                    extract_text_from_html(&text)
                }
                _ => text,
            };

            Ok(ToolResult::with_metadata(
                output,
                json!({
                    "url": params.url,
                    "format": params.format,
                    "content_type": content_type,
                }),
            ))
        })
    }
}

fn html_to_markdown(html: &str) -> String {
    let mut result = html.to_string();
    
    let script_re = regex::Regex::new(r"<script[^>]*>.*?</script>").unwrap();
    let style_re = regex::Regex::new(r"<style[^>]*>.*?</style>").unwrap();
    result = script_re.replace_all(&result, "").to_string();
    result = style_re.replace_all(&result, "").to_string();
    
    let h1_re = regex::Regex::new(r"<h1[^>]*>(.*?)</h1>").unwrap();
    let h2_re = regex::Regex::new(r"<h2[^>]*>(.*?)</h2>").unwrap();
    let h3_re = regex::Regex::new(r"<h3[^>]*>(.*?)</h3>").unwrap();
    result = h1_re.replace_all(&result, "# $1").to_string();
    result = h2_re.replace_all(&result, "## $1").to_string();
    result = h3_re.replace_all(&result, "### $1").to_string();
    
    let link_re = regex::Regex::new(r#"<a[^>]*href="([^"]*)"[^>]*>(.*?)</a>"#).unwrap();
    result = link_re.replace_all(&result, "[$2]($1)").to_string();
    
    let bold_re = regex::Regex::new(r"<(b|strong)[^>]*>(.*?)</(b|strong)>").unwrap();
    let italic_re = regex::Regex::new(r"<(i|em)[^>]*>(.*?)</(i|em)>").unwrap();
    result = bold_re.replace_all(&result, "**$2**").to_string();
    result = italic_re.replace_all(&result, "*$2*").to_string();
    
    let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
    result = tag_re.replace_all(&result, "").to_string();
    
    result.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn extract_text_from_html(html: &str) -> String {
    let mut result = html.to_string();
    
    let remove_re = regex::Regex::new(r"<(script|style|noscript|iframe|object|embed)[^>]*>.*?</(script|style|noscript|iframe|object|embed)>").unwrap();
    result = remove_re.replace_all(&result, "").to_string();
    
    let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
    result = tag_re.replace_all(&result, "").to_string();
    
    result.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}