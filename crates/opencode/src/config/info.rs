use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<ServerConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<HashMap<String, CommandConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<SkillsConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<HashMap<String, ReferenceConfig>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher: Option<WatcherConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<Vec<PluginSpec>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub share: Option<ShareMode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoupdate: Option<AutoUpdateMode>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_providers: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_providers: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub small_model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<HashMap<String, AgentConfigEntry>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<HashMap<String, ProviderConfigEntry>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<HashMap<String, McpConfigEntry>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatter: Option<FormatterConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<LspConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<HashMap<String, bool>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<AttachmentConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise: Option<EnterpriseConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<ToolOutputConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<ExperimentalConfig>,
}

pub fn load_project_config(start: &Path) -> anyhow::Result<Option<Config>> {
    let files = project_config_files(start);
    if files.is_empty() {
        return Ok(None);
    }

    let mut merged = serde_json::Value::Object(serde_json::Map::new());
    for file in files {
        let text = std::fs::read_to_string(&file)?;
        let value = parse_config_value(&text, &file)?;
        merge_json(&mut merged, value);
    }

    Ok(Some(serde_json::from_value(merged)?))
}

fn project_config_files(start: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut current = if start.is_file() {
        start.parent().map(Path::to_path_buf)
    } else {
        Some(start.to_path_buf())
    };

    while let Some(dir) = current {
        dirs.push(dir.clone());
        current = dir.parent().map(Path::to_path_buf);
    }
    dirs.reverse();

    let mut files = Vec::new();
    for dir in dirs {
        for name in ["opencode.json", "opencode.jsonc"] {
            let path = dir.join(name);
            if path.is_file() {
                files.push(path);
            }
        }
    }
    files
}

fn parse_config_value(text: &str, source: &Path) -> anyhow::Result<serde_json::Value> {
    if source.extension().and_then(|e| e.to_str()) == Some("jsonc") {
        let parsed = jsonc_parser::parse_text(text)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {:?}", source.display(), e))?;
        match parsed.value {
            Some(value) => Ok(jsonc_to_json(value)?),
            None => Ok(serde_json::Value::Null),
        }
    } else {
        Ok(serde_json::from_str(text)?)
    }
}

fn jsonc_to_json(value: jsonc_parser::ast::Value) -> anyhow::Result<serde_json::Value> {
    use jsonc_parser::ast::Value;

    Ok(match value {
        Value::StringLit(v) => serde_json::Value::String(v.value.as_ref().to_string()),
        Value::NumberLit(v) => serde_json::from_str(v.value.as_ref())?,
        Value::BooleanLit(v) => serde_json::Value::Bool(v.value),
        Value::Object(v) => {
            let mut map = serde_json::Map::new();
            for prop in v.properties {
                map.insert(prop.name.value.as_ref().to_string(), jsonc_to_json(prop.value)?);
            }
            serde_json::Value::Object(map)
        }
        Value::Array(v) => serde_json::Value::Array(
            v.elements
                .into_iter()
                .map(jsonc_to_json)
                .collect::<anyhow::Result<Vec<_>>>()?,
        ),
        Value::NullKeyword(_) => serde_json::Value::Null,
    })
}

fn merge_json(target: &mut serde_json::Value, source: serde_json::Value) {
    match (target, source) {
        (serde_json::Value::Object(target), serde_json::Value::Object(source)) => {
            for (key, value) in source {
                match target.get_mut(&key) {
                    Some(existing) => merge_json(existing, value),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mdns: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mdns_domain: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cors: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirs: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareMode {
    Manual,
    Auto,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AutoUpdateMode {
    Bool(bool),
    Notify(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfigEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    #[serde(rename = "top_p", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, serde_json::Value>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfigEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub whitelist: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub blacklist: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ProviderOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<HashMap<String, ModelConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderOptions {
    #[serde(rename = "apiKey", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    #[serde(rename = "baseURL", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    #[serde(rename = "enterpriseUrl", skip_serializing_if = "Option::is_none")]
    pub enterprise_url: Option<String>,

    #[serde(rename = "setCacheKey", skip_serializing_if = "Option::is_none")]
    pub set_cache_key: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    #[serde(rename = "chunkTimeout", skip_serializing_if = "Option::is_none")]
    pub chunk_timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<bool>,

    #[serde(rename = "tool_call", skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<ModelLimit>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<ModelModalities>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    pub context: f64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<f64>,

    pub output: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelModalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpConfigEntry {
    Full(McpServerConfig),
    Disabled { enabled: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpCommand {
    String(String),
    Array(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<McpCommand>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    #[serde(alias = "environment", skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

impl McpServerConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn command_and_args(&self) -> Option<(String, Vec<String>)> {
        match self.command.as_ref()? {
            McpCommand::String(command) => {
                Some((command.clone(), self.args.clone().unwrap_or_default()))
            }
            McpCommand::Array(parts) => {
                let (command, rest) = parts.split_first()?;
                let mut args = rest.to_vec();
                if let Some(extra) = &self.args {
                    args.extend(extra.clone());
                }
                Some((command.clone(), args))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormatterConfig {
    Disabled(bool),
    Enabled(FormatterOptions),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prettier: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub rustfmt: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LspConfig {
    Disabled(bool),
    Enabled(LspOptions),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_analyzer: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionConfig {
    #[serde(flatten)]
    pub rules: HashMap<String, PermissionRuleValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PermissionRuleValue {
    Action(String),
    Object(HashMap<String, String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterpriseConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub prune: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_turns: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub preserve_recent_tokens: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_paste_summary: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_tool: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_telemetry: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_tools: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub continue_loop_on_deny: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_timeout: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_config_accepts_ts_local_command_shape() {
        let parsed: Config = serde_json::from_str(
            r#"{
              "mcp": {
                "playwright": {
                  "type": "local",
                  "command": ["npx", "-y", "@playwright/mcp"],
                  "environment": {
                    "DEBUG": "pw:mcp"
                  },
                  "enabled": true,
                  "timeout": 5000
                }
              }
            }"#,
        )
        .expect("TS opencode MCP local config should parse");

        let mcp = parsed.mcp.unwrap();
        let entry = mcp.get("playwright").expect("playwright MCP entry");
        match entry {
            McpConfigEntry::Full(server) => {
                assert_eq!(
                    server.command_and_args(),
                    Some((
                        "npx".to_string(),
                        vec!["-y".to_string(), "@playwright/mcp".to_string()]
                    ))
                );
                assert_eq!(
                    server.env.as_ref().and_then(|env| env.get("DEBUG")),
                    Some(&"pw:mcp".to_string())
                );
            }
            McpConfigEntry::Disabled { .. } => panic!("enabled local MCP must not parse as disabled-only entry"),
        }
    }

    #[test]
    fn load_project_config_reads_jsonc_and_merges_parent_to_child() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child");
        std::fs::create_dir(&child).unwrap();

        std::fs::write(
            temp.path().join("opencode.jsonc"),
            r#"{
              // parent config
              "mcp": {
                "parent": {
                  "type": "local",
                  "command": ["parent-cmd"]
                }
              }
            }"#,
        )
        .unwrap();
        std::fs::write(
            child.join("opencode.json"),
            r#"{
              "mcp": {
                "child": {
                  "type": "local",
                  "command": "child-cmd"
                }
              }
            }"#,
        )
        .unwrap();

        let config = load_project_config(&child)
            .unwrap()
            .expect("merged config should load");
        let mcp = config.mcp.unwrap();
        assert!(mcp.contains_key("parent"));
        assert!(mcp.contains_key("child"));
    }
}
