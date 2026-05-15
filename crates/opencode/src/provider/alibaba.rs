use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/text-generation/generation";

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("qwen-max")),
            name: Some("Qwen Max".to_string()),
            family: Some("qwen".to_string()),
            reasoning: Some(false),
            tool_call: Some(true),
            attachment: Some(true),
            temperature: Some(true),
            interleaved: None,
            cost: None,
            limit: None,
            modalities: None,
            experimental: None,
            release_date: None,
            status: Some("active".to_string()),
            provider: None,
            options: None,
            headers: None,
            variants: None,
        },
        ModelInfo {
            id: Some(ModelID::new("qwen-turbo")),
            name: Some("Qwen Turbo".to_string()),
            family: Some("qwen".to_string()),
            reasoning: Some(false),
            tool_call: Some(true),
            attachment: Some(false),
            temperature: Some(true),
            interleaved: None,
            cost: None,
            limit: None,
            modalities: None,
            experimental: None,
            release_date: None,
            status: Some("active".to_string()),
            provider: None,
            options: None,
            headers: None,
            variants: None,
        },
    ];
}

pub struct AlibabaProvider {
    client: Client,
    api_key: String,
}

impl AlibabaProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }
    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("ALIBABA_API_KEY")
            .or_else(|_| std::env::var("DASHSCOPE_API_KEY"))
            .map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }
}

#[async_trait]
impl Provider for AlibabaProvider {
    fn name(&self) -> &str {
        "alibaba"
    }
    fn default_model(&self) -> Option<&ModelInfo> {
        MODELS.first()
    }
    fn models(&self) -> &[ModelInfo] {
        &MODELS
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|msg| serde_json::json!({"role": msg.role, "content": msg.content}))
            .collect();

        let body = serde_json::json!({
            "model": model,
            "input": { "messages": messages },
            "parameters": { "max_tokens": request.max_tokens.unwrap_or(4096) }
        });

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(ProviderError::api(status.as_u16(), text));
        }

        let data: serde_json::Value = response.json().await?;

        Ok(CompletionResponse {
            content: data["output"]["text"].as_str().unwrap_or("").to_string(),
            tool_calls: vec![],
            usage: TokenUsage {
                input: 0,
                output: 0,
                cache_read: None,
                cache_write: None,
            },
            stop_reason: Some("stop".to_string()),
            model,
            reasoning: None,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        // DashScope streaming: SSE wire format, but opt-in via the
        // `X-DashScope-SSE: enable` request header. Each `data:` line is
        // a full response document with the cumulative `output.text`
        // (NOT a delta!), so we keep a "last text seen" cursor and yield
        // only the suffix as a delta. The final frame carries
        // `output.finish_reason`.
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|msg| serde_json::json!({"role": msg.role, "content": msg.content}))
            .collect();
        let body = serde_json::json!({
            "model": model,
            "input": { "messages": messages },
            "parameters": {
                "max_tokens": request.max_tokens.unwrap_or(4096),
                "incremental_output": true,
            }
        });
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        let stream = async_stream::try_stream! {
            use futures::StreamExt;
            let response = client
                .post(API_URL)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .header("X-DashScope-SSE", "enable")
                .header("Accept", "text/event-stream")
                .json(&body)
                .send()
                .await?;
            let response = response.error_for_status()
                .map_err(|e| ProviderError::api(e.status().map(|s| s.as_u16()).unwrap_or(0), e.to_string()))?;
            let mut bytes = response.bytes_stream();
            let mut buffer = String::new();
            // With incremental_output: true, output.text is already a
            // delta, not cumulative. So we just yield it as-is.
            while let Some(chunk) = bytes.next().await.transpose()? {
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(nl) = buffer.find('\n') {
                    let line: String = buffer.drain(..=nl).collect();
                    let line = line.trim();
                    if line.is_empty() || !line.starts_with("data:") {
                        continue;
                    }
                    let data = line.trim_start_matches("data:").trim();
                    let v: serde_json::Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let delta = v["output"]["text"].as_str().unwrap_or("").to_string();
                    let finish = v["output"]["finish_reason"].as_str();
                    if !delta.is_empty() {
                        yield StreamEvent {
                            event_type: "content_block_delta".to_string(),
                            delta: Some(delta),
                            tool_call: None,
                            stop_reason: None,
                            usage: None,
                        };
                    }
                    if let Some(fr) = finish {
                        if fr != "null" {
                            let usage = TokenUsage {
                                input: v["usage"]["input_tokens"].as_u64().unwrap_or(0),
                                output: v["usage"]["output_tokens"].as_u64().unwrap_or(0),
                                cache_read: None,
                                cache_write: None,
                            };
                            yield StreamEvent::message_stop(fr.to_string(), usage);
                        }
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}
