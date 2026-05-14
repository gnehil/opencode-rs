use std::collections::HashMap;

use crate::permission::{Action, PermissionRule};

use super::info::AgentInfo;
use super::mode::AgentMode;

fn default_permissions() -> Vec<PermissionRule> {
    vec![
        PermissionRule {
            permission: "*".to_string(),
            pattern: "*".to_string(),
            action: Action::Allow,
        },
        PermissionRule::ask_tool("doom_loop"),
        PermissionRule::deny_tool("question"),
        PermissionRule::deny_tool("plan_enter"),
        PermissionRule::deny_tool("plan_exit"),
        PermissionRule::deny_tool("repo_clone"),
        PermissionRule::deny_tool("repo_overview"),
        PermissionRule {
            permission: "read".to_string(),
            pattern: "*.env".to_string(),
            action: Action::Ask,
        },
        PermissionRule {
            permission: "read".to_string(),
            pattern: "*.env.*".to_string(),
            action: Action::Ask,
        },
        PermissionRule {
            permission: "read".to_string(),
            pattern: "*.env.example".to_string(),
            action: Action::Allow,
        },
    ]
}

fn build_permissions() -> Vec<PermissionRule> {
    let mut rules = default_permissions();
    rules.push(PermissionRule::allow_tool("question"));
    rules.push(PermissionRule::allow_tool("plan_enter"));
    rules
}

fn plan_permissions() -> Vec<PermissionRule> {
    let mut rules = default_permissions();
    rules.push(PermissionRule::allow_tool("question"));
    rules.push(PermissionRule::allow_tool("plan_exit"));
    rules.push(PermissionRule::deny_tool("edit"));
    let allow_plan_files = PermissionRule {
        permission: "edit".to_string(),
        pattern: ".opencode/plans/*.md".to_string(),
        action: Action::Allow,
    };
    let allow_data_plans = PermissionRule {
        permission: "edit".to_string(),
        pattern: "data/plans/*.md".to_string(),
        action: Action::Allow,
    };
    rules.push(allow_plan_files);
    rules.push(allow_data_plans);
    rules
}

fn general_permissions() -> Vec<PermissionRule> {
    let mut rules = default_permissions();
    rules.push(PermissionRule::deny_tool("todowrite"));
    rules
}

fn opts() -> HashMap<String, serde_json::Value> {
    HashMap::new()
}

pub fn build_agent() -> AgentInfo {
    AgentInfo {
        name: "build".to_string(),
        description: Some(
            "The default agent. Executes tools based on configured permissions.".to_string(),
        ),
        mode: AgentMode::Primary,
        native: Some(true),
        hidden: None,
        top_p: None,
        temperature: None,
        color: None,
        permission: build_permissions(),
        model: None,
        variant: None,
        prompt: None,
        options: opts(),
        steps: None,
    }
}

pub fn plan_agent() -> AgentInfo {
    AgentInfo {
        name: "plan".to_string(),
        description: Some("Plan mode. Disallows all edit tools.".to_string()),
        mode: AgentMode::Primary,
        native: Some(true),
        hidden: None,
        top_p: None,
        temperature: None,
        color: None,
        permission: plan_permissions(),
        model: None,
        variant: None,
        prompt: None,
        options: opts(),
        steps: None,
    }
}

pub fn general_agent() -> AgentInfo {
    AgentInfo {
        name: "general".to_string(),
        description: Some("General-purpose agent for researching complex questions and executing multi-step tasks.".to_string()),
        mode: AgentMode::Subagent,
        native: Some(true),
        hidden: None,
        top_p: None,
        temperature: None,
        color: None,
        permission: general_permissions(),
        model: None,
        variant: None,
        prompt: None,
        options: opts(),
        steps: None,
    }
}

pub fn explore_agent() -> AgentInfo {
    let desc = r#"Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns (eg. "src/components/**/*.tsx"), search code for keywords (eg. "API endpoints"), or answer questions about the codebase. When calling this agent, specify the desired thoroughness level: "quick" for basic searches, "medium" for moderate exploration, or "very thorough" for comprehensive analysis."#;
    let mut perm = default_permissions();
    perm.extend(vec![
        PermissionRule::deny_tool("*"),
        PermissionRule::allow_tool("grep"),
        PermissionRule::allow_tool("glob"),
        PermissionRule::allow_tool("list"),
        PermissionRule::allow_tool("bash"),
        PermissionRule::allow_tool("webfetch"),
        PermissionRule::allow_tool("websearch"),
        PermissionRule::allow_tool("read"),
    ]);
    AgentInfo {
        name: "explore".to_string(),
        description: Some(desc.to_string()),
        mode: AgentMode::Subagent,
        native: Some(true),
        hidden: None,
        top_p: None,
        temperature: None,
        color: None,
        permission: perm,
        model: None,
        variant: None,
        prompt: Some(super::prompts::PROMPT_EXPLORE.to_owned()),
        options: opts(),
        steps: None,
    }
}

