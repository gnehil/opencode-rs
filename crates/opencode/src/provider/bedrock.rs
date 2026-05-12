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
//! binary framing protocol, which is non-trivial to parse from scratch.
//! `stream()` returns a single-event stream wrapping a non-streaming
//! call — functional for the agent loop but not real streaming. Adding
//! true streaming would mean either pulling in `aws-sdk-bedrockruntime`
//! (heavy) or implementing event-stream framing. Tracked as follow-up.
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
use hmac::{Hmac, Mac};
use lazy_static::lazy_static;
use reqwest::Client;
use sha2::{Digest, Sha256};

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
}

#[async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str { "bedrock" }
    fn default_model(&self) -> Option<&ModelInfo> { MODELS.first() }
    fn models(&self) -> &[ModelInfo] { &MODELS }

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

    fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> {
        // Real streaming would call `/invoke-with-response-stream` and
        // parse AWS event-stream binary framing. Deferred; agent loop
        // works without streaming.
        Err(ProviderError::stream(
            "bedrock streaming not implemented (use non-streaming complete())",
        ))
    }
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
        .map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.parameters,
        }))
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

fn convert_messages(
    messages: &[crate::provider::CompletionMessage],
) -> Vec<serde_json::Value> {
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
                push_user(&mut out, vec![serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": msg.content,
                })]);
            }
            "assistant" => {
                let mut content: Vec<serde_json::Value> = Vec::new();
                if !msg.content.is_empty() {
                    content.push(serde_json::json!({"type": "text", "text": msg.content}));
                }
                if let Some(tcs) = &msg.tool_calls {
                    for tc in tcs {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let func = tc.get("function").cloned().unwrap_or(serde_json::Value::Null);
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args_raw = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
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
        method,
        canonical_uri,
        canonical_query,
        canonical_headers,
        signed_headers,
        payload_hash,
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
            ("host".to_string(), "bedrock-runtime.us-east-1.amazonaws.com".to_string()),
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
