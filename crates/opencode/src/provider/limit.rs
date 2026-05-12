use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    pub context: f64,
    pub input: Option<f64>,
    pub output: f64,
}
