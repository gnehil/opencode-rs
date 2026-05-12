use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u64,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContent>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContent {
    Text { text: String },
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicCompleteResponse {
    #[allow(dead_code)]
    id: String,
    model: String,
    content: Vec<AnthropicResponseBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicResponseBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u64,
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    models: Vec<ModelInfo>,
}

impl AnthropicProvider {
    pub fn new(api_key: String, models: Option<Vec<ModelInfo>>) -> Self {
        let models = models.unwrap_or_else(Self::default_models);
        Self {
            client: Client::new(),
            api_key,
            models,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key, None))
    }

    fn default_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: Some(ModelID::new("claude-3-5-sonnet-20241022")),
                name: Some("Claude 3.5 Sonnet".to_string()),
                family: Some("claude-3.5".to_string()),
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
                id: Some(ModelID::new("claude-3-5-haiku-20241022")),
                name: Some("Claude 3.5 Haiku".to_string()),
                family: Some("claude-3.5".to_string()),
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
                id: Some(ModelID::new("claude-3-opus-20240229")),
                name: Some("Claude 3 Opus".to_string()),
                family: Some("claude-3".to_string()),
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
        ]
    }

    fn build_request(&self, request: &CompletionRequest, stream: bool) -> AnthropicRequest {
        let messages: Vec<AnthropicMessage> = request
            .messages
            .iter()
            .map(|msg| AnthropicMessage {
                role: msg.role.clone(),
                content: vec![AnthropicContent::Text { text: msg.content.clone() }],
            })
            .collect();

        let tools: Vec<AnthropicTool> = request
            .tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
            })
            .collect();

        AnthropicRequest {
            model: request.model.as_str().to_string(),
            max_tokens: request.max_tokens.unwrap_or(4096),
            messages,
            system: request.system.clone(),
            tools,
            temperature: request.temperature,
            top_p: request.top_p,
            stop_sequences: request.stop_sequences.clone().unwrap_or_default(),
            stream: if stream { Some(true) } else { None },
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let anthropic_req = self.build_request(&request, false);

        let response = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&anthropic_req)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await?;
            return Err(ProviderError::api(status, body));
        }

        let body: AnthropicCompleteResponse = response.json().await?;

        let content = body
            .content
            .iter()
            .filter_map(|c| match c {
                AnthropicResponseBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let tool_calls: Vec<ToolCall> = body
            .content
            .iter()
            .filter_map(|c| match c {
                AnthropicResponseBlock::ToolUse { id, name, input } => Some(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.to_string(),
                }),
                _ => None,
            })
            .collect();

        Ok(CompletionResponse {
            content,
            tool_calls,
            stop_reason: body.stop_reason,
            usage: TokenUsage {
                input: body.usage.input_tokens,
                output: body.usage.output_tokens,
                cache_read: if body.usage.cache_read_input_tokens > 0 {
                    Some(body.usage.cache_read_input_tokens)
                } else {
                    None
                },
                cache_write: if body.usage.cache_creation_input_tokens > 0 {
                    Some(body.usage.cache_creation_input_tokens)
                } else {
                    None
                },
            },
            model: body.model,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        let anthropic_req = self.build_request(&request, true);
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        let stream = async_stream::try_stream! {
            let response = client
                .post(API_URL)
                .header("x-api-key", &api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .header("accept", "text/event-stream")
                .json(&anthropic_req)
                .send()
                .await?;

            let response = response.error_for_status()
                .map_err(|e| ProviderError::api(e.status().map(|s| s.as_u16()).unwrap_or(0), e.to_string()))?;

            let mut stream_reader = response.bytes_stream();

            let mut buffer = String::new();

            while let Some(chunk) = stream_reader.next().await.transpose()? {
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                let lines: Vec<String> = buffer.split('\n').map(String::from).collect();

                if lines.len() <= 1 {
                    if let Some(last) = lines.last() {
                        buffer = last.clone();
                    }
                    continue;
                }

                buffer = lines.last().cloned().unwrap_or_default();

                for line in &lines[..lines.len() - 1] {
                    let line = line.trim();
                    if line.is_empty() || !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];
                    if data == "[DONE]" {
                        yield StreamEvent::message_stop(
                            "end_turn".to_string(),
                            TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
                        );
                        continue;
                    }

                    if let Ok(event) = Self::parse_sse_event(data) {
                        yield event;
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    fn default_model(&self) -> Option<&ModelInfo> {
        self.models.first()
    }
}

impl AnthropicProvider {
    fn parse_sse_event(data: &str) -> Result<StreamEvent, serde_json::Error> {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct SseEvent {
            #[serde(rename = "type")]
            event_type: String,
            index: Option<u32>,
            delta: Option<SseDelta>,
            content_block: Option<serde_json::Value>,
            usage: Option<serde_json::Value>,
        }

        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct SseDelta {
            #[serde(rename = "type")]
            delta_type: String,
            text: Option<String>,
            partial_json: Option<String>,
        }

        let event: SseEvent = serde_json::from_str(data)?;

        match event.event_type.as_str() {
            "message_start" => Ok(StreamEvent {
                event_type: "message_start".to_string(),
                delta: None,
                tool_call: None,
                stop_reason: None,
                usage: event.usage.and_then(|u| serde_json::from_value(u).ok()),
            }),
            "content_block_start" => Ok(StreamEvent {
                event_type: "content_block_start".to_string(),
                delta: None,
                tool_call: event.content_block.and_then(|cb| {
                    if cb.get("type")?.as_str()? == "tool_use" {
                        Some(ToolCall {
                            id: cb.get("id")?.as_str()?.to_string(),
                            name: cb.get("name")?.as_str()?.to_string(),
                            arguments: cb.get("input")?.to_string(),
                        })
                    } else {
                        None
                    }
                }),
                stop_reason: None,
                usage: None,
            }),
            "content_block_delta" => Ok(StreamEvent {
                event_type: "content_block_delta".to_string(),
                delta: event.delta.and_then(|d| {
                    d.text.or(d.partial_json)
                }),
                tool_call: None,
                stop_reason: None,
                usage: None,
            }),
            "message_delta" => Ok(StreamEvent {
                event_type: "message_delta".to_string(),
                delta: None,
                tool_call: None,
                stop_reason: None,
                usage: event.usage.and_then(|u| serde_json::from_value(u).ok()),
            }),
            _ => Ok(StreamEvent {
                event_type: event.event_type,
                delta: None,
                tool_call: None,
                stop_reason: None,
                usage: None,
            }),
        }
    }
}
