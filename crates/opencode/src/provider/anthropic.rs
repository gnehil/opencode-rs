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
    // Always emit as a content-block array so we can attach
    // cache_control to the system prompt. Anthropic accepts either a
    // plain string or this array form; we standardize on the array so
    // the prompt cache picks up on long-stable system prompts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<AnthropicSystemBlock>,
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

/// Cache breakpoint marker. Attaching `cache_control: ephemeral` to a
/// content block tells Anthropic to cache everything in the request up
/// to and including that block; subsequent requests that share the same
/// prefix hit the cache. Up to 4 breakpoints per request.
#[derive(Debug, Serialize, Clone, Copy)]
struct CacheControl {
    #[serde(rename = "type")]
    cache_type: &'static str,
}

const EPHEMERAL: CacheControl = CacheControl {
    cache_type: "ephemeral",
};

#[derive(Debug, Serialize)]
struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContent>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContent {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    Image {
        source: AnthropicImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
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

/// Join the `thinking` payloads from a Claude response's content blocks into
/// a single reasoning string. Returns `None` when no thinking blocks were
/// present so callers can preserve the "no reasoning channel" signal.
fn aggregate_reasoning(blocks: &[AnthropicResponseBlock]) -> Option<String> {
    let mut chunks = blocks.iter().filter_map(|block| match block {
        AnthropicResponseBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
        _ => None,
    });
    let first = chunks.next()?;
    let mut out = first.to_string();
    for chunk in chunks {
        out.push('\n');
        out.push_str(chunk);
    }
    Some(out)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicResponseBlock {
    Text {
        text: String,
    },
    /// Claude extended thinking blocks. The `signature` field is opaque
    /// metadata used when echoing thinking back to the model on subsequent
    /// turns; the user-facing `thinking` text is the part we surface.
    Thinking {
        thinking: String,
        #[serde(default)]
        #[allow(dead_code)]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
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
        let api_key =
            std::env::var("ANTHROPIC_API_KEY").map_err(|_| ProviderError::MissingApiKey)?;
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

        // Prompt-cache the system message and the trailing tool entry.
        // Together these typically account for >90% of the static part
        // of an agent turn (the system prompt restates the role +
        // available tools; the tool defs are several KB of JSON schema).
        // Anthropic charges 25% extra on the first request that creates
        // a breakpoint and 10% on cache reads — net win after 2 turns.
        let system = match &request.system {
            Some(text) if !text.is_empty() => vec![AnthropicSystemBlock {
                block_type: "text",
                text: text.clone(),
                cache_control: Some(EPHEMERAL),
            }],
            _ => Vec::new(),
        };

        let mut tools: Vec<AnthropicTool> = request
            .tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.parameters.clone(),
                cache_control: None,
            })
            .collect();
        // Mark the LAST tool with cache_control. Per the Anthropic
        // protocol, this marks the entire tools array as a cache prefix
        // (everything up to and including the marker is the cache key).
        if let Some(last) = tools.last_mut() {
            last.cache_control = Some(EPHEMERAL);
        }

        AnthropicRequest {
            model: request.model.as_str().to_string(),
            max_tokens: request.max_tokens.unwrap_or(4096),
            messages,
            system,
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

        let reasoning = aggregate_reasoning(&body.content);

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
            reasoning,
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
                delta: event.delta.and_then(|d| d.text.or(d.partial_json)),
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
        out.push(AnthropicMessage {
            role: "user".to_string(),
            content,
        });
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
                        cache_control: None,
                    }],
                );
            }
            "assistant" => {
                let mut content: Vec<AnthropicContent> = Vec::new();
                if !msg.content.is_empty() {
                    content.push(AnthropicContent::Text {
                        text: msg.content.clone(),
                        cache_control: None,
                    });
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        let id = tc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let func = tc
                            .get("function")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let name = func
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let raw_args = func
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let input: serde_json::Value =
                            serde_json::from_str(raw_args).unwrap_or(serde_json::json!({}));
                        content.push(AnthropicContent::ToolUse {
                            id,
                            name,
                            input,
                            cache_control: None,
                        });
                    }
                }
                if !content.is_empty() {
                    out.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content,
                    });
                }
            }
            // "user" and any other role we treat as a user text turn.
            _ => {
                let mut blocks: Vec<AnthropicContent> = Vec::new();
                if !msg.content.is_empty() {
                    blocks.push(AnthropicContent::Text {
                        text: msg.content.clone(),
                        cache_control: None,
                    });
                }
                for img in &msg.images {
                    blocks.push(AnthropicContent::Image {
                        source: parse_image_source(img),
                        cache_control: None,
                    });
                }
                if !blocks.is_empty() {
                    push_user(&mut out, blocks);
                }
            }
        }
    }

    // Mark the LAST content block of the LAST message with cache_control.
    // Combined with the markers already on system + tools, this gives us
    // exactly 3 cache breakpoints per request (system, tools, history).
    // The cached prefix grows by one assistant + tool_result pair each
    // turn, so any model "thinking" beyond turn 3 hits an ever-larger
    // cached prefix.
    if let Some(last_msg) = out.last_mut() {
        if let Some(last_block) = last_msg.content.last_mut() {
            set_cache_control(last_block, Some(EPHEMERAL));
        }
    }

    out
}

