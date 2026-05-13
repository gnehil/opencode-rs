use serde::{Deserialize, Serialize};

/// Text output format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputFormatText {
    #[serde(rename = "type")]
    pub type_: String,
}

impl Default for OutputFormatText {
    fn default() -> Self {
        Self {
            type_: "text".to_string(),
        }
    }
}

/// JSON Schema output format
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFormatJsonSchema {
    #[serde(rename = "type")]
    pub type_: String,
    pub schema: serde_json::Value,
    #[serde(default = "default_retry_count")]
    pub retry_count: u32,
}

fn default_retry_count() -> u32 {
    2
}

/// Union enum for output formats, tagged by "type"
/// The variant names are automatically serialized as the "type" value
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    #[serde(rename = "json_schema")]
    JsonSchema {
        schema: serde_json::Value,
        #[serde(default = "default_retry_count", rename = "retryCount")]
        retry_count: u32,
    },
}
