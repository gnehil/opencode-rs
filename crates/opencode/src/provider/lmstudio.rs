use async_trait::async_trait;
use reqwest::Client;
use lazy_static::lazy_static;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const DEFAULT_URL: &str = "http://localhost:1234/v1/chat/completions";

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("local-model")),
            name: Some("Local Model".to_string()),
            family: Some("local".to_string()),
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
    ];
}

pub struct LMStudioProvider {
    client: Client,
    base_url: String,
}

impl LMStudioProvider {
    pub fn new(base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.unwrap_or(DEFAULT_URL.to_string()),
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let base_url = std::env::var("LMSTUDIO_BASE_URL").ok();
        Ok(Self::new(base_url))
    }
}

#[async_trait]
impl Provider for LMStudioProvider {
    fn name(&self) -> &str { "lmstudio" }
    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }
    fn models(&self) -> &[ModelInfo] { &MODELS }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request.messages.iter().map(crate::provider::openai_compat_message_json).collect();

        let body = serde_json::json!({ "model": model, "messages": messages, "max_tokens": request.max_tokens.unwrap_or(4096) });

        let response = self.client
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let data: serde_json::Value = response.json().await.map_err(|e| ProviderError::api(0, e.to_string()))?;

        Ok(CompletionResponse {
            content: data["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string(),
            tool_calls: vec![],
            usage: TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
            stop_reason: Some("stop".to_string()),
            model: model.clone(),
        })
    }

    fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> { Err(ProviderError::stream("streaming not implemented")) }
}