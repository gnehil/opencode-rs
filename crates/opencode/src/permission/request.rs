use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::id::MessageID;
use crate::id::SessionID;
use crate::permission::PermissionID;

/// Reference to a tool call that triggered this permission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRef {
    /// The message containing the tool call.
    pub message_id: MessageID,
    /// The tool call ID.
    pub call_id: String,
}

/// A permission request to be evaluated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    /// Unique request ID.
    pub id: PermissionID,
    /// The session this request belongs to.
    pub session_id: SessionID,
    /// The permission type being requested (e.g., "read", "edit", "bash").
    pub permission: String,
    /// The patterns/paths being accessed.
    pub patterns: Vec<String>,
    /// Additional metadata about the request.
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
    /// Patterns that should always use the same reply ("always" reply).
    #[serde(default)]
    pub always: Vec<String>,
    /// Reference to the tool call that triggered this request, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolRef>,
}
