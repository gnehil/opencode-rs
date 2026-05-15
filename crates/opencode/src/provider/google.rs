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

pub struct GoogleProvider {
    client: Client,
    api_key: String,
}

impl GoogleProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub fn from_env() -> ProviderResult<Self> {
        let api_key = std::env::var("GOOGLE_API_KEY").map_err(|_| ProviderError::MissingApiKey)?;
        Ok(Self::new(api_key))
    }

    fn build_contents(&self, request: &CompletionRequest) -> Vec<GoogleContent> {
        request
            .messages
            .iter()
            .map(|msg| {
                let role = if msg.role == "assistant" {
                    "model".to_string()
                } else {
                    msg.role.clone()
                };
                GoogleContent {
                    role,
                    parts: vec![GooglePart {
                        text: msg.content.clone(),
                    }],
                }
            })
            .collect()
    }

    fn build_request(&self, request: &CompletionRequest) -> GoogleRequest {
        GoogleRequest {
            contents: self.build_contents(request),
            generation_config: Some(GoogleGenerationConfig {
                max_output_tokens: request.max_tokens,
                temperature: request.temperature,
                top_p: request.top_p,
            }),
            tools: if request.tools.is_empty() {
                None
            } else {
                Some(
                    request
                        .tools
                        .iter()
                        .map(|t| GoogleTool {
                            function_declarations: vec![GoogleFunctionDecl {
                                name: t.name.clone(),
                                description: t.description.clone(),
                                parameters: t.parameters.clone(),
                            }],
                        })
                        .collect(),
                )
            },
        }
    }
}

#[derive(Serialize)]
struct GoogleContent {
    role: String,
    parts: Vec<GooglePart>,
}

#[derive(Serialize)]
struct GooglePart {
    text: String,
}

#[derive(Serialize)]
struct GoogleRequest {
    contents: Vec<GoogleContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GoogleGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GoogleTool>>,
}

#[derive(Serialize)]
struct GoogleGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
}

#[derive(Serialize)]
struct GoogleTool {
    function_declarations: Vec<GoogleFunctionDecl>,
}

#[derive(Serialize)]
struct GoogleFunctionDecl {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct GoogleResponse {
    candidates: Vec<GoogleCandidate>,
    usage_metadata: Option<GoogleUsageMetadata>,
}

#[derive(Deserialize)]
struct GoogleCandidate {
    content: GoogleResponseContent,
    finish_reason: String,
}

#[derive(Deserialize)]
struct GoogleResponseContent {
    parts: Vec<GoogleResponsePart>,
}

#[derive(Deserialize)]
struct GoogleResponsePart {
    text: Option<String>,
    function_call: Option<GoogleFunctionCall>,
    /// Gemini thinking models flag reasoning parts with `thought: true`.
    /// The accompanying `text` is the chain-of-thought rather than
    /// user-visible content.
    #[serde(default)]
    thought: bool,
}

/// Walk Gemini response parts and split their text payloads into a
/// user-visible answer and an optional reasoning channel based on the
/// `thought: true` flag. Multiple parts of the same kind are joined with
/// newlines; reasoning is `None` when no thinking parts were present.
fn split_text_parts(parts: &[GoogleResponsePart]) -> (String, Option<String>) {
    let mut content = String::new();
    let mut reasoning = String::new();
    for part in parts {
        let Some(text) = part.text.as_deref() else {
            continue;
        };
        let target = if part.thought {
            &mut reasoning
        } else {
            &mut content
        };
        if !target.is_empty() {
            target.push('\n');
        }
        target.push_str(text);
    }
    let reasoning = (!reasoning.is_empty()).then_some(reasoning);
    (content, reasoning)
}

#[derive(Deserialize)]
struct GoogleFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Deserialize)]
struct GoogleUsageMetadata {
    prompt_token_count: u64,
    candidates_token_count: u64,
    total_token_count: u64,
}

