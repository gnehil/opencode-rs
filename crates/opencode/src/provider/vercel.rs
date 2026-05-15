use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "https://api.vercel.ai/v1/chat/completions";

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![ModelInfo {
        id: Some(ModelID::new("v0-1.5-md")),
        name: Some("V0 1.5 MD".to_string()),
        family: Some("v0".to_string()),
        reasoning: Some(false),
        tool_call: Some(false),
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
    },];
}

pub struct VercelProvider {
    client: Client,
    api_key: String,
}

impl VercelProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }
    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("VERCEL_API_KEY").map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }
}

#[async_trait]
impl Provider for VercelProvider {
    fn name(&self) -> &str {
        "vercel"
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

        let body = serde_json::json!({ "model": model, "messages": messages, "max_tokens": request.max_tokens.unwrap_or(4096) });

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

        Ok(CompletionResponse {
            content: data["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            tool_calls: vec![],
            usage: TokenUsage {
                input: 0,
                output: 0,
                cache_read: None,
                cache_write: None,
            },
            stop_reason: Some("stop".to_string()),
            model: model.clone(),
            reasoning: None,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
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
        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            format!("Bearer {}", self.api_key),
        );
        crate::provider::openai_sse::stream_openai_sse(
            self.client.clone(),
            API_URL.to_string(),
            headers,
            body,
        )
    }
}
