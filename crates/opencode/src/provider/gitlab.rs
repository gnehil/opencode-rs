use async_trait::async_trait;
use reqwest::Client;
use lazy_static::lazy_static;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("code-assistant")),
            name: Some("GitLab Code Assistant".to_string()),
            family: Some("gitlab".to_string()),
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

pub struct GitLabProvider {
    client: Client,
    base_url: String,
    token: String,
}

impl GitLabProvider {
    pub fn new(base_url: Option<String>, token: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.unwrap_or_else(|| "https://gitlab.com".to_string()),
            token,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let base_url = std::env::var("GITLAB_BASE_URL").ok();
        let token = std::env::var("GITLAB_TOKEN")
            .map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(base_url, token))
    }
}

impl Provider for GitLabProvider {
    fn name(&self) -> &str { "gitlab" }
    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }
    fn models(&self) -> &[ModelInfo] { &MODELS }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let url = format!("{}/api/v4/chat/completions", self.base_url);
        let model = request.model.to_string();
        let messages: Vec<serde_json::Value> = request.messages.iter().map(|msg| match msg {
            crate::message::Message::User(u) => serde_json::json!({
                "role": "user", "content": u.summary.as_ref().and_then(|s| s.body.clone()).unwrap_or_default()
            }),
            crate::message::Message::Assistant(_) => serde_json::json!({ "role": "assistant", "content": "" }),
        }).collect();

        let body = serde_json::json!({ "model": model, "messages": messages, "max_tokens": request.max_tokens.unwrap_or(4096) });

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        let data: serde_json::Value = response.json().await.map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        Ok(CompletionResponse {
            content: data["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string(),
            tool_calls: vec![],
            usage: TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
            stop_reason: "stop".to_string(),
        })
    }

    async fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> { Err(ProviderError::StreamNotSupported) }
}