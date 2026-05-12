use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// Permission action: whether to allow, deny, or ask for a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, EnumString, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Action {
    Allow,
    Deny,
    Ask,
}
