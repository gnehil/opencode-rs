use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Time structure for running tool state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStateRunningTime {
    pub start: i64,
}

/// Time structure for completed/error tool state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStateEndedTime {
    pub start: i64,
    pub end: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacted: Option<i64>,
}

/// Pending tool call state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatePending {
    pub input: HashMap<String, serde_json::Value>,
    pub raw: String,
}

/// Running tool call state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStateRunning {
    pub input: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    pub time: ToolStateRunningTime,
}

/// Completed tool call state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStateCompleted {
    pub input: HashMap<String, serde_json::Value>,
    pub output: String,
    pub title: String,
    pub metadata: HashMap<String, serde_json::Value>,
    pub time: ToolStateEndedTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<crate::message::part::FilePart>>,
}

/// Error tool call state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStateError {
    pub input: HashMap<String, serde_json::Value>,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    pub time: ToolStateEndedTime,
}

/// Union enum for all tool states, tagged by "status"
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ToolState {
    Pending(ToolStatePending),
    Running(ToolStateRunning),
    Completed(ToolStateCompleted),
    Error(ToolStateError),
}
