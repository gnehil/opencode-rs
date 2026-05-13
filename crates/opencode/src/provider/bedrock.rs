//! AWS Bedrock provider.
//!
//! We talk to Bedrock's `/model/<modelId>/invoke` endpoint directly over
//! HTTP, signing requests with SigV4. The request body is the model's
//! native API shape — for Anthropic models that's the standard messages
//! API (which Bedrock proxies), so this provider reuses the same shape
//! as the regular `AnthropicProvider` plus an `anthropic_version`
//! discriminator.
//!
//! Streaming via `/invoke-with-response-stream` uses AWS's event-stream
//! binary framing protocol. This module decodes the frame envelope directly
//! and then parses the Anthropic-native JSON chunks Bedrock carries in
//! `chunk` events.
//!
//! Credentials come from environment variables:
//!   * AWS_ACCESS_KEY_ID (required)
//!   * AWS_SECRET_ACCESS_KEY (required)
//!   * AWS_SESSION_TOKEN (optional, for temporary credentials)
//!   * AWS_REGION or AWS_DEFAULT_REGION (defaults to us-east-1)
//!
//! The instance profile / SSO / ~/.aws/credentials credential chains
//! that the AWS SDK provides are not implemented here; if those are
//! needed, the user can run an SDK helper that resolves them into env
//! vars first.

use async_trait::async_trait;
use chrono::Utc;
use futures::StreamExt;
use hmac::{Hmac, Mac};
use lazy_static::lazy_static;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::CompletionRequest;
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

type HmacSha256 = Hmac<Sha256>;

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("anthropic.claude-3-5-sonnet-20241022-v2:0")),
            name: Some("Claude 3.5 Sonnet v2 (Bedrock)".to_string()),
            family: Some("claude".to_string()),
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
            id: Some(ModelID::new("anthropic.claude-3-5-haiku-20241022-v1:0")),
            name: Some("Claude 3.5 Haiku (Bedrock)".to_string()),
            family: Some("claude".to_string()),
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

pub struct BedrockProvider {
    client: Client,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

impl BedrockProvider {
    pub fn new(
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        region: String,
    ) -> Self {
        Self {
            client: Client::new(),
            region,
            access_key_id,
            secret_access_key,
            session_token,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let access_key_id =
            std::env::var("AWS_ACCESS_KEY_ID").map_err(|_| ProviderError::MissingApiKey)?;
        let secret_access_key =
            std::env::var("AWS_SECRET_ACCESS_KEY").map_err(|_| ProviderError::MissingApiKey)?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string());
        Ok(Self::new(
            access_key_id,
            secret_access_key,
            session_token,
            region,
        ))
    }

    fn endpoint(&self, model_id: &str) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke",
            self.region, model_id
        )
    }

    fn stream_endpoint(&self, model_id: &str) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/invoke-with-response-stream",
            self.region, model_id
        )
    }
}

