use serde::{Deserialize, Serialize};

use super::id::ModelID;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional image attachments. Each entry is either an `https://` URL
    /// or a `data:image/<mime>;base64,<...>` data URL.
    ///
    /// Anthropic and OpenAI both support multi-modal input via content
    /// blocks; their respective providers serialize this field into the
    /// appropriate wire shape. Providers that don't support vision (most
    /// of the OpenAI-compatible ecosystem hosted by Cerebras / Together /
    /// Groq / etc.) silently drop it. We don't reject the request — the
    /// model just sees the text content without the image.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<CompletionMessage>,
    pub model: ModelID,
    pub tools: Vec<ToolDefinition>,
    pub system: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub top_p: Option<f64>,
    pub stop_sequences: Option<Vec<String>>,
    /// Extra HTTP headers to fold into the provider call. Populated by the
    /// `chat.headers` plugin hook; providers should layer these on top of
    /// their own required headers without overriding auth/content headers.
    #[allow(dead_code)]
    pub extra_headers: std::collections::HashMap<String, String>,
}
