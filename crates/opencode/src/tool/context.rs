use serde::{Deserialize, Serialize};
use crate::id::{PartID, SessionID, MessageID};

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: SessionID,
    pub working_dir: std::path::PathBuf,
    pub permission_rules: crate::permission::Ruleset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<FilePart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    pub id: PartID,
    #[serde(rename = "sessionID")]
    pub session_id: SessionID,
    #[serde(rename = "messageID")]
    pub message_id: MessageID,
    pub mime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub url: String,
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