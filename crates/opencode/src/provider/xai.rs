use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::cost::ModelCost;
use super::id::ModelID;
use super::limit::ModelLimit;
use super::model::ModelInfo;
use super::request::{CompletionRequest, ToolDefinition};
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "https://api.x.ai/v1/chat/completions";

pub struct XAIProvider {
    client: Client,
    api_key: String,
}

impl XAIProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("XAI_API_KEY").map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }

    fn build_messages(&self, request: &CompletionRequest) -> Vec<XAIMessage> {
        request
            .messages
            .iter()
            .map(|msg| XAIMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
                tool_calls: msg.tool_calls.clone(),
                tool_call_id: msg.tool_call_id.clone(),
            })
            .collect()
    }

    fn build_request(&self, request: &CompletionRequest, stream: bool) -> XAIRequest {
        XAIRequest {
            model: request.model.to_string(),
            messages: self.build_messages(request),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(request.tools.clone())
            },
            stream: if stream { Some(true) } else { None },
        }
    }
}

#[derive(Serialize)]
struct XAIMessage {
    role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct XAIRequest {
    model: String,
    messages: Vec<XAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Deserialize)]
struct XAIResponse {
    model: String,
    choices: Vec<XAIChoice>,
    usage: XAIUsage,
}

#[derive(Deserialize)]
struct XAIChoice {
    message: XAIResponseMessage,
    finish_reason: String,
}

#[derive(Deserialize)]
struct XAIResponseMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<XAIToolCallResponse>>,
}

#[derive(Deserialize)]
struct XAIToolCallResponse {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: XAIFunctionResponse,
}

#[derive(Deserialize)]
struct XAIFunctionResponse {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct XAIUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

use lazy_static::lazy_static;

lazy_static! {
    static ref XAI_MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("grok-beta")),
            name: Some("Grok Beta".to_string()),
            family: None,
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 5.0,
                output: 15.0,
                cache_read: None,
                cache_write: None,
                context_over_200k: None
            }),
            limit: Some(ModelLimit {
                context: 131072.0,
                input: None,
                output: 8192.0
            }),
            modalities: None,
            experimental: None,
            status: None,
            provider: None,
            options: None,
            headers: None,
            variants: None,
        },
        ModelInfo {
            id: Some(ModelID::new("grok-2-latest")),
            name: Some("Grok 2 Latest".to_string()),
            family: None,
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 2.0,
                output: 10.0,
                cache_read: None,
                cache_write: None,
                context_over_200k: None
            }),
            limit: Some(ModelLimit {
                context: 131072.0,
                input: None,
                output: 8192.0
            }),
            modalities: None,
            experimental: None,
            status: None,
            provider: None,
            options: None,
            headers: None,
            variants: None,
        },
    ];
}

#[async_trait::async_trait]
impl Provider for XAIProvider {
    fn name(&self) -> &str {
        "xai"
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let xai_req = self.build_request(&request, false);

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&xai_req)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await?;
            return Err(ProviderError::api(status, body));
        }

        let raw: serde_json::Value = response.json().await?;
        let reasoning = crate::provider::extract_openai_compat_reasoning(&raw);
        let xai_resp: XAIResponse = serde_json::from_value(raw)
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let content = xai_resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let tool_calls: Vec<ToolCall> = xai_resp
            .choices
            .first()
            .and_then(|c| c.message.tool_calls.as_ref())
            .map(|tc| {
                tc.iter()
                    .map(|t| ToolCall {
                        id: t.id.clone(),
                        name: t.function.name.clone(),
                        arguments: t.function.arguments.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(CompletionResponse {
            content,
            tool_calls,
            stop_reason: Some(
                xai_resp
                    .choices
                    .first()
                    .map(|c| c.finish_reason.clone())
                    .unwrap_or_default(),
            ),
            usage: TokenUsage {
                input: xai_resp.usage.prompt_tokens,
                output: xai_resp.usage.completion_tokens,
                cache_read: None,
                cache_write: None,
            },
            model: xai_resp.model,
            reasoning,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        let xai_req = self.build_request(&request, true);
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        let stream = async_stream::try_stream! {
            let response = client
                .post(API_URL)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&xai_req)
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
                            "stop".to_string(),
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
        &XAI_MODELS
    }

    fn default_model(&self) -> Option<&ModelInfo> {
        XAI_MODELS.first()
    }
}

impl XAIProvider {
    fn parse_sse_event(data: &str) -> Result<StreamEvent, serde_json::Error> {
        #[derive(Debug, Deserialize)]
        struct XAISseResponse {
            id: Option<String>,
            choices: Vec<XAISseChoice>,
        }

        #[derive(Debug, Deserialize)]
        struct XAISseChoice {
            index: u32,
            delta: XAISseDelta,
            finish_reason: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        struct XAISseDelta {
            role: Option<String>,
            content: Option<String>,
            tool_calls: Option<Vec<XAISseToolCall>>,
        }

        #[derive(Debug, Deserialize)]
        struct XAISseToolCall {
            index: u32,
            id: Option<String>,
            function: Option<XAISseFunction>,
        }

        #[derive(Debug, Deserialize)]
        struct XAISseFunction {
            name: Option<String>,
            arguments: Option<String>,
        }

        let response: XAISseResponse = serde_json::from_str(data)?;

        let choice = response.choices.first();
        let delta_content = choice.and_then(|c| c.delta.content.clone());
        let finish_reason = choice.and_then(|c| c.finish_reason.clone());

        let tool_call = choice
            .and_then(|c| c.delta.tool_calls.as_ref())
            .and_then(|tc| tc.first())
            .and_then(|t| {
                Some(ToolCall {
                    id: t.id.clone()?,
                    name: t.function.as_ref()?.name.clone()?,
                    arguments: t.function.as_ref()?.arguments.clone()?,
                })
            });

        Ok(StreamEvent {
            event_type: if finish_reason.is_some() {
                "message_stop"
            } else {
                "content_block_delta"
            }
            .to_string(),
            delta: delta_content,
            tool_call,
            stop_reason: finish_reason,
            usage: None,
        })
    }
}