use lazy_static::lazy_static;

lazy_static! {
    static ref GOOGLE_MODELS: Vec<ModelInfo> = vec![
        ModelInfo {
            id: Some(ModelID::new("gemini-2.0-flash")),
            name: Some("Gemini 2.0 Flash".to_string()),
            family: Some("gemini".to_string()),
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
            id: Some(ModelID::new("gemini-1.5-pro")),
            name: Some("Gemini 1.5 Pro".to_string()),
            family: Some("gemini".to_string()),
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 1.25,
                output: 5.0,
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
            id: Some(ModelID::new("gemini-1.5-flash")),
            name: Some("Gemini 1.5 Flash".to_string()),
            family: Some("gemini".to_string()),
            release_date: None,
            attachment: None,
            reasoning: None,
            temperature: None,
            tool_call: None,
            interleaved: None,
            cost: Some(ModelCost {
                input: 0.075,
                output: 0.3,
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
impl Provider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
    }

    async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
        let model = request.model.to_string();
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            model, self.api_key
        );

        let google_req = self.build_request(&request);

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&google_req)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await?;
            return Err(ProviderError::api(status, body));
        }

        let google_resp: GoogleResponse = response.json().await?;

        let candidate = google_resp.candidates.first();
        // Gemini thinking models interleave `thought: true` parts with the
        // user-visible answer. Split text parts on the `thought` flag so
        // chain-of-thought lands in the reasoning channel and the content
        // string mirrors only what the model wants the user to see.
        let (content, reasoning) = match candidate {
            Some(c) => split_text_parts(&c.content.parts),
            None => (String::new(), None),
        };

        let tool_calls: Vec<ToolCall> = candidate
            .and_then(|c| {
                c.content
                    .parts
                    .iter()
                    .find_map(|p| p.function_call.as_ref())
            })
            .map(|fc| {
                vec![ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: fc.name.clone(),
                    arguments: serde_json::to_string(&fc.args).unwrap_or_default(),
                }]
            })
            .unwrap_or_default();

        let usage = google_resp
            .usage_metadata
            .map(|u| TokenUsage {
                input: u.prompt_token_count,
                output: u.candidates_token_count,
                cache_read: None,
                cache_write: None,
            })
            .unwrap_or_else(|| TokenUsage {
                input: 0,
                output: 0,
                cache_read: None,
                cache_write: None,
            });

        Ok(CompletionResponse {
            content,
            tool_calls,
            stop_reason: Some(
                candidate
                    .map(|c| c.finish_reason.clone())
                    .unwrap_or_default(),
            ),
            usage,
            model,
            reasoning,
        })
    }

    fn stream(&self, request: CompletionRequest) -> ProviderResult<EventStream> {
        let model = request.model.to_string();
        let api_key = self.api_key.clone();
        let google_req = self.build_request(&request);
        let client = self.client.clone();

        let stream = async_stream::try_stream! {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?key={}&alt=sse",
                model, api_key
            );

            let response = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .json(&google_req)
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
                    if line.is_empty() {
                        continue;
                    }

                    if let Some(event_data) = Self::parse_gemini_sse_line(line) {
                        if let Some(event) = Self::parse_gemini_event(&event_data) {
                            yield event;
                        }
                    }
                }
            }

            yield StreamEvent::message_stop(
                "STOP".to_string(),
                TokenUsage { input: 0, output: 0, cache_read: None, cache_write: None },
            );
        };

        Ok(Box::pin(stream))
    }

    fn models(&self) -> &[ModelInfo] {
        &GOOGLE_MODELS
    }

    fn default_model(&self) -> Option<&ModelInfo> {
        GOOGLE_MODELS.first()
    }
}

impl GoogleProvider {
    fn parse_gemini_sse_line(line: &str) -> Option<String> {
        if line.starts_with("data: ") {
            Some(line[6..].to_string())
        } else if line.starts_with("data:") {
            Some(line[5..].to_string())
        } else {
            None
        }
    }

