use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

const API_URL: &str = "https://api.openai.com/v1/chat/completions";

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAITool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct OpenAIMessage {
    role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct OpenAITool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAIFunction,
}

#[derive(Debug, Serialize)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAICompleteResponse {
    id: String,
    model: String,
    choices: Vec<OpenAIChoice>,
    usage: OpenAIUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    index: u32,
    message: OpenAIMessageResponse,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIMessageResponse {
    role: String,
    content: Option<String>,
    tool_calls: Option<Vec<OpenAIToolCallResponse>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolCallResponse {
    id: String,
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAIFunctionResponse,
}

#[derive(Debug, Deserialize)]
struct OpenAIFunctionResponse {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    models: Vec<ModelInfo>,
}

impl OpenAIProvider {
    pub fn new(api_key: String, models: Option<Vec<ModelInfo>>) -> Self {
        let models = models.unwrap_or_else(Self::default_models);
        Self {
            client: Client::new(),
            api_key,
            models,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key, None))
    }

    fn default_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: Some(ModelID::new("gpt-4o")),
                name: Some("GPT-4o".to_string()),
                family: Some("gpt-4".to_string()),
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
                id: Some(ModelID::new("gpt-4o-mini")),
                name: Some("GPT-4o Mini".to_string()),
                family: Some("gpt-4".to_string()),
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
                id: Some(ModelID::new("gpt-4-turbo")),
                name: Some("GPT-4 Turbo".to_string()),
                family: Some("gpt-4".to_string()),
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
                id: Some(ModelID::new("o1-preview")),
                name: Some("o1 Preview".to_string()),
                family: Some("o1".to_string()),
                reasoning: Some(true),
                tool_call: Some(false),
                attachment: Some(false),
                temperature: Some(false),
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
                id: Some(ModelID::new("o1-mini")),
                name: Some("o1 Mini".to_string()),
                family: Some("o1".to_string()),
                reasoning: Some(true),
                tool_call: Some(false),
                attachment: Some(false),
                temperature: Some(false),
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

    fn build_request(&self, request: &CompletionRequest, stream: bool) -> OpenAIRequest {
        let messages: Vec<OpenAIMessage> = request
            .messages
            .iter()
            .map(|msg| OpenAIMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
                tool_calls: msg.tool_calls.clone(),
                tool_call_id: msg.tool_call_id.clone(),
            })
            .collect();

        let tools: Vec<OpenAITool> = request
            .tools
            .iter()
            .map(|t| OpenAITool {
                tool_type: "function".to_string(),
                function: OpenAIFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect();

        OpenAIRequest {
            model: request.model.as_str().to_string(),
            max_tokens: request.max_tokens,
            messages,
            tools,
            temperature: request.temperature,
            top_p: request.top_p,
            stop: request.stop_sequences.clone().unwrap_or_default(),
            stream: if stream { Some(true) } else { None },
        }
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let openai_req = self.build_request(&request, false);

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&openai_req)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await?;
            return Err(ProviderError::api(status, body));
        }

        let body: OpenAICompleteResponse = response.json().await?;

        let choice = body.choices.first();
        let content = choice
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();

        let tool_calls: Vec<ToolCall> = choice
            .and_then(|c| c.message.tool_calls.as_ref())
            .map(|tc| tc.iter().map(|t| ToolCall {
                id: t.id.clone(),
                name: t.function.name.clone(),
                arguments: t.function.arguments.clone(),
            }).collect())
            .unwrap_or_default();

        let finish_reason = choice.and_then(|c| c.finish_reason.clone());

        Ok(CompletionResponse {
            content,
            tool_calls,
            stop_reason: finish_reason,
            usage: TokenUsage {
                input: body.usage.prompt_tokens,
                output: body.usage.completion_tokens,
                cache_read: None,
                cache_write: None,
            },
            model: body.model,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        let openai_req = self.build_request(&request, true);
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        let stream = async_stream::try_stream! {
            let response = client
                .post(API_URL)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&openai_req)
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
        &self.models
    }

    fn default_model(&self) -> Option<&ModelInfo> {
        self.models.first()
    }
}

impl OpenAIProvider {
    fn parse_sse_event(data: &str) -> Result<StreamEvent, serde_json::Error> {
        #[derive(Debug, Deserialize)]
        struct OpenAISseResponse {
            id: Option<String>,
            choices: Vec<OpenAISseChoice>,
        }

        #[derive(Debug, Deserialize)]
        struct OpenAISseChoice {
            index: u32,
            delta: OpenAISseDelta,
            finish_reason: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        struct OpenAISseDelta {
            role: Option<String>,
            content: Option<String>,
            tool_calls: Option<Vec<OpenAISseToolCall>>,
        }

        #[derive(Debug, Deserialize)]
        struct OpenAISseToolCall {
            index: u32,
            id: Option<String>,
            function: Option<OpenAISseFunction>,
        }

        #[derive(Debug, Deserialize)]
        struct OpenAISseFunction {
            name: Option<String>,
            arguments: Option<String>,
        }

        let response: OpenAISseResponse = serde_json::from_str(data)?;

        let choice = response.choices.first();
        let delta_content = choice.and_then(|c| c.delta.content.clone());
        let finish_reason = choice.and_then(|c| c.finish_reason.clone());

        let tool_call = choice.and_then(|c| c.delta.tool_calls.as_ref())
            .and_then(|tc| tc.first())
            .and_then(|t| {
                Some(ToolCall {
                    id: t.id.clone()?,
                    name: t.function.as_ref()?.name.clone()?,
                    arguments: t.function.as_ref()?.arguments.clone()?,
                })
            });

        Ok(StreamEvent {
            event_type: if finish_reason.is_some() { "message_stop" } else { "content_block_delta" }.to_string(),
            delta: delta_content,
            tool_call,
            stop_reason: finish_reason,
            usage: None,
        })
    }
}