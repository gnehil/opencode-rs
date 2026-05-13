use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "http://localhost:11434/api/chat";

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("llama3.2")),
            name: Some("Llama 3.2".to_string()),
            family: Some("llama".to_string()),
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
            id: Some(ModelID::new("codellama")),
            name: Some("Code Llama".to_string()),
            family: Some("llama".to_string()),
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
        ModelInfo {
            id: Some(ModelID::new("mistral")),
            name: Some("Mistral".to_string()),
            family: Some("mistral".to_string()),
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

pub struct OllamaProvider {
    client: Client,
    base_url: String,
}

impl OllamaProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.unwrap_or(API_URL.to_string()),
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let base_url = std::env::var("OLLAMA_BASE_URL").ok();
        Ok(Self::new(base_url))
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
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
            .map(crate::provider::openai_compat_message_json)
            .collect();

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
        });

        let response = self
            .client
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let content = data["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            usage: TokenUsage {
                input: 0,
                output: 0,
                cache_read: None,
                cache_write: None,
            },
            stop_reason: Some("stop".to_string()),
            model: model.clone(),
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        // Ollama's /api/chat streams NDJSON (newline-delimited JSON), not
        // SSE. Each line is a full chat-response object with a `message`
        // field whose `content` field is the incremental token(s) and a
        // top-level `done: bool`.
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(crate::provider::openai_compat_message_json)
            .collect();
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });
        let client = self.client.clone();
        let url = self.base_url.clone();

        let stream = async_stream::try_stream! {
            use futures::StreamExt;
            let response = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;
            let response = response.error_for_status()
                .map_err(|e| ProviderError::api(e.status().map(|s| s.as_u16()).unwrap_or(0), e.to_string()))?;
            let mut bytes = response.bytes_stream();
            let mut buffer = String::new();
            while let Some(chunk) = bytes.next().await.transpose()? {
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(nl) = buffer.find('\n') {
                    let line: String = buffer.drain(..=nl).collect();
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let v: serde_json::Value = match serde_json::from_str(line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let delta = v["message"]["content"].as_str().unwrap_or("").to_string();
                    let done = v["done"].as_bool().unwrap_or(false);
                    if done {
                        // Ollama reports cumulative tokens in the final
                        // frame; surface them as input/output for parity
                        // with other providers.
                        let usage = TokenUsage {
                            input: v["prompt_eval_count"].as_u64().unwrap_or(0),
                            output: v["eval_count"].as_u64().unwrap_or(0),
                            cache_read: None,
                            cache_write: None,
                        };
                        yield StreamEvent::message_stop("stop".to_string(), usage);
                    } else if !delta.is_empty() {
                        yield StreamEvent {
                            event_type: "content_block_delta".to_string(),
                            delta: Some(delta),
                            tool_call: None,
                            stop_reason: None,
                            usage: None,
                        };
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}