    fn parse_gemini_event(data: &str) -> Option<StreamEvent> {
        #[derive(Debug, Deserialize)]
        struct GeminiStreamResponse {
            candidates: Vec<GeminiStreamCandidate>,
            usage_metadata: Option<GeminiStreamUsage>,
        }

        #[derive(Debug, Deserialize)]
        struct GeminiStreamCandidate {
            content: GeminiStreamContent,
            finish_reason: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        struct GeminiStreamContent {
            parts: Vec<GeminiStreamPart>,
        }

        #[derive(Debug, Deserialize)]
        struct GeminiStreamPart {
            text: Option<String>,
            function_call: Option<GeminiStreamFunctionCall>,
        }

        #[derive(Debug, Deserialize)]
        struct GeminiStreamFunctionCall {
            name: String,
            args: serde_json::Value,
        }

        #[derive(Debug, Deserialize)]
        struct GeminiStreamUsage {
            prompt_token_count: Option<u64>,
            candidates_token_count: Option<u64>,
            total_token_count: Option<u64>,
        }

        let response: GeminiStreamResponse = serde_json::from_str(data).ok()?;

        let candidate = response.candidates.first();
        let part = candidate.and_then(|c| c.content.parts.first());

        if let Some(text) = part.and_then(|p| p.text.clone()) {
            return Some(StreamEvent::text_delta(text));
        }

        if let Some(fc) = part.and_then(|p| p.function_call.as_ref()) {
            return Some(StreamEvent {
                event_type: "tool_call".to_string(),
                delta: None,
                tool_call: Some(ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: fc.name.clone(),
                    arguments: serde_json::to_string(&fc.args).unwrap_or_default(),
                }),
                stop_reason: None,
                usage: None,
            });
        }

        if let Some(finish_reason) = candidate.and_then(|c| c.finish_reason.clone()) {
            let usage = response
                .usage_metadata
                .map(|u| TokenUsage {
                    input: u.prompt_token_count.unwrap_or(0),
                    output: u.candidates_token_count.unwrap_or(0),
                    cache_read: None,
                    cache_write: None,
                })
                .unwrap_or_else(|| TokenUsage {
                    input: 0,
                    output: 0,
                    cache_read: None,
                    cache_write: None,
                });
            return Some(StreamEvent::message_stop(finish_reason, usage));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_part(text: &str, thought: bool) -> GoogleResponsePart {
        GoogleResponsePart {
            text: Some(text.to_string()),
            function_call: None,
            thought,
        }
    }

    #[test]
    fn split_routes_thought_text_to_reasoning_channel() {
        let parts = vec![
            text_part("thinking through", true),
            text_part("here is the answer", false),
        ];
        let (content, reasoning) = split_text_parts(&parts);
        assert_eq!(content, "here is the answer");
        assert_eq!(reasoning.as_deref(), Some("thinking through"));
    }

    #[test]
    fn split_joins_multiple_parts_with_newlines() {
        let parts = vec![
            text_part("step one", true),
            text_part("step two", true),
            text_part("answer a", false),
            text_part("answer b", false),
        ];
        let (content, reasoning) = split_text_parts(&parts);
        assert_eq!(content, "answer a\nanswer b");
        assert_eq!(reasoning.as_deref(), Some("step one\nstep two"));
    }

    #[test]
    fn split_returns_none_when_no_thought_parts_present() {
        let parts = vec![text_part("just an answer", false)];
        let (content, reasoning) = split_text_parts(&parts);
        assert_eq!(content, "just an answer");
        assert!(reasoning.is_none());
    }

    #[test]
    fn thought_flag_defaults_to_false_when_absent() {
        let raw = serde_json::json!({ "text": "answer" });
        let part: GoogleResponsePart = serde_json::from_value(raw).unwrap();
        assert!(!part.thought);
    }
}
