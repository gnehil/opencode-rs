use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModel {
    pub model_id: String,
    pub provider_id: String,
}