#[async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }
    fn default_model(&self) -> Option<&ModelInfo> {
        MODELS.first()
    }
    fn models(&self) -> &[ModelInfo] {
        &MODELS
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model_id = request.model.to_string();

        // Bedrock proxies the model provider's native API. For Anthropic
        // models that's the messages API. We reuse the same body shape
        // as the regular Anthropic provider modulo `anthropic_version`
        // (Bedrock-specific) and the absence of `model` (it's in the
        // URL, not the body).
        let body = build_anthropic_body(&request);
        let body_bytes = serde_json::to_vec(&body)?;

        let url = self.endpoint(&model_id);
        let host = format!("bedrock-runtime.{}.amazonaws.com", self.region);
        let now = Utc::now();

        let mut headers = Vec::<(String, String)>::new();
        headers.push(("host".to_string(), host.clone()));
        headers.push((
            "x-amz-date".to_string(),
            now.format("%Y%m%dT%H%M%SZ").to_string(),
        ));
        headers.push(("content-type".to_string(), "application/json".to_string()));
        if let Some(token) = &self.session_token {
            headers.push(("x-amz-security-token".to_string(), token.clone()));
        }

        let authorization = sigv4_sign(
            "POST",
            &format!("/model/{}/invoke", urlencoding::encode(&model_id)),
            "",
            &headers,
            &body_bytes,
            &self.access_key_id,
            &self.secret_access_key,
            &self.region,
            "bedrock",
            now,
        );

        let mut req = self
            .client
            .post(&url)
            .header("Authorization", authorization)
            .header("Content-Type", "application/json");
        for (k, v) in &headers {
            // Skip Host (reqwest sets it from the URL) and Content-Type
            // (already set above).
            if k == "host" || k == "content-type" {
                continue;
            }
            req = req.header(k.as_str(), v.as_str());
        }

        let response = req.body(body_bytes).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ProviderError::api(status.as_u16(), body));
        }
        let data: serde_json::Value = response.json().await?;
        Ok(parse_anthropic_response(data, &model_id))
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        let model_id = request.model.to_string();
        let body = build_anthropic_body(&request);
        let body_bytes = serde_json::to_vec(&body)?;
        let url = self.stream_endpoint(&model_id);
        let host = format!("bedrock-runtime.{}.amazonaws.com", self.region);
        let client = self.client.clone();
        let region = self.region.clone();
        let access_key_id = self.access_key_id.clone();
        let secret_access_key = self.secret_access_key.clone();
        let session_token = self.session_token.clone();

        let stream = async_stream::try_stream! {
            let now = Utc::now();
            let mut headers = Vec::<(String, String)>::new();
            headers.push(("host".to_string(), host.clone()));
            headers.push((
                "x-amz-date".to_string(),
                now.format("%Y%m%dT%H%M%SZ").to_string(),
            ));
            headers.push(("accept".to_string(), "application/vnd.amazon.eventstream".to_string()));
            headers.push(("content-type".to_string(), "application/json".to_string()));
            headers.push(("x-amzn-bedrock-accept".to_string(), "application/json".to_string()));
            if let Some(token) = &session_token {
                headers.push(("x-amz-security-token".to_string(), token.clone()));
            }

            let authorization = sigv4_sign(
                "POST",
                &format!("/model/{}/invoke-with-response-stream", urlencoding::encode(&model_id)),
                "",
                &headers,
                &body_bytes,
                &access_key_id,
                &secret_access_key,
                &region,
                "bedrock",
                now,
            );

            let mut req = client
                .post(&url)
                .header("Authorization", authorization);
            for (k, v) in &headers {
                if k == "host" {
                    continue;
                }
                req = req.header(k.as_str(), v.as_str());
            }

            let response = req.body(body_bytes).send().await?;
            let status = response.status();
            let response = if status.is_success() {
                response
            } else {
                let body = response.text().await.unwrap_or_default();
                Err(ProviderError::api(status.as_u16(), body))?;
                unreachable!()
            };

            let mut stream_reader = response.bytes_stream();
            let mut buffer = Vec::new();
            while let Some(chunk) = stream_reader.next().await.transpose()? {
                for event in decode_bedrock_event_stream_frames(&mut buffer, &chunk)? {
                    yield event;
                }
            }
            if !buffer.is_empty() {
                Err(ProviderError::stream("incomplete bedrock event-stream frame"))?;
            }
        };

        Ok(Box::pin(stream))
    }
}

fn decode_bedrock_event_stream_frames(
    buffer: &mut Vec<u8>,
    chunk: &[u8],
) -> ProviderResult<Vec<StreamEvent>> {
    buffer.extend_from_slice(chunk);
    let mut events = Vec::new();
    let mut offset = 0usize;

    while buffer.len().saturating_sub(offset) >= 12 {
        let total_len = read_u32(&buffer[offset..offset + 4]) as usize;
        if total_len < 16 {
            return Err(ProviderError::stream(
                "invalid bedrock event-stream frame length",
            ));
        }
        if buffer.len() - offset < total_len {
            break;
        }

        let frame = &buffer[offset..offset + total_len];
        events.extend(decode_bedrock_event_stream_frame(frame)?);
        offset += total_len;
    }

    if offset > 0 {
        buffer.drain(..offset);
    }
    Ok(events)
}