fn set_cache_control(content: &mut AnthropicContent, cc: Option<CacheControl>) {
    match content {
        AnthropicContent::Text { cache_control, .. } => *cache_control = cc,
        AnthropicContent::Image { cache_control, .. } => *cache_control = cc,
        AnthropicContent::ToolUse { cache_control, .. } => *cache_control = cc,
        AnthropicContent::ToolResult { cache_control, .. } => *cache_control = cc,
    }
}

/// Parse a `data:image/<mime>;base64,<payload>` URL or fall through to the
/// `url:` source for `https://...` references.
fn parse_image_source(url: &str) -> AnthropicImageSource {
    if let Some(after_data) = url.strip_prefix("data:") {
        if let Some((header, payload)) = after_data.split_once(",") {
            let media_type = header.split(';').next().unwrap_or("image/png").to_string();
            // Anthropic only accepts base64 sources for data URLs; if the
            // user passed `data:image/png,...` (no base64 encoding) we'd
            // need to re-encode. For now assume base64.
            return AnthropicImageSource::Base64 {
                media_type,
                data: payload.to_string(),
            };
        }
    }
    AnthropicImageSource::Url {
        url: url.to_string(),
    }
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
            images: Vec::new(),
        }
    }

    #[test]
    fn user_assistant_turns_serialize_as_text_blocks() {
        let result = convert_messages(&[msg("user", "hi"), msg("assistant", "hello")]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "user");
        assert!(matches!(
            result[0].content[0],
            AnthropicContent::Text { .. }
        ));
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
            images: Vec::new(),
        };
        let result = convert_messages(&[assistant]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.len(), 2);
        assert!(matches!(
            result[0].content[0],
            AnthropicContent::Text { .. }
        ));
        match &result[0].content[1] {
            AnthropicContent::ToolUse {
                id, name, input, ..
            } => {
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
                images: Vec::new(),
            },
            CompletionMessage {
                role: "tool".to_string(),
                content: "out2".to_string(),
                tool_calls: None,
                tool_call_id: Some("toolu_2".to_string()),
                images: Vec::new(),
            },
        ]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "user");
        assert_eq!(result[0].content.len(), 2);
        match (&result[0].content[0], &result[0].content[1]) {
            (
                AnthropicContent::ToolResult {
                    tool_use_id: id1, ..
                },
                AnthropicContent::ToolResult {
                    tool_use_id: id2, ..
                },
            ) => {
                assert_eq!(id1, "toolu_1");
                assert_eq!(id2, "toolu_2");
            }
            _ => panic!("expected both ToolResult"),
        }
    }

    fn provider() -> AnthropicProvider {
        AnthropicProvider::new("test-key".to_string(), None)
    }

    fn req(system: Option<&str>, tool_count: usize) -> CompletionRequest {
        CompletionRequest {
            model: crate::provider::ModelID::new("claude-3-5-sonnet-20241022"),
            messages: vec![msg("user", "hi")],
            system: system.map(|s| s.to_string()),
            tools: (0..tool_count)
                .map(|i| crate::provider::ToolDefinition {
                    name: format!("tool_{i}"),
                    description: "desc".to_string(),
                    parameters: serde_json::json!({"type": "object"}),
                })
                .collect(),
            max_tokens: Some(1024),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        }
    }

    #[test]
    fn system_block_carries_cache_control() {
        let p = provider();
        let r = p.build_request(&req(Some("you are an agent"), 0), false);
        assert_eq!(r.system.len(), 1);
        assert!(r.system[0].cache_control.is_some());
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""cache_control":{"type":"ephemeral"}"#));
        // and that the system block is the array form, not a plain string
        assert!(json.contains(r#""system":[{"type":"text","text":"you are an agent""#));
    }

    #[test]
    fn empty_system_omits_field_entirely() {
        let p = provider();
        let r = p.build_request(&req(None, 0), false);
        assert!(r.system.is_empty());
        let json = serde_json::to_string(&r).unwrap();
        // skip_serializing_if = "Vec::is_empty" -> no `system` key.
        assert!(!json.contains(r#""system":"#), "{json}");
    }

    #[test]
    fn only_last_tool_gets_cache_control() {
        let p = provider();
        let r = p.build_request(&req(None, 3), false);
        assert_eq!(r.tools.len(), 3);
        assert!(r.tools[0].cache_control.is_none());
        assert!(r.tools[1].cache_control.is_none());
        assert!(r.tools[2].cache_control.is_some());
    }

    #[test]
    fn zero_tools_does_not_panic() {
        let p = provider();
        let r = p.build_request(&req(Some("x"), 0), false);
        assert!(r.tools.is_empty());
    }

    fn cc(content: &AnthropicContent) -> Option<CacheControl> {
        match content {
            AnthropicContent::Text { cache_control, .. } => *cache_control,
            AnthropicContent::Image { cache_control, .. } => *cache_control,
            AnthropicContent::ToolUse { cache_control, .. } => *cache_control,
            AnthropicContent::ToolResult { cache_control, .. } => *cache_control,
        }
    }

    #[test]
    fn last_history_block_gets_cache_breakpoint() {
        // Two turns: user → assistant. The assistant's text block is
        // the last content of the last message and should carry
        // cache_control.
        let history = convert_messages(&[msg("user", "hi"), msg("assistant", "hello")]);
        let last = history.last().unwrap();
        assert!(cc(last.content.last().unwrap()).is_some());

        // Earlier blocks must NOT be marked (we use exactly 3
        // breakpoints in a request: system, tools, history-tail).
        let first = &history[0];
        assert!(cc(first.content.last().unwrap()).is_none());
    }

    #[test]
    fn last_tool_result_in_coalesced_user_gets_cache_breakpoint() {
        // Two tool results coalesce into a single user message. The
        // breakpoint lands on the LAST tool_result, caching both.
        let result = convert_messages(&[
            CompletionMessage {
                role: "tool".to_string(),
                content: "r1".to_string(),
                tool_calls: None,
                tool_call_id: Some("toolu_1".to_string()),
                images: Vec::new(),
            },
            CompletionMessage {
                role: "tool".to_string(),
                content: "r2".to_string(),
                tool_calls: None,
                tool_call_id: Some("toolu_2".to_string()),
                images: Vec::new(),
            },
        ]);
        assert_eq!(result.len(), 1);
        assert!(cc(&result[0].content[0]).is_none());
        assert!(cc(&result[0].content[1]).is_some());
    }

    #[test]
    fn empty_history_does_not_panic() {
        let result = convert_messages(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn data_url_image_becomes_base64_source() {
        let user = CompletionMessage {
            role: "user".to_string(),
            content: "what is this?".to_string(),
            tool_calls: None,
            tool_call_id: None,
            images: vec!["data:image/png;base64,iVBORw0KGgoAAAANSUhEUg".to_string()],
        };
        let result = convert_messages(&[user]);
        assert_eq!(result.len(), 1);
        // First block: the text. Second block: the image.
        assert_eq!(result[0].content.len(), 2);
        match &result[0].content[1] {
            AnthropicContent::Image { source, .. } => match source {
                AnthropicImageSource::Base64 { media_type, data } => {
                    assert_eq!(media_type, "image/png");
                    assert_eq!(data, "iVBORw0KGgoAAAANSUhEUg");
                }
                _ => panic!("expected base64 source"),
            },
            _ => panic!("expected Image block"),
        }
    }

    #[test]
    fn https_image_becomes_url_source() {
        let user = CompletionMessage {
            role: "user".to_string(),
            content: String::new(),
            tool_calls: None,
            tool_call_id: None,
            images: vec!["https://example.com/cat.png".to_string()],
        };
        let result = convert_messages(&[user]);
        assert_eq!(result.len(), 1);
        // No text, so just the one Image block.
        assert_eq!(result[0].content.len(), 1);
        match &result[0].content[0] {
            AnthropicContent::Image {
                source: AnthropicImageSource::Url { url },
                ..
            } => {
                assert_eq!(url, "https://example.com/cat.png");
            }
            _ => panic!("expected URL image"),
        }
    }

    #[test]
    fn thinking_blocks_deserialize_from_anthropic_response() {
        let payload = serde_json::json!([
            { "type": "thinking", "thinking": "stepping through", "signature": "sig-1" },
            { "type": "text", "text": "answer" }
        ]);
        let blocks: Vec<AnthropicResponseBlock> = serde_json::from_value(payload).unwrap();
        assert!(matches!(
            &blocks[0],
            AnthropicResponseBlock::Thinking { thinking, .. } if thinking == "stepping through"
        ));
    }

    #[test]
    fn aggregate_reasoning_joins_thinking_blocks() {
        let blocks = vec![
            AnthropicResponseBlock::Thinking {
                thinking: "first".to_string(),
                signature: None,
            },
            AnthropicResponseBlock::Text {
                text: "user answer".to_string(),
            },
            AnthropicResponseBlock::Thinking {
                thinking: "second".to_string(),
                signature: Some("sig".to_string()),
            },
        ];
        assert_eq!(aggregate_reasoning(&blocks).as_deref(), Some("first\nsecond"));
    }

    #[test]
    fn aggregate_reasoning_is_none_without_thinking_blocks() {
        let blocks = vec![AnthropicResponseBlock::Text {
            text: "hi".to_string(),
        }];
        assert!(aggregate_reasoning(&blocks).is_none());
    }
}
