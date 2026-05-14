use std::collections::{HashMap, HashSet};

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

pub mod action;
pub mod arity;
pub mod broker;
pub mod error;
pub mod id;
pub mod reply;
pub mod request;
pub mod rule;

pub use action::Action;
pub use arity::split_commands;
pub use broker::PermissionBroker;
pub use error::PermissionError;
pub use id::PermissionID;
pub use reply::Reply;
pub use request::{PermissionRequest, ToolRef};
pub use rule::PermissionRule;

pub type Ruleset = Vec<PermissionRule>;

const EDIT_TOOLS: &[&str] = &["edit", "write", "apply_patch"];

pub fn expand_pattern(pattern: &str) -> String {
    if pattern == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| pattern.to_owned());
    }
    if let Some(rest) = pattern.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_owned());
        return format!("{}/{}", home, rest);
    }
    if let Some(rest) = pattern.strip_prefix("$HOME/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "$HOME".to_owned());
        return format!("{}/{}", home, rest);
    }
    if let Some(rest) = pattern.strip_prefix("$HOME") {
        let home = std::env::var("HOME").unwrap_or_else(|_| "$HOME".to_owned());
        return format!("{}{}", home, rest);
    }
    pattern.to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigPermissionValue {
    Shorthand(Action),
    Rules(HashMap<String, Action>),
}

pub fn from_config(config: HashMap<String, ConfigPermissionValue>) -> Ruleset {
    config
        .into_iter()
        .flat_map(|(permission, value)| match value {
            ConfigPermissionValue::Shorthand(action) => {
                vec![PermissionRule {
                    permission,
                    pattern: "*".to_owned(),
                    action,
                }]
            }
            ConfigPermissionValue::Rules(rules) => rules
                .into_iter()
                .map(|(pattern, action)| PermissionRule {
                    permission: permission.clone(),
                    pattern: expand_pattern(&pattern),
                    action,
                })
                .collect::<Vec<_>>(),
        })
        .collect()
}

pub fn merge(rulesets: &[Ruleset]) -> Ruleset {
    rulesets
        .iter()
        .flat_map(|ruleset| ruleset.iter().cloned())
        .collect()
}

pub fn evaluate(permission: &str, pattern: &str, rulesets: &[Ruleset]) -> PermissionRule {
    let mut result: Option<PermissionRule> = None;
    for ruleset in rulesets {
        for rule in ruleset {
            if wildcard_match(permission, &rule.permission)
                && wildcard_match(pattern, &rule.pattern)
            {
                result = Some(rule.clone());
            }
        }
    }
    result.unwrap_or_else(|| PermissionRule {
        permission: permission.to_owned(),
        pattern: pattern.to_owned(),
        action: Action::Ask,
    })
}

fn wildcard_match(input: &str, pattern: &str) -> bool {
    let input = input.replace('\\', "/");
    let pattern = pattern.replace('\\', "/");
    let mut escaped = String::new();
    for ch in pattern.chars() {
        match ch {
            '*' => escaped.push_str(".*"),
            '?' => escaped.push('.'),
            _ => escaped.push_str(&regex::escape(&ch.to_string())),
        }
    }
    if escaped.ends_with(" .*") {
        escaped.truncate(escaped.len() - 3);
        escaped.push_str("( .*)?");
    }
    RegexBuilder::new(&format!("^{escaped}$"))
        .case_insensitive(cfg!(windows))
        .dot_matches_new_line(true)
        .build()
        .map(|re| re.is_match(&input))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_permission_matches_any_tool() {
        let rules = vec![PermissionRule {
            permission: "*".to_string(),
            pattern: "*".to_string(),
            action: Action::Allow,
        }];

        let decision = evaluate("bash", "git status", &[rules]);

        assert_eq!(decision.action, Action::Allow);
        assert_eq!(decision.permission, "*");
    }

    #[test]
    fn later_specific_rule_overrides_wildcard_permission() {
        let rules = vec![
            PermissionRule {
                permission: "*".to_string(),
                pattern: "*".to_string(),
                action: Action::Allow,
            },
            PermissionRule {
                permission: "edit".to_string(),
                pattern: "*".to_string(),
                action: Action::Deny,
            },
        ];

        let decision = evaluate("edit", "src/main.rs", &[rules]);

        assert_eq!(decision.action, Action::Deny);
        assert_eq!(decision.permission, "edit");
    }

    #[test]
    fn wildcard_patterns_match_typescript_semantics() {
        assert!(wildcard_match("src/main.rs", "*"));
        assert!(wildcard_match("src/.env", "*.env"));
        assert!(wildcard_match("ls", "ls *"));
        assert!(wildcard_match("ls -la", "ls *"));
    }
}

pub fn disabled(tools: &[String], ruleset: &Ruleset) -> HashSet<String> {
    let mut result = HashSet::new();
    for tool in tools {
        let perm = if EDIT_TOOLS.contains(&tool.as_str()) {
            "edit"
        } else {
            tool.as_str()
        };
        if let Some(rule) = ruleset
            .iter()
            .rev()
            .find(|r| wildcard_match(perm, &r.permission))
        {
            if rule.pattern == "*" && rule.action == Action::Deny {
                result.insert(tool.clone());
            }
        }
    }
    result
}