pub fn scout_agent() -> AgentInfo {
    let desc = r#"Docs and dependency-source specialist. Use this when you need to inspect external documentation, clone dependency repositories into the managed cache, and research library implementation details without modifying the user's workspace."#;
    let mut perm = default_permissions();
    perm.extend(vec![
        PermissionRule::deny_tool("*"),
        PermissionRule::allow_tool("grep"),
        PermissionRule::allow_tool("glob"),
        PermissionRule::allow_tool("webfetch"),
        PermissionRule::allow_tool("websearch"),
        PermissionRule::allow_tool("read"),
        PermissionRule::allow_tool("repo_clone"),
        PermissionRule::allow_tool("repo_overview"),
    ]);
    AgentInfo {
        name: "scout".to_string(),
        description: Some(desc.to_string()),
        mode: AgentMode::Subagent,
        native: Some(true),
        hidden: None,
        top_p: None,
        temperature: None,
        color: None,
        permission: perm,
        model: None,
        variant: None,
        prompt: Some(super::prompts::PROMPT_SCOUT.to_owned()),
        options: opts(),
        steps: None,
    }
}

fn deny_all_permissions() -> Vec<PermissionRule> {
    vec![PermissionRule {
        permission: "*".to_string(),
        pattern: "*".to_string(),
        action: Action::Deny,
    }]
}

pub fn compaction_agent() -> AgentInfo {
    AgentInfo {
        name: "compaction".to_string(),
        description: None,
        mode: AgentMode::Primary,
        native: Some(true),
        hidden: Some(true),
        top_p: None,
        temperature: None,
        color: None,
        permission: deny_all_permissions(),
        model: None,
        variant: None,
        prompt: Some(super::prompts::PROMPT_COMPACTION.to_owned()),
        options: opts(),
        steps: None,
    }
}

pub fn title_agent() -> AgentInfo {
    AgentInfo {
        name: "title".to_string(),
        description: None,
        mode: AgentMode::Primary,
        native: Some(true),
        hidden: Some(true),
        top_p: None,
        temperature: Some(0.5),
        color: None,
        permission: deny_all_permissions(),
        model: None,
        variant: None,
        prompt: Some(super::prompts::PROMPT_TITLE.to_owned()),
        options: opts(),
        steps: None,
    }
}

pub fn summary_agent() -> AgentInfo {
    AgentInfo {
        name: "summary".to_string(),
        description: None,
        mode: AgentMode::Primary,
        native: Some(true),
        hidden: Some(true),
        top_p: None,
        temperature: None,
        color: None,
        permission: deny_all_permissions(),
        model: None,
        variant: None,
        prompt: Some(super::prompts::PROMPT_SUMMARY.to_owned()),
        options: opts(),
        steps: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(agent: &AgentInfo, permission: &str, pattern: &str) -> Action {
        crate::permission::evaluate(permission, pattern, std::slice::from_ref(&agent.permission))
            .action
    }

    #[test]
    fn build_permissions_match_default_tool_policy() {
        let agent = build_agent();

        assert_eq!(decision(&agent, "bash", "git status"), Action::Allow);
        assert_eq!(decision(&agent, "question", "*"), Action::Allow);
        assert_eq!(decision(&agent, "repo_clone", "*"), Action::Deny);
    }

    #[test]
    fn plan_permissions_are_read_only_with_plan_file_exception() {
        let agent = plan_agent();

        assert_eq!(decision(&agent, "read", "src/main.rs"), Action::Allow);
        assert_eq!(decision(&agent, "edit", "src/main.rs"), Action::Deny);
        assert_eq!(
            decision(&agent, "edit", ".opencode/plans/feature.md"),
            Action::Allow
        );
        assert_eq!(decision(&agent, "question", "*"), Action::Allow);
    }

    #[test]
    fn subagent_permissions_limit_tools_like_typescript() {
        let general = general_agent();
        assert_eq!(decision(&general, "bash", "git status"), Action::Allow);
        assert_eq!(decision(&general, "todowrite", "*"), Action::Deny);

        let explore = explore_agent();
        assert_eq!(decision(&explore, "read", "src/main.rs"), Action::Allow);
        assert_eq!(decision(&explore, "edit", "src/main.rs"), Action::Deny);
    }
}
