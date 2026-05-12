use serde::{Deserialize, Serialize};

use crate::id::{MessageID, SessionID};
use crate::message::error::AssistantError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantTime {
    pub created: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input: f64,
    pub output: f64,
    pub reasoning: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    pub cache: CacheUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheUsage {
    pub read: f64,
    pub write: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    pub cwd: String,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub id: MessageID,
    #[serde(rename = "sessionID")]
    pub session_id: SessionID,
    pub role: String,
    pub time: AssistantTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AssistantError>,
    #[serde(rename = "parentID")]
    pub parent_id: String,
    #[serde(rename = "modelID")]
    pub model_id: String,
    #[serde(rename = "providerID")]
    pub provider_id: String,
    pub mode: String,
    pub agent: String,
    pub path: PathInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<bool>,
    pub cost: f64,
    pub tokens: TokenUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
}

impl Default for AssistantMessage {
    fn default() -> Self {
        Self {
            id: MessageID::new(),
            session_id: SessionID::new(),
            role: "assistant".to_string(),
            time: AssistantTime {
                created: 0,
                completed: None,
            },
            error: None,
            parent_id: String::new(),
            model_id: String::new(),
            provider_id: String::new(),
            mode: String::new(),
            agent: String::new(),
            path: PathInfo {
                cwd: String::new(),
                root: String::new(),
            },
            summary: None,
            cost: 0.0,
            tokens: TokenUsage {
                input: 0.0,
                output: 0.0,
                reasoning: 0.0,
                total: None,
                cache: CacheUsage {
                    read: 0.0,
                    write: 0.0,
                },
            },
            structured: None,
            variant: None,
            finish: None,
        }
    }
}