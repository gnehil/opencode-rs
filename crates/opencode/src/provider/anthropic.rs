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
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
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
        let messages = convert_messages(&request.messages);

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

/// Convert the provider-neutral `CompletionMessage` list into Anthropic's
/// nested-content-block format.
///
/// Anthropic only has `user` and `assistant` roles; tool_use lives in
/// assistant content, tool_result lives in user content. Consecutive
/// tool_result entries are coalesced into a single user message because
/// Anthropic rejects multiple user messages in a row.
fn convert_messages(messages: &[crate::provider::CompletionMessage]) -> Vec<AnthropicMessage> {
    let mut out: Vec<AnthropicMessage> = Vec::new();

    let push_user = |out: &mut Vec<AnthropicMessage>, content: Vec<AnthropicContent>| {
        if content.is_empty() {
            return;
        }
        // Coalesce with the previous user message if it was just a
        // tool_result run; otherwise push a new entry.
        if let Some(last) = out.last_mut() {
            if last.role == "user" {
                last.content.extend(content);
                return;
            }
        }
        out.push(AnthropicMessage { role: "user".to_string(), content });
    };

    for msg in messages {
        match msg.role.as_str() {
            "tool" => {
                // OpenAI-shaped tool result: convert to Anthropic tool_result
                // and attach to the (coalesced) user message.
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                push_user(
                    &mut out,
                    vec![AnthropicContent::ToolResult {
                        tool_use_id,
                        content: msg.content.clone(),
                        is_error: None,
                    }],
                );
            }
            "assistant" => {
                let mut content: Vec<AnthropicContent> = Vec::new();
                if !msg.content.is_empty() {
                    content.push(AnthropicContent::Text { text: msg.content.clone() });
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let func = tc.get("function").cloned().unwrap_or(serde_json::Value::Null);
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let raw_args = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
                        let input: serde_json::Value =
                            serde_json::from_str(raw_args).unwrap_or(serde_json::json!({}));
                        content.push(AnthropicContent::ToolUse { id, name, input });
                    }
                }
                if !content.is_empty() {
                    out.push(AnthropicMessage { role: "assistant".to_string(), content });
                }
            }
            // "user" and any other role we treat as a user text turn.
            _ => {
                push_user(
                    &mut out,
                    vec![AnthropicContent::Text { text: msg.content.clone() }],
                );
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::CompletionMessage;

    fn msg(role: &str, content: &str) -> CompletionMessage {
        CompletionMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn user_assistant_turns_serialize_as_text_blocks() {
        let result = convert_messages(&[
            msg("user", "hi"),
            msg("assistant", "hello"),
        ]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "user");
        assert!(matches!(result[0].content[0], AnthropicContent::Text { .. }));
    }

    #[test]
    fn assistant_tool_call_emits_tool_use_block() {
        let assistant = CompletionMessage {
            role: "assistant".to_string(),
            content: "let me check".to_string(),
            tool_calls: Some(vec![serde_json::json!({
                "id": "toolu_1",
                "type": "function",
                "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"}
            })]),
            tool_call_id: None,
        };
        let result = convert_messages(&[assistant]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.len(), 2);
        assert!(matches!(result[0].content[0], AnthropicContent::Text { .. }));
        match &result[0].content[1] {
            AnthropicContent::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "bash");
                assert_eq!(input["command"], "ls");
            }
            _ => panic!("expected ToolUse"),
        }
    }

    #[test]
    fn tool_role_coalesces_into_user_message() {
        let result = convert_messages(&[
            CompletionMessage {
                role: "tool".to_string(),
                content: "out1".to_string(),
                tool_calls: None,
                tool_call_id: Some("toolu_1".to_string()),
            },
            CompletionMessage {
                role: "tool".to_string(),
                content: "out2".to_string(),
                tool_calls: None,
                tool_call_id: Some("toolu_2".to_string()),
            },
        ]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[0].content.len(), 2);
        match (&result[0].content[0], &result[0].content[1]) {
            (
                AnthropicContent::ToolResult { tool_use_id: id1, .. },
                AnthropicContent::ToolResult { tool_use_id: id2, .. },
            ) => {
                assert_eq!(id1, "toolu_1");
                assert_eq!(id2, "toolu_2");
            }
            _ => panic!("expected both ToolResult"),
        }
    }
}