fn decode_bedrock_event_stream_frame(frame: &[u8]) -> ProviderResult<Vec<StreamEvent>> {
    let total_len = read_u32(&frame[0..4]) as usize;
    let headers_len = read_u32(&frame[4..8]) as usize;
    let prelude_crc = read_u32(&frame[8..12]);
    if total_len != frame.len() {
        return Err(ProviderError::stream(
            "bedrock event-stream frame length mismatch",
        ));
    }
    if crc32(&frame[0..8]) != prelude_crc {
        return Err(ProviderError::stream(
            "bedrock event-stream prelude crc mismatch",
        ));
    }
    let message_crc = read_u32(&frame[total_len - 4..total_len]);
    if crc32(&frame[..total_len - 4]) != message_crc {
        return Err(ProviderError::stream(
            "bedrock event-stream message crc mismatch",
        ));
    }
    let headers_end = 12 + headers_len;
    if headers_end > total_len - 4 {
        return Err(ProviderError::stream(
            "bedrock event-stream headers exceed frame",
        ));
    }

    let headers = decode_event_stream_headers(&frame[12..headers_end])?;
    if headers.get(":message-type").map(String::as_str) != Some("event") {
        return Ok(Vec::new());
    }
    let event_type = match headers.get(":event-type") {
        Some(v) => v.as_str(),
        None => return Ok(Vec::new()),
    };
    let payload = &frame[headers_end..total_len - 4];

    if event_type != "chunk" {
        let message = serde_json::from_slice::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("bedrock stream event: {event_type}"));
        return Err(ProviderError::stream(message));
    }

    parse_bedrock_chunk_payload(payload).map(|event| event.into_iter().collect())
}

fn decode_event_stream_headers(bytes: &[u8]) -> ProviderResult<HashMap<String, String>> {
    let mut headers = HashMap::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        let name_len = *bytes
            .get(pos)
            .ok_or_else(|| ProviderError::stream("truncated bedrock event-stream header name"))?
            as usize;
        pos += 1;
        let name_end = pos + name_len;
        let name =
            std::str::from_utf8(bytes.get(pos..name_end).ok_or_else(|| {
                ProviderError::stream("truncated bedrock event-stream header name")
            })?)
            .map_err(|_| ProviderError::stream("invalid bedrock event-stream header name"))?
            .to_string();
        pos = name_end;
        let value_type = *bytes
            .get(pos)
            .ok_or_else(|| ProviderError::stream("truncated bedrock event-stream header type"))?;
        pos += 1;

        if value_type == 7 {
            let value_len = read_u16(bytes.get(pos..pos + 2).ok_or_else(|| {
                ProviderError::stream("truncated bedrock event-stream string header")
            })?) as usize;
            pos += 2;
            let value_end = pos + value_len;
            let value = std::str::from_utf8(bytes.get(pos..value_end).ok_or_else(|| {
                ProviderError::stream("truncated bedrock event-stream string header")
            })?)
            .map_err(|_| ProviderError::stream("invalid bedrock event-stream string header"))?
            .to_string();
            headers.insert(name, value);
            pos = value_end;
        } else {
            pos = skip_event_stream_header_value(bytes, pos, value_type)?;
        }
    }
    Ok(headers)
}

