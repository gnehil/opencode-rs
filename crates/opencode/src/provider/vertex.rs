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

#[async_trait]
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

        let messages: Vec<serde_json::Value> = request.messages.iter().map(crate::provider::openai_compat_message_json).collect();

        let body = serde_json::json!({ "contents": messages, "generationConfig": { "maxOutputTokens": request.max_tokens.unwrap_or(4096) } });

        let response = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let data: serde_json::Value = response.json().await.map_err(|e| ProviderError::api(0, e.to_string()))?;

        let content = data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str().unwrap_or("").to_string();

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            usage: TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
            stop_reason: Some(data["candidates"][0]["finishReason"].as_str().unwrap_or("STOP").to_string()),
            model: model.clone(),
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        // Vertex Gemini's streamGenerateContent endpoint returns SSE (when
        // ?alt=sse is set; otherwise it's a streamed JSON array which is
        // harder to parse incrementally). Each `data:` chunk is a
        // GenerateContentResponse whose candidates[0].content.parts[*].text
        // is the *delta* in stream mode.
        let model = request.model.to_string();
        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:streamGenerateContent?alt=sse",
            self.location, self.project_id, self.location, model
        );
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(crate::provider::openai_compat_message_json)
            .collect();
        let body = serde_json::json!({
            "contents": messages,
            "generationConfig": { "maxOutputTokens": request.max_tokens.unwrap_or(4096) }
        });
        let client = self.client.clone();
        let access_token = self.access_token.clone();

        let stream = async_stream::try_stream! {
            use futures::StreamExt;
            let response = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", access_token))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
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
                    if line.is_empty() || !line.starts_with("data:") {
                        continue;
                    }
                    let data = line.trim_start_matches("data:").trim();
                    let v: serde_json::Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    // Concatenate every text part of the first candidate
                    // (a single chunk usually has one part, but Gemini
                    // can interleave text + thoughts).
                    let mut delta = String::new();
                    if let Some(parts) = v["candidates"][0]["content"]["parts"].as_array() {
                        for part in parts {
                            if let Some(t) = part["text"].as_str() {
                                delta.push_str(t);
                            }
                        }
                    }
                    let finish = v["candidates"][0]["finishReason"].as_str()
                        .filter(|s| !s.is_empty() && *s != "FINISH_REASON_UNSPECIFIED");
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
                        let usage = TokenUsage {
                            input: v["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0),
                            output: v["usageMetadata"]["candidatesTokenCount"].as_u64().unwrap_or(0),
                            cache_read: None,
                            cache_write: None,
                        };
                        yield StreamEvent::message_stop(fr.to_lowercase(), usage);
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}