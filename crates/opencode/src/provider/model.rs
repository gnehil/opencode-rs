use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cost::ModelCost;
use super::id::ModelID;
use super::limit::ModelLimit;
use super::modalities::ModelModalities;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<ModelID>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", rename = "release_date")]
    pub release_date: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none", rename = "tool_call")]
    pub tool_call: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub interleaved: Option<InterleavedConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<ModelLimit>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<ModelModalities>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderRef>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub variants: Option<HashMap<String, VariantConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InterleavedConfig {
    Enabled(bool),
    Details(InterleavedDetails),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterleavedDetails {
    pub field: InterleavedField,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterleavedField {
    ReasoningContent,
    ReasoningDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VariantConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,

    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}
