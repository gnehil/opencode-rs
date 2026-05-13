mod default;
mod info;
mod mode;
mod model;
pub mod prompts;

use std::collections::BTreeMap;
use std::str::FromStr;

pub use default::*;
pub use info::*;
pub use mode::*;
pub use model::*;
pub use prompts::*;

pub const DEFAULT_AGENT_NAME: &str = "build";
const BUILTIN_AGENT_NAMES: &[&str] = &[
    "build",
    "plan",
    "general",
    "explore",
    "scout",
    "compaction",
    "title",
    "summary",
];

pub fn get_default_agent() -> AgentInfo {
    build_agent()
}

pub fn get_agent(name: &str) -> Option<AgentInfo> {
    match name {
        "build" => Some(build_agent()),
        "plan" => Some(plan_agent()),
        "general" => Some(general_agent()),
        "explore" => Some(explore_agent()),
        "scout" => Some(scout_agent()),
        "compaction" => Some(compaction_agent()),
        "title" => Some(title_agent()),
        "summary" => Some(summary_agent()),
        _ => None,
    }
}

pub fn list_agents(config: Option<&crate::config::Config>) -> Vec<AgentInfo> {
    let mut agents: BTreeMap<String, AgentInfo> = BUILTIN_AGENT_NAMES
        .iter()
        .filter_map(|name| get_agent(name).map(|agent| ((*name).to_string(), agent)))
        .collect();

    let global_permission = config
        .and_then(|config| config.permission.as_ref())
        .map(rules_from_permission_config)
        .unwrap_or_default();
    if !global_permission.is_empty() {
        for agent in agents.values_mut() {
            agent.permission.extend(global_permission.clone());
        }
    }

    if let Some(config_agents) = config.and_then(|config| config.agent.as_ref()) {
        for (key, entry) in config_agents {
            if entry.disable.unwrap_or(false) {
                agents.remove(key);
                continue;
            }

            let agent = agents.entry(key.clone()).or_insert_with(|| AgentInfo {
                name: key.clone(),
                description: None,
                mode: AgentMode::All,
                native: Some(false),
                hidden: None,
                top_p: None,
                temperature: None,
                color: None,
                permission: custom_agent_permissions(&global_permission),
                model: None,
                variant: None,
                prompt: None,
                options: std::collections::HashMap::new(),
                steps: None,
            });
            apply_agent_config(agent, entry);
        }
    }

    let default_agent = config
        .and_then(|config| config.default_agent.as_deref())
        .unwrap_or(DEFAULT_AGENT_NAME);
    let mut list = agents.into_values().collect::<Vec<_>>();
    list.sort_by(
        |a, b| match (a.name == default_agent, b.name == default_agent) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        },
    );
    list
}

fn apply_agent_config(agent: &mut AgentInfo, entry: &crate::config::AgentConfigEntry) {
    if let Some(model) = entry.model.as_deref() {
        agent.model = Some(parse_agent_model(model));
    }
    if let Some(variant) = &entry.variant {
        agent.variant = Some(variant.clone());
    }
    if let Some(prompt) = &entry.prompt {
        agent.prompt = Some(prompt.clone());
    }
    if let Some(description) = &entry.description {
        agent.description = Some(description.clone());
    }
    if let Some(temperature) = entry.temperature {
        agent.temperature = Some(temperature);
    }
    if let Some(top_p) = entry.top_p {
        agent.top_p = Some(top_p);
    }
    if let Some(mode) = entry
        .mode
        .as_deref()
        .and_then(|mode| AgentMode::from_str(mode).ok())
    {
        agent.mode = mode;
    }
    if let Some(color) = &entry.color {
        agent.color = Some(color.clone());
    }
    if let Some(hidden) = entry.hidden {
        agent.hidden = Some(hidden);
    }
    if let Some(name) = &entry.name {
        agent.name = name.clone();
    }
    if let Some(steps) = entry.steps {
        agent.steps = Some(steps);
    }
    if let Some(options) = &entry.options {
        merge_options(&mut agent.options, options.clone());
    }
    if let Some(permission) = &entry.permission {
        agent
            .permission
            .extend(rules_from_permission_config(permission));
    }
}

fn custom_agent_permissions(
    global_permission: &crate::permission::Ruleset,
) -> crate::permission::Ruleset {
    let mut rules = build_agent().permission;
    rules.extend(global_permission.to_vec());
    rules
}

fn parse_agent_model(model: &str) -> AgentModel {
    let mut parts = model.split('/');
    let provider_id = parts.next().unwrap_or_default().to_string();
    let model_id = parts.collect::<Vec<_>>().join("/");
    AgentModel {
        provider_id,
        model_id,
    }
}

fn rules_from_permission_config(
    config: &crate::config::PermissionConfig,
) -> crate::permission::Ruleset {
    config
        .rules
        .iter()
        .flat_map(|(permission, value)| match value {
            crate::config::PermissionRuleValue::Action(action) => parse_action(action)
                .map(|action| {
                    vec![crate::permission::PermissionRule {
                        permission: permission.clone(),
                        pattern: "*".to_string(),
                        action,
                    }]
                })
                .unwrap_or_default(),
            crate::config::PermissionRuleValue::Object(rules) => rules
                .iter()
                .filter_map(|(pattern, action)| {
                    parse_action(action).map(|action| crate::permission::PermissionRule {
                        permission: permission.clone(),
                        pattern: crate::permission::expand_pattern(pattern),
                        action,
                    })
                })
                .collect::<Vec<_>>(),
        })
        .collect()
}

fn parse_action(action: &str) -> Option<crate::permission::Action> {
    crate::permission::Action::from_str(action).ok()
}

fn merge_options(
    target: &mut std::collections::HashMap<String, serde_json::Value>,
    source: std::collections::HashMap<String, serde_json::Value>,
) {
    for (key, value) in source {
        match target.get_mut(&key) {
            Some(existing) => merge_json_value(existing, value),
            None => {
                target.insert(key, value);
            }
        }
    }
}

fn merge_json_value(target: &mut serde_json::Value, source: serde_json::Value) {
    match (target, source) {
        (serde_json::Value::Object(target), serde_json::Value::Object(source)) => {
            for (key, value) in source {
                match target.get_mut(&key) {
                    Some(existing) => merge_json_value(existing, value),
                    None => {
                        target.insert(key, value);
                    }
                }
            }
        }
        (target, source) => {
            *target = source;
        }
    }
}
