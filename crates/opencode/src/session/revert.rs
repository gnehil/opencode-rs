use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRevert {
    #[serde(rename = "messageID", alias = "messageId")]
    pub message_id: String,
    #[serde(rename = "partID", alias = "partId")]
    pub part_id: Option<String>,
    pub snapshot: Option<String>,
    pub diff: Option<String>,
}
