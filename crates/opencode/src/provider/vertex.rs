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
            id: Some(ModelID::new("gemini-2.0-flash-exp")),
            name: Some("Gemini 2.0 Flash (Vertex)".to_string()),
            family: Some("gemini".to_string()),
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

pub struct VertexProvider {
    client: Client,
    project_id: String,
    location: String,
    access_token: String,
}

impl VertexProvider {
    pub fn new(project_id: String, location: String, access_token: String) -> Self {
        Self { client: Client::new(), project_id, location, access_token }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let project_id = std::env::var("GOOGLE_PROJECT_ID")
            .or_else(|_| std::env::var("GCP_PROJECT_ID"))
            .map_err(|_| ProviderError::MissingApiKey)?;
        let location = std::env::var("GOOGLE_LOCATION").unwrap_or_else(|_| "us-central1".to_string());
        let access_token = std::env::var("GOOGLE_ACCESS_TOKEN")
            .map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(project_id, location, access_token))
    }
}

impl Provider for VertexProvider {
    fn name(&self) -> &str { "google-vertex" }
    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }
    fn models(&self) -> &[ModelInfo] { &MODELS }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model = request.model.to_string();
        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
            self.location, self.project_id, self.location, model
        );

        let messages: Vec<serde_json::Value> = request.messages.iter().map(|msg| match msg {
            crate::message::Message::User(u) => serde_json::json!({
                "role": "user",
                "parts": [{"text": u.summary.as_ref().and_then(|s| s.body.clone()).unwrap_or_default()}]
            }),
            crate::message::Message::Assistant(_) => serde_json::json!({ "role": "model", "parts": [{"text": ""}] }),
        }).collect();

        let body = serde_json::json!({ "contents": messages, "generationConfig": { "maxOutputTokens": request.max_tokens.unwrap_or(4096) } });

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        let data: serde_json::Value = response.json().await.map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        let content = data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str().unwrap_or("").to_string();

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            usage: TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
            stop_reason: data["candidates"][0]["finishReason"].as_str().unwrap_or("STOP").to_string(),
        })
    }

    async fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> { Err(ProviderError::StreamNotSupported) }
}