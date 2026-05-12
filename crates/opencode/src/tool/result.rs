use serde::{Deserialize, Serialize};

use crate::message::part::FilePart;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<FilePart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl ToolResult {
    pub fn text(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            attachments: None,
            metadata: None,
        }
    }

    pub fn with_metadata(output: impl Into<String>, metadata: serde_json::Value) -> Self {
        Self {
            output: output.into(),
            attachments: None,
            metadata: Some(metadata),
        }
    }

    pub fn with_attachments(output: impl Into<String>, attachments: Vec<FilePart>) -> Self {
        Self {
            output: output.into(),
            attachments: Some(attachments),
            metadata: None,
        }
    }
}
