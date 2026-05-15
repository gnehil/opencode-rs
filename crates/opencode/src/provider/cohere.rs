use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "https://api.cohere.ai/v1/chat";

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("command-r")),
            name: Some("Command R".to_string()),
            family: Some("cohere".to_string()),
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
            id: Some(ModelID::new("command-r-plus")),
            name: Some("Command R+".to_string()),
            family: Some("cohere".to_string()),
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

pub struct CohereProvider {
    client: Client,
    api_key: String,
}

impl CohereProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("COHERE_API_KEY").map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }
}

#[async_trait]
impl Provider for CohereProvider {
    fn name(&self) -> &str {
        "cohere"
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
            "max_tokens": request.max_tokens.unwrap_or(4096),
        });

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let content = data["text"].as_str().unwrap_or("").to_string();

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            usage: TokenUsage {
                input: data["meta"]["tokens"]["input_tokens"].as_u64().unwrap_or(0),
                output: data["meta"]["tokens"]["output_tokens"]
                    .as_u64()
                    .unwrap_or(0),
                cache_read: None,
                cache_write: None,
            },
            stop_reason: Some("stop".to_string()),
            model: model.clone(),
            reasoning: None,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        // Cohere /v1/chat streams NDJSON with `event_type`-discriminated
        // chunks:
        //   {"event_type":"stream-start","generation_id":"..."}
        //   {"event_type":"text-generation","text":"hi"}
        //   {"event_type":"stream-end","finish_reason":"COMPLETE","response":{...}}
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(crate::provider::openai_compat_message_json)
            .collect();
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
            "stream": true,
        });
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        let stream = async_stream::try_stream! {
            use futures::StreamExt;
            let response = client
                .post(API_URL)
                .header("Authorization", format!("Bearer {}", api_key))
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
                    match v["event_type"].as_str() {
                        Some("text-generation") => {
                            let delta = v["text"].as_str().unwrap_or("").to_string();
                            if !delta.is_empty() {
                                yield StreamEvent {
                                    event_type: "content_block_delta".to_string(),
                                    delta: Some(delta),
                                    tool_call: None,
                                    stop_reason: None,
                                    usage: None,
                                };
                            }
                        }
                        Some("stream-end") => {
                            let usage = TokenUsage {
                                input: v["response"]["meta"]["tokens"]["input_tokens"].as_u64().unwrap_or(0),
                                output: v["response"]["meta"]["tokens"]["output_tokens"].as_u64().unwrap_or(0),
                                cache_read: None,
                                cache_write: None,
                            };
                            let stop = v["finish_reason"].as_str().unwrap_or("stop").to_lowercase();
                            yield StreamEvent::message_stop(stop, usage);
                        }
                        // stream-start, tool-calls-generation, etc.: drop.
                        _ => {}
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}
