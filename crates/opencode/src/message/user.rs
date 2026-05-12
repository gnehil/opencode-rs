use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::id::{MessageID, SessionID};
use crate::message::format::OutputFormat;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default)]
    pub diffs: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
    pub id: MessageID,
    #[serde(rename = "sessionID")]
    pub session_id: SessionID,
    // The discriminant lives on the enum (`#[serde(tag = "role")]`); we keep
    // a local copy for ergonomics inside the struct but skip it on both
    // serialize and deserialize so the JSON has exactly one `role` key.
    #[serde(skip, default = "user_role_default")]
    pub role: String,
    pub time: UserTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<OutputFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<UserSummary>,
    pub agent: String,
    pub model: ModelRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<HashMap<String, bool>>,
}

impl Default for UserMessage {
    fn default() -> Self {
        Self {
            id: MessageID::new(),
            session_id: SessionID::new(),
            role: "user".to_string(),
            time: UserTime { created: 0 },
            format: None,
            summary: None,
            agent: String::new(),
            model: ModelRef {
                provider_id: String::new(),
                model_id: String::new(),
                variant: None,
            },
            system: None,
            tools: None,
        }
    }
}

fn user_role_default() -> String { "user".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTime {
    pub created: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    #[serde(rename = "providerID")]
    pub provider_id: String,
    #[serde(rename = "modelID")]
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}