use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::model::ModelInfo;
use super::options::ProviderOptions;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub whitelist: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub blacklist: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ProviderOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, ModelInfo>>,
}