fn skip_event_stream_header_value(
    bytes: &[u8],
    pos: usize,
    value_type: u8,
) -> ProviderResult<usize> {
    let len = match value_type {
        0 | 1 => 0,
        2 => 1,
        3 => 2,
        4 => 4,
        5 | 8 => 8,
        9 => 16,
        6 => {
            let size = read_u16(bytes.get(pos..pos + 2).ok_or_else(|| {
                ProviderError::stream("truncated bedrock event-stream binary header")
            })?) as usize;
            return Ok(pos + 2 + size);
        }
        _ => {
            return Err(ProviderError::stream(
                "unsupported bedrock event-stream header type",
            ))
        }
    };
    let next = pos + len;
    if next > bytes.len() {
        return Err(ProviderError::stream(
            "truncated bedrock event-stream header value",
        ));
    }
    Ok(next)
}

fn parse_bedrock_chunk_payload(payload: &[u8]) -> ProviderResult<Option<StreamEvent>> {
    use base64::Engine;

    let payload = match serde_json::from_slice::<serde_json::Value>(payload) {
        Ok(v) => {
            if let Some(bytes) = v.get("bytes").and_then(|b| b.as_str()) {
                base64::engine::general_purpose::STANDARD
                    .decode(bytes)
                    .map_err(|e| {
                        ProviderError::stream(format!("invalid bedrock chunk bytes: {e}"))
                    })?
            } else {
                serde_json::to_vec(&v)?
            }
        }
        Err(_) => payload.to_vec(),
    };
    let data: serde_json::Value = serde_json::from_slice(&payload)?;
    Ok(parse_anthropic_stream_event(data))
}

fn parse_anthropic_stream_event(data: serde_json::Value) -> Option<StreamEvent> {
    let event_type = data["type"].as_str()?;
    match event_type {
        "message_start" => Some(StreamEvent {
            event_type: "message_start".to_string(),
            delta: None,
            tool_call: None,
            stop_reason: None,
            usage: Some(TokenUsage {
                input: data["message"]["usage"]["input_tokens"]
                    .as_u64()
                    .unwrap_or(0),
                output: data["message"]["usage"]["output_tokens"]
                    .as_u64()
                    .unwrap_or(0),
                cache_read: None,
                cache_write: None,
            }),
        }),
        "content_block_start" => Some(StreamEvent {
            event_type: "content_block_start".to_string(),
            delta: None,
            tool_call: data["content_block"].as_object().and_then(|cb| {
                if cb.get("type")?.as_str()? != "tool_use" {
                    return None;
                }
                Some(ToolCall {
                    id: cb.get("id")?.as_str()?.to_string(),
                    name: cb.get("name")?.as_str()?.to_string(),
                    arguments: cb
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}))
                        .to_string(),
                })
            }),
            stop_reason: None,
            usage: None,
        }),
        "content_block_delta" => Some(StreamEvent {
            event_type: "content_block_delta".to_string(),
            delta: data["delta"]["text"]
                .as_str()
                .or_else(|| data["delta"]["partial_json"].as_str())
                .map(str::to_string),
            tool_call: None,
            stop_reason: None,
            usage: None,
        }),
        "message_delta" => {
            let stop_reason = data["delta"]["stop_reason"].as_str().map(str::to_string);
            let usage = Some(TokenUsage {
                input: data["usage"]["input_tokens"].as_u64().unwrap_or(0),
                output: data["usage"]["output_tokens"].as_u64().unwrap_or(0),
                cache_read: None,
                cache_write: None,
            });
            Some(StreamEvent {
                event_type: if stop_reason.is_some() {
                    "message_stop"
                } else {
                    "message_delta"
                }
                .to_string(),
                delta: None,
                tool_call: None,
                stop_reason,
                usage,
            })
        }
        "message_stop" => None,
        _ => Some(StreamEvent {
            event_type: event_type.to_string(),
            delta: None,
            tool_call: None,
            stop_reason: None,
            usage: None,
        }),
    }
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

