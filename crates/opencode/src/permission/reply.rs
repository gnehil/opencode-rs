use serde::{Deserialize, Serialize};
use strum::EnumString;

/// Reply to a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Reply {
    Once,
    Always,
    Reject,
}
