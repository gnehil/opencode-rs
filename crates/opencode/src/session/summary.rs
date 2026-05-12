use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub additions: i64,
    pub deletions: i64,
    pub files: i64,
    pub diffs: Option<Vec<String>>,
}
