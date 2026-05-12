use serde::{Deserialize, Serialize};
use crate::id::{PartID, SessionID, MessageID};

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: SessionID,
    pub working_dir: std::path::PathBuf,
    pub permission_rules: crate::permission::Ruleset,
}

impl ToolContext {
    /// Resolve whether `permission` should be granted for `pattern`. Empty
    /// rulesets allow by default so that callers (CLI, tests) that haven't
    /// loaded any rules don't break. Deny rules always win over Allow.
    /// An `Ask` decision is returned as an error — the orchestrator is
    /// expected to surface a prompt and retry with a ruleset that resolves
    /// the gate.
    pub fn check_permission(&self, permission: &str, pattern: &str) -> anyhow::Result<()> {
        use crate::permission::Action;
        if self.permission_rules.is_empty() {
            return Ok(());
        }
        let decision = crate::permission::evaluate(
            permission,
            pattern,
            &[self.permission_rules.clone()],
        );
        match decision.action {
            Action::Allow => Ok(()),
            Action::Deny => Err(anyhow::anyhow!(
                "Tool '{}' denied by permission rule (pattern '{}')",
                permission,
                decision.pattern
            )),
            Action::Ask => Err(anyhow::anyhow!(
                "Tool '{}' requires explicit user approval for pattern '{}'",
                permission,
                pattern
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<FilePart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePart {
    pub id: PartID,
    #[serde(rename = "sessionID")]
    pub session_id: SessionID,
    #[serde(rename = "messageID")]
    pub message_id: MessageID,
    pub mime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub url: String,
}

impl ToolResult {
    pub fn text(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            attachments: None,
            metadata: None,
        }
    }

    pub fn with_metadata(output: impl Into<String>, metadata: serde_json::Value) -> Self {
        Self {
            output: output.into(),
            attachments: None,
            metadata: Some(metadata),
        }
    }

    pub fn with_attachments(output: impl Into<String>, attachments: Vec<FilePart>) -> Self {
        Self {
            output: output.into(),
            attachments: Some(attachments),
            metadata: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{Action, PermissionRule};

    fn ctx(rules: Vec<PermissionRule>) -> ToolContext {
        ToolContext {
            session_id: SessionID::new(),
            working_dir: std::path::PathBuf::from("/tmp"),
            permission_rules: rules,
        }
    }

    #[test]
    fn empty_ruleset_allows() {
        assert!(ctx(vec![]).check_permission("bash", "ls").is_ok());
    }

    #[test]
    fn explicit_deny_blocks() {
        let rule = PermissionRule {
            permission: "bash".to_string(),
            pattern: "rm *".to_string(),
            action: Action::Deny,
        };
        assert!(ctx(vec![rule]).check_permission("bash", "rm -rf /").is_err());
    }

    #[test]
    fn explicit_allow_passes() {
        let rule = PermissionRule {
            permission: "bash".to_string(),
            pattern: "ls *".to_string(),
            action: Action::Allow,
        };
        assert!(ctx(vec![rule]).check_permission("bash", "ls -la").is_ok());
    }

    #[test]
    fn no_match_with_nonempty_ruleset_asks() {
        // A non-empty ruleset that doesn't match the pattern should fall
        // back to Ask, which surfaces as an error to the tool.
        let rule = PermissionRule {
            permission: "bash".to_string(),
            pattern: "rm *".to_string(),
            action: Action::Deny,
        };
        let err = ctx(vec![rule])
            .check_permission("bash", "ls -la")
            .unwrap_err()
            .to_string();
        assert!(err.contains("explicit user approval"), "got: {err}");
    }
}