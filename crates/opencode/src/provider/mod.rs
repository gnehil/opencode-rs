mod alibaba;
mod anthropic;
mod azure;
mod bedrock;
mod cerebras;
mod cohere;
mod config;
mod copilot;
mod cost;
mod deepinfra;
mod deepseek;
mod fireworks;
mod gitlab;
mod google;
mod groq;
mod id;
mod limit;
mod lmstudio;
mod mistral;
mod modalities;
mod model;
mod ollama;
mod openai;
pub mod openai_sse;
mod openrouter;
mod options;
mod perplexity;
mod request;
mod response;
mod togetherai;
mod trait_;
mod venice;
mod vercel;
mod vertex;
mod xai;

pub use alibaba::AlibabaProvider;
pub use anthropic::AnthropicProvider;
pub use azure::AzureProvider;
pub use bedrock::BedrockProvider;
pub use cerebras::CerebrasProvider;
pub use cohere::CohereProvider;
pub use config::ProviderConfig;
pub use copilot::GitHubCopilotProvider;
pub use cost::{ModelCost, Over200kCost};
pub use deepinfra::DeepInfraProvider;
pub use deepseek::DeepSeekProvider;
pub use fireworks::FireworksProvider;
pub use gitlab::GitLabProvider;
pub use google::GoogleProvider;
pub use groq::GroqProvider;
pub use id::{ModelID, ProviderID};
pub use limit::ModelLimit;
pub use lmstudio::LMStudioProvider;
pub use mistral::MistralProvider;
pub use modalities::{Modality, ModelModalities};
pub use model::{
    InterleavedConfig, InterleavedDetails, InterleavedField, ModelInfo, ProviderRef, VariantConfig,
};
pub use ollama::OllamaProvider;
pub use openai::OpenAIProvider;
pub use openrouter::OpenRouterProvider;
pub use options::{ProviderOptions, Timeout};
pub use perplexity::PerplexityProvider;
pub use request::{CompletionMessage, CompletionRequest, ToolDefinition};
pub use response::{CompletionResponse, StreamEvent, TokenUsage, ToolCall};
pub use togetherai::TogetherAIProvider;
pub use trait_::{EventStream, Provider, ProviderError, ProviderResult};
pub use venice::VeniceProvider;
pub use vercel::VercelProvider;
pub use vertex::VertexProvider;
pub use xai::XAIProvider;

/// Serialize a `CompletionMessage` into the OpenAI chat-completions message
/// shape. Used by the OpenAI provider and every OpenAI-compatible provider
/// (Groq, Mistral, xAI, Together, etc.) so tool_calls / tool_call_id are
/// preserved on the wire instead of being stripped to a flat `{role, content}`.
///
/// Honours `msg.images`: if any are set, `content` is emitted as the
/// content-parts array form `[{type:"text", ...}, {type:"image_url", ...}]`
/// that OpenAI's vision models accept. Providers in the OpenAI-compat
/// ecosystem that don't have vision (Groq, Cerebras, etc.) will see the
/// array form too; some will silently drop the image parts, which is fine
/// — falling back to text-only content is acceptable behavior.
pub fn openai_compat_message_json(msg: &CompletionMessage) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "role".to_string(),
        serde_json::Value::String(msg.role.clone()),
    );
    let content = openai::openai_content_value(msg);
    if !content.is_null() {
        obj.insert("content".to_string(), content);
    }
    if let Some(tc) = &msg.tool_calls {
        obj.insert(
            "tool_calls".to_string(),
            serde_json::Value::Array(tc.clone()),
        );
    }
    if let Some(id) = &msg.tool_call_id {
        obj.insert(
            "tool_call_id".to_string(),
            serde_json::Value::String(id.clone()),
        );
    }
    serde_json::Value::Object(obj)
}

/// Extract the reasoning / "thinking" text from an OpenAI-compatible chat
/// completions JSON response, when the provider exposes one. Different
/// providers spell this field differently — DeepSeek uses
/// `reasoning_content`, OpenAI o1 uses `reasoning_summary`, OpenRouter
/// surfaces `reasoning` — so the helper checks each spelling in order and
/// returns the first non-empty string. Returns `None` when no reasoning
/// channel is present.
pub fn extract_openai_compat_reasoning(value: &serde_json::Value) -> Option<String> {
    let message = value.get("choices")?.get(0)?.get("message")?;
    for key in ["reasoning_content", "reasoning", "reasoning_summary"] {
        match message.get(key) {
            Some(serde_json::Value::String(text)) if !text.is_empty() => {
                return Some(text.clone());
            }
            Some(serde_json::Value::Array(items)) => {
                let joined: String = items
                    .iter()
                    .filter_map(|item| match item {
                        serde_json::Value::String(text) => Some(text.clone()),
                        serde_json::Value::Object(map) => map
                            .get("text")
                            .or_else(|| map.get("content"))
                            .or_else(|| map.get("summary"))
                            .and_then(|v| v.as_str().map(str::to_string)),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !joined.is_empty() {
                    return Some(joined);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod openai_compat_reasoning_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_reasoning_content_string() {
        let v = json!({
            "choices": [{ "message": { "content": "answer", "reasoning_content": "thinking" } }]
        });
        assert_eq!(
            extract_openai_compat_reasoning(&v).as_deref(),
            Some("thinking")
        );
    }

    #[test]
    fn extracts_reasoning_string() {
        let v = json!({
            "choices": [{ "message": { "content": "answer", "reasoning": "thinking" } }]
        });
        assert_eq!(
            extract_openai_compat_reasoning(&v).as_deref(),
            Some("thinking")
        );
    }

    #[test]
    fn joins_reasoning_summary_array_objects() {
        let v = json!({
            "choices": [{ "message": {
                "content": "answer",
                "reasoning_summary": [
                    { "text": "step one" },
                    { "text": "step two" }
                ]
            } }]
        });
        assert_eq!(
            extract_openai_compat_reasoning(&v).as_deref(),
            Some("step one\nstep two")
        );
    }

    #[test]
    fn none_when_no_reasoning_field_present() {
        let v = json!({
            "choices": [{ "message": { "content": "answer" } }]
        });
        assert!(extract_openai_compat_reasoning(&v).is_none());
    }

    #[test]
    fn skips_empty_reasoning_strings() {
        let v = json!({
            "choices": [{ "message": { "content": "answer", "reasoning_content": "" } }]
        });
        assert!(extract_openai_compat_reasoning(&v).is_none());
    }
}
