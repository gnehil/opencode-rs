use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permission::PermissionRule;

use super::mode::AgentMode;
use super::model::AgentModel;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub description: Option<String>,
    pub mode: AgentMode,
    pub native: Option<bool>,
    pub hidden: Option<bool>,
    #[serde(rename = "topP")]
    pub top_p: Option<f64>,
    pub temperature: Option<f64>,
    pub color: Option<String>,
    pub permission: Vec<PermissionRule>,
    pub model: Option<AgentModel>,
    pub variant: Option<String>,
    pub prompt: Option<String>,
    pub options: HashMap<String, Value>,
    pub steps: Option<u32>,
}
