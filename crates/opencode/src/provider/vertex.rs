use async_trait::async_trait;
use lazy_static::lazy_static;
use reqwest::Client;

use super::id::ModelID;
use super::model::ModelInfo;
use super::request::{CompletionMessage, CompletionRequest};
use super::response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, Provider, ProviderError, ProviderResult};

lazy_static! {
    static ref MODELS: Vec<ModelInfo> = vec![ModelInfo {
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
    },];
}

pub struct VertexProvider {
    client: Client,
    project_id: String,
    location: String,
    access_token: String,
}

impl VertexProvider {
    pub fn new(project_id: String, location: String, access_token: String) -> Self {
        Self {
            client: Client::new(),
            project_id,
            location,
            access_token,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let project_id = std::env::var("GOOGLE_PROJECT_ID")
            .or_else(|_| std::env::var("GCP_PROJECT_ID"))
            .map_err(|_| ProviderError::MissingApiKey)?;
        let location =
            std::env::var("GOOGLE_LOCATION").unwrap_or_else(|_| "us-central1".to_string());
        let access_token =
            std::env::var("GOOGLE_ACCESS_TOKEN").map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(project_id, location, access_token))
    }
}

#[async_trait]
impl Provider for VertexProvider {
    fn name(&self) -> &str {
        "google-vertex"
    }
    fn default_model(&self) -> Option<&ModelInfo> {
        MODELS.first()
    }
    fn models(&self) -> &[ModelInfo] {
        &MODELS
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model = request.model.to_string();
        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
            self.location, self.project_id, self.location, model
        );

        let contents = convert_messages_gemini(&request.messages);
        let body = build_gemini_body(&request, contents);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| ProviderError::api(0, e.to_string()))?;

        let content = data["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        Ok(CompletionResponse {
            content,
            tool_calls: vec![],
            usage: TokenUsage {
                input: 0,
                output: 0,
                cache_read: None,
                cache_write: None,
            },
            stop_reason: Some(
                data["candidates"][0]["finishReason"]
                    .as_str()
                    .unwrap_or("STOP")
                    .to_string(),
            ),
            model: model.clone(),
            reasoning: None,
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
        let contents = convert_messages_gemini(&request.messages);
        let body = build_gemini_body(&request, contents);
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

/// Convert provider-neutral `CompletionMessage`s to Gemini's `contents`
/// array. Gemini's schema is:
///
///   {"role": "user"|"model", "parts": [{"text": "..."} | {"inlineData": {...}}]}
///
/// `assistant` role maps to `model`. `tool` results map to a `user` turn
/// with a `functionResponse` part. `tool_calls` on assistant messages
/// map to `functionCall` parts.
///
/// Image attachments (msg.images) are emitted as `inlineData` parts for
/// `data:` URLs, or as `fileData` parts for `https://` URLs. Both shapes
/// are documented by Gemini.
fn convert_messages_gemini(messages: &[CompletionMessage]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    for msg in messages {
        let role = match msg.role.as_str() {
            "assistant" => "model",
            "tool" => "user", // Gemini puts functionResponse on a user turn.
            _ => "user",
        };

        let mut parts: Vec<serde_json::Value> = Vec::new();

        if msg.role == "tool" {
            // tool_call_id is the function name + invocation id; Gemini
            // uses it to correlate to the prior functionCall.
            parts.push(serde_json::json!({
                "functionResponse": {
                    "name": msg.tool_call_id.clone().unwrap_or_default(),
                    "response": { "content": msg.content.clone() }
                }
            }));
        } else {
            if !msg.content.is_empty() {
                parts.push(serde_json::json!({"text": msg.content}));
            }
            // assistant tool_calls -> Gemini functionCall parts.
            if let Some(tcs) = &msg.tool_calls {
                for tc in tcs {
                    let func = tc
                        .get("function")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let args_raw = func
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    let args: serde_json::Value =
                        serde_json::from_str(args_raw).unwrap_or(serde_json::json!({}));
                    parts.push(serde_json::json!({
                        "functionCall": { "name": name, "args": args }
                    }));
                }
            }
            // Image attachments.
            for img in &msg.images {
                if let Some(after) = img.strip_prefix("data:") {
                    if let Some((header, payload)) = after.split_once(',') {
                        let mime = header.split(';').next().unwrap_or("image/png");
                        parts.push(serde_json::json!({
                            "inlineData": { "mimeType": mime, "data": payload }
                        }));
                        continue;
                    }
                }
                // Fall through to fileData for https/gs:// URIs.
                parts.push(serde_json::json!({
                    "fileData": { "mimeType": "image/png", "fileUri": img }
                }));
            }
        }

        if parts.is_empty() {
            continue;
        }
        out.push(serde_json::json!({ "role": role, "parts": parts }));
    }
    out
}

fn build_gemini_body(
    request: &CompletionRequest,
    contents: Vec<serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "contents": contents,
        "generationConfig": { "maxOutputTokens": request.max_tokens.unwrap_or(4096) }
    });
    if let Some(system) = &request.system {
        if !system.is_empty() {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": system}]
            });
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::CompletionMessage;

    fn msg(role: &str, content: &str, images: Vec<&str>) -> CompletionMessage {
        CompletionMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
            images: images.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn assistant_role_maps_to_model() {
        let out = convert_messages_gemini(&[msg("assistant", "hi", vec![])]);
        assert_eq!(out[0]["role"], "model");
    }

    #[test]
    fn data_url_image_becomes_inline_data() {
        let out = convert_messages_gemini(&[msg(
            "user",
            "what's this",
            vec!["data:image/jpeg;base64,XYZ"],
        )]);
        let parts = out[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/jpeg");
        assert_eq!(parts[1]["inlineData"]["data"], "XYZ");
    }

    #[test]
    fn https_image_becomes_file_data() {
        let out = convert_messages_gemini(&[msg("user", "", vec!["https://example.com/a.png"])]);
        let parts = out[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["fileData"]["fileUri"], "https://example.com/a.png");
    }

    #[test]
    fn tool_role_emits_function_response() {
        let mut m = msg("tool", "result text", vec![]);
        m.tool_call_id = Some("lookup".to_string());
        let out = convert_messages_gemini(&[m]);
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[0]["parts"][0]["functionResponse"]["name"], "lookup");
    }

    #[test]
    fn assistant_tool_calls_become_function_call_parts() {
        let mut a = msg("assistant", "let me check", vec![]);
        a.tool_calls = Some(vec![serde_json::json!({
            "id": "x",
            "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"}
        })]);
        let out = convert_messages_gemini(&[a]);
        let parts = out[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["functionCall"]["name"], "bash");
        assert_eq!(parts[1]["functionCall"]["args"]["command"], "ls");
    }

    #[test]
    fn system_prompt_lands_in_system_instruction() {
        let req = CompletionRequest {
            model: crate::provider::ModelID::new("x"),
            messages: vec![msg("user", "hi", vec![])],
            system: Some("you are an agent".to_string()),
            tools: vec![],
            max_tokens: Some(100),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        };
        let body = build_gemini_body(&req, convert_messages_gemini(&req.messages));
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "you are an agent"
        );
    }
}