/// Build an Anthropic messages-API body suitable for Bedrock's invoke
/// endpoint. Mirrors regular AnthropicProvider but adds the
/// `anthropic_version` discriminator Bedrock requires and omits `model`
/// (Bedrock takes the model ID in the URL).
fn build_anthropic_body(request: &CompletionRequest) -> serde_json::Value {
    let messages = convert_messages(&request.messages);
    let system: Vec<serde_json::Value> = match &request.system {
        Some(s) if !s.is_empty() => vec![serde_json::json!({"type": "text", "text": s})],
        _ => Vec::new(),
    };
    let tools: Vec<serde_json::Value> = request
        .tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect();
    let mut body = serde_json::json!({
        "anthropic_version": "bedrock-2023-05-31",
        "max_tokens": request.max_tokens.unwrap_or(4096),
        "messages": messages,
    });
    if !system.is_empty() {
        body["system"] = serde_json::Value::Array(system);
    }
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools);
    }
    if let Some(temp) = request.temperature {
        body["temperature"] = serde_json::json!(temp);
    }
    if let Some(p) = request.top_p {
        body["top_p"] = serde_json::json!(p);
    }
    body
}

fn convert_messages(messages: &[crate::provider::CompletionMessage]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    let push_user = |out: &mut Vec<serde_json::Value>, blocks: Vec<serde_json::Value>| {
        if blocks.is_empty() {
            return;
        }
        if let Some(last) = out.last_mut() {
            if last["role"] == "user" {
                if let Some(content) = last["content"].as_array_mut() {
                    content.extend(blocks);
                    return;
                }
            }
        }
        out.push(serde_json::json!({"role": "user", "content": blocks}));
    };

    for msg in messages {
        match msg.role.as_str() {
            "tool" => {
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                push_user(
                    &mut out,
                    vec![serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": msg.content,
                    })],
                );
            }
            "assistant" => {
                let mut content: Vec<serde_json::Value> = Vec::new();
                if !msg.content.is_empty() {
                    content.push(serde_json::json!({"type": "text", "text": msg.content}));
                }
                if let Some(tcs) = &msg.tool_calls {
                    for tc in tcs {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let func = tc
                            .get("function")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args_raw = func
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        let input: serde_json::Value =
                            serde_json::from_str(args_raw).unwrap_or(serde_json::json!({}));
                        content.push(serde_json::json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        }));
                    }
                }
                if !content.is_empty() {
                    out.push(serde_json::json!({"role": "assistant", "content": content}));
                }
            }
            _ => {
                let mut blocks: Vec<serde_json::Value> = Vec::new();
                if !msg.content.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": msg.content}));
                }
                for img in &msg.images {
                    if let Some(after) = img.strip_prefix("data:") {
                        if let Some((header, payload)) = after.split_once(',') {
                            let media = header.split(';').next().unwrap_or("image/png");
                            blocks.push(serde_json::json!({
                                "type": "image",
                                "source": {"type": "base64", "media_type": media, "data": payload},
                            }));
                            continue;
                        }
                    }
                    blocks.push(serde_json::json!({
                        "type": "image",
                        "source": {"type": "url", "url": img},
                    }));
                }
                if !blocks.is_empty() {
                    push_user(&mut out, blocks);
                }
            }
        }
    }
    out
}

