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
    let dirs = project_config_dirs(start);
    let mut merged = serde_json::Value::Object(serde_json::Map::new());
    let mut found = false;
    for dir in dirs {
        for config_dir in [dir.clone(), dir.join(".opencode")] {
            for name in ["opencode.json", "opencode.jsonc"] {
                let file = config_dir.join(name);
                if !file.is_file() {
                    continue;
                }
                let text = std::fs::read_to_string(&file)?;
                let value = parse_config_value(&text, &file)?;
                merge_json(&mut merged, value);
                found = true;
            }
        }

        let agents = load_agent_markdown_configs(&dir)?;
        if !agents.is_empty() {
            merge_json(
                &mut merged,
                serde_json::json!({
                    "agent": agents,
                }),
            );
            found = true;
        }
    }

    if found {
        Ok(Some(serde_json::from_value(merged)?))
    } else {
        Ok(None)
    }
}

fn project_config_dirs(start: &Path) -> Vec<PathBuf> {
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
    dirs
}

fn load_agent_markdown_configs(
    dir: &Path,
) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let mut agents = serde_json::Map::new();
    for base in [dir.to_path_buf(), dir.join(".opencode")] {
        load_agent_markdown_dir(&base.join("agent"), &mut agents, false, false)?;
        load_agent_markdown_dir(&base.join("agents"), &mut agents, false, false)?;
        load_agent_markdown_dir(&base.join("mode"), &mut agents, true, true)?;
        load_agent_markdown_dir(&base.join("modes"), &mut agents, true, true)?;
    }
    Ok(agents)
}

fn load_agent_markdown_dir(
    root: &Path,
    agents: &mut serde_json::Map<String, serde_json::Value>,
    force_primary_mode: bool,
    shallow: bool,
) -> anyhow::Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .max_depth(if shallow { 1 } else { usize::MAX })
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"));

    for entry in walker {
        let path = entry.path();
        let text = std::fs::read_to_string(path)?;
        let (frontmatter, prompt) = parse_markdown_config(&text)?;
        let mut object = frontmatter.as_object().cloned().unwrap_or_default();
        let path_name = markdown_entry_name(root, path);
        object
            .entry("name".to_string())
            .or_insert_with(|| serde_json::Value::String(path_name));
        if force_primary_mode {
            object.insert(
                "mode".to_string(),
                serde_json::Value::String("primary".to_string()),
            );
        }
        object.insert("prompt".to_string(), serde_json::Value::String(prompt));
        let name = object
            .get("name")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);
        if let Some(name) = name {
            agents.insert(name, serde_json::Value::Object(object));
        }
    }
    Ok(())
}

fn markdown_entry_name(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let without_ext = relative.with_extension("");
    without_ext
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn parse_markdown_config(text: &str) -> anyhow::Result<(serde_json::Value, String)> {
    let trimmed = text.trim();
    if !trimmed.starts_with("---") {
        return Ok((serde_json::json!({}), trimmed.to_string()));
    }
    let Some(end_marker_idx) = trimmed[3..].find("---") else {
        return Ok((serde_json::json!({}), trimmed.to_string()));
    };
    let frontmatter = trimmed[3..end_marker_idx + 3].trim();
    let body = trimmed[end_marker_idx + 6..].trim().to_string();
    let frontmatter = if frontmatter.is_empty() {
        serde_json::json!({})
    } else {
        serde_yaml::from_str(frontmatter)?
    };
    Ok((frontmatter, body))
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
                map.insert(
                    prop.name.value.as_ref().to_string(),
                    jsonc_to_json(prop.value)?,
                );
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask: Option<bool>,

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

/// A user-config plugin declaration. Matches the TypeScript
/// `ConfigPlugin.Spec`: either a bare identifier string, or a `[identifier,
/// options]` pair carrying inline options. The identifier is an npm package
/// spec or a path-like local spec (`./plugin.ts`, an absolute path, or a
/// `file://` URL).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginSpec {
    Bare(String),
    WithOptions(
        String,
        std::collections::BTreeMap<String, serde_json::Value>,
    ),
}

impl PluginSpec {
    /// The plugin identifier — answers "what should we load?".
    pub fn specifier(&self) -> &str {
        match self {
            PluginSpec::Bare(spec) => spec,
            PluginSpec::WithOptions(spec, _) => spec,
        }
    }

    /// Inline options attached to the spec, if any.
    pub fn options(&self) -> Option<&std::collections::BTreeMap<String, serde_json::Value>> {
        match self {
            PluginSpec::Bare(_) => None,
            PluginSpec::WithOptions(_, options) => Some(options),
        }
    }
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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<McpOAuthConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpOAuthConfig {
    Enabled(bool),
    Options(McpOAuthOptions),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthOptions {
    #[serde(rename = "clientId", skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,

    #[serde(rename = "clientSecret", skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,

    #[serde(rename = "redirectUri", skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
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
    pub image: Option<ImageAttachmentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAttachmentConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_resize: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_base64_bytes: Option<u64>,
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
            McpConfigEntry::Disabled { .. } => {
                panic!("enabled local MCP must not parse as disabled-only entry")
            }
        }
    }

    #[test]
    fn load_project_config_reads_jsonc_and_merges_parent_to_child() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let child_opencode = child.join(".opencode");
        std::fs::create_dir(&child_opencode).unwrap();

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
        std::fs::write(
            child_opencode.join("opencode.jsonc"),
            r#"{
              "mcp": {
                "local": {
                  "type": "local",
                  "command": "local-cmd"
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
        assert!(mcp.contains_key("local"));
    }

    #[test]
    fn load_project_config_discovers_agent_and_mode_markdown() {
        let temp = tempfile::tempdir().unwrap();
        let opencode = temp.path().join(".opencode");
        let agent_dir = opencode.join("agents").join("review");
        let mode_dir = opencode.join("mode");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(&mode_dir).unwrap();
        std::fs::write(
            agent_dir.join("security.md"),
            r#"---
description: Review security issues
mode: subagent
model: openai/gpt-4.1
permission:
  bash: deny
---
Inspect the code for security problems.
"#,
        )
        .unwrap();
        std::fs::write(
            mode_dir.join("architect.md"),
            r#"---
description: Architecture planning
---
Plan the implementation.
"#,
        )
        .unwrap();

        let config = load_project_config(temp.path())
            .unwrap()
            .expect("markdown agents should produce config");
        let agents = config.agent.unwrap();
        let security = agents.get("review/security").unwrap();
        assert_eq!(
            security.description.as_deref(),
            Some("Review security issues")
        );
        assert_eq!(security.mode.as_deref(), Some("subagent"));
        assert_eq!(security.model.as_deref(), Some("openai/gpt-4.1"));
        assert_eq!(
            security.prompt.as_deref(),
            Some("Inspect the code for security problems.")
        );
        assert_eq!(security.name.as_deref(), Some("review/security"));
        let permission = security.permission.as_ref().unwrap();
        match permission.rules.get("bash").unwrap() {
            PermissionRuleValue::Action(action) => assert_eq!(action, "deny"),
            PermissionRuleValue::Object(_) => panic!("bash should parse as shorthand action"),
        }

        let architect = agents.get("architect").unwrap();
        assert_eq!(architect.mode.as_deref(), Some("primary"));
        assert_eq!(
            architect.prompt.as_deref(),
            Some("Plan the implementation.")
        );
    }
}
