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

const API_URL: &str = "https://api.groq.com/openai/v1/chat/completions";

pub struct GroqProvider {
    client: Client,
    api_key: String,
}

impl GroqProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("GROQ_API_KEY").map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }

    fn build_messages(&self, request: &CompletionRequest) -> Vec<GroqMessage> {
        request
            .messages
            .iter()
            .map(|msg| GroqMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
                tool_calls: msg.tool_calls.clone(),
                tool_call_id: msg.tool_call_id.clone(),
            })
            .collect()
    }

    fn build_request(&self, request: &CompletionRequest, stream: bool) -> GroqRequest {
        GroqRequest {
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
struct GroqMessage {
    role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct GroqRequest {
    model: String,
    messages: Vec<GroqMessage>,
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
struct GroqResponse {
    model: String,
    choices: Vec<GroqChoice>,
    usage: GroqUsage,
}

#[derive(Deserialize)]
struct GroqChoice {
    message: GroqResponseMessage,
    finish_reason: String,
}

#[derive(Deserialize)]
struct GroqResponseMessage {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<GroqToolCallResponse>>,
}

#[derive(Deserialize)]
struct GroqToolCallResponse {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: GroqFunctionResponse,
}

#[derive(Deserialize)]
struct GroqFunctionResponse {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct GroqUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

use lazy_static::lazy_static;

lazy_static! {
    static ref GROQ_MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("llama-3.3-70b-versatile")),
            name: Some("Llama 3.3 70B Versatile".to_string()),
            family: Some("llama".to_string()),
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: None,
                cache_write: None,
                context_over_200k: None
            }),
            limit: Some(ModelLimit {
                context: 8192.0,
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
            id: Some(ModelID::new("llama-3.1-8b-instant")),
            name: Some("Llama 3.1 8B Instant".to_string()),
            family: Some("llama".to_string()),
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: None,
                cache_write: None,
                context_over_200k: None
            }),
            limit: Some(ModelLimit {
                context: 8192.0,
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
            id: Some(ModelID::new("mixtral-8x7b-32768")),
            name: Some("Mixtral 8x7B".to_string()),
            family: Some("mixtral".to_string()),
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: None,
                cache_write: None,
                context_over_200k: None
            }),
            limit: Some(ModelLimit {
                context: 32768.0,
                input: None,
                output: 32768.0
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
            id: Some(ModelID::new("gemma2-9b-it")),
            name: Some("Gemma 2 9B".to_string()),
            family: Some("gemma".to_string()),
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: None,
                cache_write: None,
                context_over_200k: None
            }),
            limit: Some(ModelLimit {
                context: 8192.0,
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
impl Provider for GroqProvider {
    fn name(&self) -> &str {
        "groq"
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let groq_req = self.build_request(&request, false);

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&groq_req)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await?;
            return Err(ProviderError::api(status, body));
        }

        let groq_resp: GroqResponse = response.json().await?;

        let content = groq_resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let tool_calls: Vec<ToolCall> = groq_resp
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
                groq_resp
                    .choices
                    .first()
                    .map(|c| c.finish_reason.clone())
                    .unwrap_or_default(),
            ),
            usage: TokenUsage {
                input: groq_resp.usage.prompt_tokens,
                output: groq_resp.usage.completion_tokens,
                cache_read: None,
                cache_write: None,
            },
            model: groq_resp.model,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        let groq_req = self.build_request(&request, true);
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        let stream = async_stream::try_stream! {
            let response = client
                .post(API_URL)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&groq_req)
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
        &GROQ_MODELS
    }

    fn default_model(&self) -> Option<&ModelInfo> {
        GROQ_MODELS.first()
    }
}

impl GroqProvider {
    fn parse_sse_event(data: &str) -> Result<StreamEvent, serde_json::Error> {
        #[derive(Debug, Deserialize)]
        struct GroqSseResponse {
            id: Option<String>,
            choices: Vec<GroqSseChoice>,
        }

        #[derive(Debug, Deserialize)]
        struct GroqSseChoice {
            index: u32,
            delta: GroqSseDelta,
            finish_reason: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        struct GroqSseDelta {
            role: Option<String>,
            content: Option<String>,
            tool_calls: Option<Vec<GroqSseToolCall>>,
        }

        #[derive(Debug, Deserialize)]
        struct GroqSseToolCall {
            index: u32,
            id: Option<String>,
            function: Option<GroqSseFunction>,
        }

        #[derive(Debug, Deserialize)]
        struct GroqSseFunction {
            name: Option<String>,
            arguments: Option<String>,
        }

        let response: GroqSseResponse = serde_json::from_str(data)?;

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