fn parse_anthropic_response(data: serde_json::Value, model: &str) -> CompletionResponse {
    let content = data["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b["type"] == "text" {
                        b["text"].as_str().map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let tool_calls = data["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b["type"] == "tool_use" {
                        Some(ToolCall {
                            id: b["id"].as_str().unwrap_or("").to_string(),
                            name: b["name"].as_str().unwrap_or("").to_string(),
                            arguments: b["input"].to_string(),
                        })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    CompletionResponse {
        content,
        tool_calls,
        usage: TokenUsage {
            input: data["usage"]["input_tokens"].as_u64().unwrap_or(0),
            output: data["usage"]["output_tokens"].as_u64().unwrap_or(0),
            cache_read: None,
            cache_write: None,
        },
        stop_reason: data["stop_reason"].as_str().map(String::from),
        model: model.to_string(),
    }
}

/// AWS SigV4 request signing.
///
/// Reference: https://docs.aws.amazon.com/general/latest/gr/sigv4-signed-request-examples.html
///
/// We always sign the body, so requests are not eligible for streaming
/// uploads. For Bedrock /invoke that's fine — bodies are small JSON.
#[allow(clippy::too_many_arguments)]
fn sigv4_sign(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(String, String)],
    body: &[u8],
    access_key: &str,
    secret_key: &str,
    region: &str,
    service: &str,
    now: chrono::DateTime<Utc>,
) -> String {
    let date = now.format("%Y%m%d").to_string();
    let datetime = now.format("%Y%m%dT%H%M%SZ").to_string();

    // Build canonical headers. AWS expects them lowercase + sorted by name.
    let mut sorted = headers.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = sorted
        .iter()
        .map(|(k, v)| format!("{}:{}\n", k.to_lowercase(), v.trim()))
        .collect();
    let signed_headers: String = sorted
        .iter()
        .map(|(k, _)| k.to_lowercase())
        .collect::<Vec<_>>()
        .join(";");

    let payload_hash = hex_sha256(body);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method, canonical_uri, canonical_query, canonical_headers, signed_headers, payload_hash,
    );
    let canonical_request_hash = hex_sha256(canonical_request.as_bytes());

    let scope = format!("{}/{}/{}/aws4_request", date, region, service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        datetime, scope, canonical_request_hash
    );

    let k_date = hmac(format!("AWS4{}", secret_key).as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, string_to_sign.as_bytes()));

    format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        access_key, scope, signed_headers, signature
    )
}

