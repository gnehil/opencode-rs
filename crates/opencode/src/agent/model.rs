use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModel {
    #[serde(rename = "modelID", alias = "model_id", alias = "modelId")]
    pub model_id: String,
    #[serde(rename = "providerID", alias = "provider_id", alias = "providerId")]
    pub provider_id: String,
}
