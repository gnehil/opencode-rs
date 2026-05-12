use async_trait::async_trait;
use reqwest::Client;
use lazy_static::lazy_static;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "https://api.openrouter.ai/v1/chat/completions";

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("anthropic/claude-3.5-sonnet")),
            name: Some("Claude 3.5 Sonnet (OpenRouter)".to_string()),
            family: Some("claude".to_string()),
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
            id: Some(ModelID::new("openai/gpt-4o")),
            name: Some("GPT-4o (OpenRouter)".to_string()),
            family: Some("gpt".to_string()),
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

pub struct OpenRouterProvider {
    client: Client,
    api_key: String,
}

impl OpenRouterProvider {
    pub fn new(api_key: String) -> Self {
        Self { client: Client::new(), api_key }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }
}

impl Provider for OpenRouterProvider {
    fn name(&self) -> &str { "openrouter" }

    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }

    fn models(&self) -> &[ModelInfo] { &MODELS }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request.messages.iter().map(|msg| {
            match msg {
                crate::message::Message::User(u) => serde_json::json!({
                    "role": "user",
                    "content": u.summary.as_ref().and_then(|s| s.body.clone()).unwrap_or_default()
                }),
                crate::message::Message::Assistant(_) => serde_json::json!({
                    "role": "assistant",
                    "content": ""
                }),
            }
        }).collect();

        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
        });

        let response = self.client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://opencode.ai")
            .header("X-Title", "OpenCode")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        let data: serde_json::Value = response.json().await
            .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        let content = data["choices"][0]["message"]["content"]
            .as_str().unwrap_or("").to_string();

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            usage: TokenUsage {
                input: data["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as i32,
                output: data["usage"]["completion_tokens"].as_u64().unwrap_or(0) as i32,
                cache_read: None,
                cache_write: None,
            },
            stop_reason: data["choices"][0]["finish_reason"].as_str().unwrap_or("stop").to_string(),
        })
    }

    async fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> {
        Err(ProviderError::StreamNotSupported)
    }
}