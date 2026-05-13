//! Shared SSE parser for OpenAI-shaped chat-completions streams.
//!
//! Most "OpenAI-compatible" hosted providers (Groq, Together, OpenRouter,
//! DeepInfra, Fireworks, Cerebras, Perplexity, Venice, Vercel, Mistral,
//! DeepSeek, Cohere, GitLab, LMStudio, Ollama, Copilot, Alibaba) speak
//! the same SSE protocol on `POST /chat/completions`:
//!
//!   data: {"id":"...","choices":[{"index":0,"delta":{"content":"hi"}}]}
//!   data: {"id":"...","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}
//!   data: [DONE]
//!
//! Before this module each provider stubbed `stream()` with
//! `Err(ProviderError::stream("streaming not implemented"))`. Now they
//! share `stream_openai_sse(url, headers, body)` which speaks the
//! protocol once and yields `StreamEvent`s.

use std::collections::HashMap;

use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;

use super::response::{StreamEvent, TokenUsage, ToolCall};
use super::trait_::{EventStream, ProviderError, ProviderResult};

/// Build the EventStream for a POST that yields OpenAI-shape SSE.
///
/// `headers` is an arbitrary HashMap of HTTP headers (the auth header
/// belongs here). `body` is the request payload serialized in advance
/// because each provider has its own request struct shape (model name,
/// max_tokens, etc.) but they all share the SSE wire format.
pub fn stream_openai_sse<B>(
    client: Client,
    url: String,
    headers: HashMap<String, String>,
    body: B,
) -> ProviderResult<EventStream>
where
    B: Serialize + Send + 'static,
{
    let stream = async_stream::try_stream! {
        let mut req = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&body);
        for (k, v) in headers {
            req = req.header(k, v);
        }

        let response = req.send().await?;
        let response = response.error_for_status()
            .map_err(|e| ProviderError::api(e.status().map(|s| s.as_u16()).unwrap_or(0), e.to_string()))?;

        let mut stream_reader = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream_reader.next().await.transpose()? {
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            // SSE frames are line-oriented; split on '\n' and re-park the
            // trailing partial line in the buffer for the next chunk.
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
                if let Some(event) = parse_chunk(data) {
                    yield event;
                }
            }
        }
    };

    Ok(Box::pin(stream))
}

/// Parse a single SSE `data:` chunk in OpenAI chat-completions schema.
/// Returns None for malformed payloads (we'd rather drop a frame than
/// abort the whole stream).
pub fn parse_chunk(data: &str) -> Option<StreamEvent> {
    #[derive(Deserialize)]
    struct Response {
        choices: Vec<Choice>,
        #[serde(default)]
        usage: Option<Usage>,
    }
    #[derive(Deserialize)]
    struct Choice {
        delta: Delta,
        #[serde(default)]
        finish_reason: Option<String>,
    }
    #[derive(Deserialize)]
    struct Delta {
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        tool_calls: Option<Vec<DeltaToolCall>>,
    }
    #[derive(Deserialize)]
    struct DeltaToolCall {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        function: Option<DeltaFunction>,
    }
    #[derive(Deserialize)]
    struct DeltaFunction {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        arguments: Option<String>,
    }
    #[derive(Deserialize)]
    struct Usage {
        #[serde(default)]
        prompt_tokens: u64,
        #[serde(default)]
        completion_tokens: u64,
    }

    let resp: Response = serde_json::from_str(data).ok()?;
    let choice = resp.choices.first()?;
    let finish_reason = choice.finish_reason.clone();
    let delta_content = choice.delta.content.clone();

    let tool_call = choice
        .delta
        .tool_calls
        .as_ref()
        .and_then(|tc| tc.first())
        .and_then(|t| {
            Some(ToolCall {
                id: t.id.clone()?,
                name: t.function.as_ref()?.name.clone()?,
                arguments: t.function.as_ref()?.arguments.clone().unwrap_or_default(),
            })
        });

    let usage = resp.usage.map(|u| TokenUsage {
        input: u.prompt_tokens,
        output: u.completion_tokens,
        cache_read: None,
        cache_write: None,
    });

    Some(StreamEvent {
        event_type: if finish_reason.is_some() {
            "message_stop"
        } else {
            "content_block_delta"
        }
        .to_string(),
        delta: delta_content,
        tool_call,
        stop_reason: finish_reason,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_delta() {
        let line = r#"{"choices":[{"index":0,"delta":{"content":"hi"}}]}"#;
        let ev = parse_chunk(line).unwrap();
        assert_eq!(ev.event_type, "content_block_delta");
        assert_eq!(ev.delta.as_deref(), Some("hi"));
    }

    #[test]
    fn parses_finish_reason() {
        let line = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        let ev = parse_chunk(line).unwrap();
        assert_eq!(ev.event_type, "message_stop");
        assert_eq!(ev.stop_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parses_tool_call_chunk() {
        let line = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"bash","arguments":"{}"}}]}}]}"#;
        let ev = parse_chunk(line).unwrap();
        assert!(ev.tool_call.is_some());
        let tc = ev.tool_call.unwrap();
        assert_eq!(tc.id, "call_1");
        assert_eq!(tc.name, "bash");
    }

    #[test]
    fn malformed_chunk_returns_none() {
        assert!(parse_chunk("not json").is_none());
    }
}
