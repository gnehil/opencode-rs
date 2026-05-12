use async_trait::async_trait;
use reqwest::Client;
use lazy_static::lazy_static;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "https://api.cerebras.ai/v1/chat/completions";

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("llama-3.3-70b")),
            name: Some("Llama 3.3 70B (Cerebras)".to_string()),
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
    ];
}

pub struct CerebrasProvider {
    client: Client,
    api_key: String,
}

impl CerebrasProvider {
    pub fn new(api_key: String) -> Self { Self { client: Client::new(), api_key } }
    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("CEREBRAS_API_KEY").map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }
}

#[async_trait]
impl Provider for CerebrasProvider {
    fn name(&self) -> &str { "cerebras" }
    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }
    fn models(&self) -> &[ModelInfo] { &MODELS }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request.messages.iter().map(|msg| serde_json::json!({"role": msg.role, "content": msg.content})).collect();

        let body = serde_json::json!({ "model": model, "messages": messages, "max_tokens": request.max_tokens.unwrap_or(4096) });

        let response = self.client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let data: serde_json::Value = response.json().await.map_err(|e| ProviderError::api(0, e.to_string()))?;

        Ok(CompletionResponse {
            content: data["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string(),
            tool_calls: vec![],
            usage: TokenUsage {
                input: data["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
                output: data["usage"]["completion_tokens"].as_u64().unwrap_or(0),
                cache_read: None, cache_write: None,
            },
            stop_reason: Some(data["choices"][0]["finish_reason"].as_str().unwrap_or("stop").to_string()),
            model: model.clone(),
        })
    }

    fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> { Err(ProviderError::stream("streaming not implemented")) }
}