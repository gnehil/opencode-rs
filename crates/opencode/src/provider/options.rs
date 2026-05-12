use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderOptions {
    #[serde(skip_serializing_if = "Option::is_none", rename = "apiKey")]
    pub api_key: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", rename = "baseURL")]
    pub base_url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", rename = "enterpriseUrl")]
    pub enterprise_url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", rename = "setCacheKey")]
    pub set_cache_key: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Timeout>,

    #[serde(skip_serializing_if = "Option::is_none", rename = "chunkTimeout")]
    pub chunk_timeout: Option<u64>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Timeout {
    Millis(u64),
    Disabled(bool),
}