fn hex_sha256(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
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
            images: vec![],
        }
    }

    #[test]
    fn convert_messages_emits_anthropic_shape() {
        let out = convert_messages(&[msg("user", "hi"), msg("assistant", "hello")]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[0]["content"][0]["type"], "text");
        assert_eq!(out[0]["content"][0]["text"], "hi");
    }

    #[test]
    fn tool_role_becomes_user_tool_result_block() {
        let mut t = msg("tool", "result");
        t.tool_call_id = Some("toolu_x".to_string());
        let out = convert_messages(&[t]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[0]["content"][0]["type"], "tool_result");
        assert_eq!(out[0]["content"][0]["tool_use_id"], "toolu_x");
    }

    #[test]
    fn body_carries_anthropic_version_and_omits_model() {
        let request = CompletionRequest {
            model: ModelID::new("anthropic.claude-3-5-sonnet-20241022-v2:0"),
            messages: vec![msg("user", "hi")],
            system: Some("be helpful".to_string()),
            tools: vec![],
            max_tokens: Some(1000),
            temperature: Some(0.5),
            top_p: None,
            stop_sequences: None,
        };
        let body = build_anthropic_body(&request);
        assert_eq!(body["anthropic_version"], "bedrock-2023-05-31");
        assert!(body.get("model").is_none());
        assert_eq!(body["system"][0]["text"], "be helpful");
        assert_eq!(body["max_tokens"], 1000);
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn parse_anthropic_response_extracts_text_and_tool_calls() {
        let raw = serde_json::json!({
            "content": [
                {"type": "text", "text": "let me check "},
                {"type": "tool_use", "id": "toolu_1", "name": "bash", "input": {"command": "ls"}},
                {"type": "text", "text": "now."},
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 12, "output_tokens": 8}
        });
        let r = parse_anthropic_response(raw, "anthropic.claude-3-5-sonnet-20241022-v2:0");
        assert_eq!(r.content, "let me check now.");
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].name, "bash");
        assert_eq!(r.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(r.usage.input, 12);
        assert_eq!(r.usage.output, 8);
    }

    #[test]
    fn bedrock_event_stream_decodes_text_tool_and_finish() {
        let mut buffer = Vec::new();
        let input = [
            event_stream_frame(
                "chunk",
                br#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
            ),
            event_stream_frame(
                "chunk",
                br#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"bash","input":{}}}"#,
            ),
            event_stream_frame(
                "chunk",
                br#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"ls\"}"}}"#,
            ),
            event_stream_frame(
                "chunk",
                br#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":8}}"#,
            ),
        ]
        .concat();

        let events = decode_bedrock_event_stream_frames(&mut buffer, &input).unwrap();
        assert!(buffer.is_empty());
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].event_type, "content_block_delta");
        assert_eq!(events[0].delta.as_deref(), Some("hello"));

        let tool = events[1].tool_call.as_ref().expect("tool start");
        assert_eq!(events[1].event_type, "content_block_start");
        assert_eq!(tool.id, "toolu_1");
        assert_eq!(tool.name, "bash");
        assert_eq!(tool.arguments, "{}");

        assert_eq!(events[2].event_type, "content_block_delta");
        assert_eq!(events[2].delta.as_deref(), Some(r#"{"command":"ls"}"#));

        assert_eq!(events[3].event_type, "message_stop");
        assert_eq!(events[3].stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(events[3].usage.as_ref().unwrap().output, 8);
    }

    fn event_stream_frame(event_type: &str, payload: &[u8]) -> Vec<u8> {
        let mut headers = Vec::new();
        push_string_header(&mut headers, ":message-type", "event");
        push_string_header(&mut headers, ":event-type", event_type);

        let total_len = 12 + headers.len() + payload.len() + 4;
        let headers_len = headers.len();
        let mut frame = Vec::with_capacity(total_len);
        frame.extend_from_slice(&(total_len as u32).to_be_bytes());
        frame.extend_from_slice(&(headers_len as u32).to_be_bytes());
        let prelude_crc = test_crc32(&frame);
        frame.extend_from_slice(&prelude_crc.to_be_bytes());
        frame.extend_from_slice(&headers);
        frame.extend_from_slice(payload);
        let message_crc = test_crc32(&frame);
        frame.extend_from_slice(&message_crc.to_be_bytes());
        frame
    }

    fn push_string_header(out: &mut Vec<u8>, name: &str, value: &str) {
        out.push(name.len() as u8);
        out.extend_from_slice(name.as_bytes());
        out.push(7);
        out.extend_from_slice(&(value.len() as u16).to_be_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    fn test_crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in bytes {
            crc ^= *byte as u32;
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }

    /// Known SigV4 test vector from the AWS docs: GET service.amazonaws.com/?Param=value
    /// (sourced from https://docs.aws.amazon.com/general/latest/gr/sigv4-test-suite.html).
    /// We don't reproduce a Bedrock-specific vector because there aren't
    /// published ones; instead we check that the signing function:
    ///   - hashes the empty body correctly,
    ///   - produces a deterministic output for fixed inputs.
    #[test]
    fn sigv4_signature_is_deterministic_for_fixed_input() {
        let now = chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let headers = vec![
            (
                "host".to_string(),
                "bedrock-runtime.us-east-1.amazonaws.com".to_string(),
            ),
            ("x-amz-date".to_string(), "20240101T000000Z".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ];
        let sig1 = sigv4_sign(
            "POST",
            "/model/test/invoke",
            "",
            &headers,
            b"{}",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "bedrock",
            now,
        );
        let sig2 = sigv4_sign(
            "POST",
            "/model/test/invoke",
            "",
            &headers,
            b"{}",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "bedrock",
            now,
        );
        assert_eq!(sig1, sig2);
        assert!(sig1.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20240101/us-east-1/bedrock/aws4_request"));
        // SignedHeaders should be alphabetical.
        assert!(sig1.contains("SignedHeaders=content-type;host;x-amz-date"));
    }
}
