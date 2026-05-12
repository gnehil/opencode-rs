use serde::{Deserialize, Serialize};

use crate::permission::action::Action;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub permission: String,
    pub pattern: String,
    pub action: Action,
}

impl PermissionRule {
    pub fn allow_tool(name: &str) -> Self {
        Self {
            permission: name.to_owned(),
            pattern: "*".to_owned(),
            action: Action::Allow,
        }
    }

    pub fn deny_tool(name: &str) -> Self {
        Self {
            permission: name.to_owned(),
            pattern: "*".to_owned(),
            action: Action::Deny,
        }
    }

    pub fn ask_tool(name: &str) -> Self {
        Self {
            permission: name.to_owned(),
            pattern: "*".to_owned(),
            action: Action::Ask,
        }
    }
}
