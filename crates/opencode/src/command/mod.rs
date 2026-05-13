use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::config::Config;

const INIT_TEMPLATE: &str = r#"Create or update `AGENTS.md` for this repository.

The goal is a compact instruction file that helps future OpenCode sessions avoid mistakes and ramp up quickly. Every line should answer: "Would an agent likely miss this without help?" If not, leave it out.

User-provided focus or constraints:
$ARGUMENTS

Read the highest-value project files first, prefer executable sources of truth over prose, preserve verified useful guidance, and avoid generic agent advice."#;

const REVIEW_TEMPLATE: &str = r#"You are a code reviewer. Review the requested changes and provide actionable feedback.

Input: $ARGUMENTS

If no arguments are provided, review uncommitted changes. If the input looks like a commit, branch, or PR, inspect that target. Focus on bugs, regressions, security issues, and missing tests before style concerns."#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask: Option<bool>,
    pub hints: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct MarkdownCommandFrontmatter {
    description: Option<String>,
    agent: Option<String>,
    model: Option<String>,
    subtask: Option<bool>,
}

pub fn load_commands(root: &Path, config: Option<&Config>) -> anyhow::Result<Vec<CommandInfo>> {
    let mut commands = BTreeMap::new();
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let root_display = root.to_string_lossy();

    insert_command(
        &mut commands,
        CommandInfo::new(
            "init",
            INIT_TEMPLATE.replace("${path}", &root_display),
            Some("guided AGENTS.md setup".to_string()),
            None,
            None,
            Some("command".to_string()),
            None,
        ),
    );
    insert_command(
        &mut commands,
        CommandInfo::new(
            "review",
            REVIEW_TEMPLATE.replace("${path}", &root_display),
            Some("review changes [commit|branch|pr], defaults to uncommitted".to_string()),
            None,
            None,
            Some("command".to_string()),
            Some(true),
        ),
    );

    if let Some(config) = config {
        if let Some(config_commands) = &config.command {
            for (name, command) in config_commands {
                if let Some(template) = command.template.as_ref().or(command.run.as_ref()) {
                    insert_command(
                        &mut commands,
                        CommandInfo::new(
                            name,
                            template.clone(),
                            command.description.clone(),
                            command.agent.clone(),
                            command.model.clone(),
                            Some("command".to_string()),
                            command.subtask,
                        ),
                    );
                }
            }
        }
    }

    for command in load_markdown_commands(&root)? {
        insert_command(&mut commands, command);
    }

    Ok(commands.into_values().collect())
}

pub fn render_template(template: &str, arguments: &str) -> String {
    let args = parse_arguments(arguments);
    let placeholders: Vec<usize> = placeholder_regex()
        .captures_iter(template)
        .filter_map(|captures| captures.get(1)?.as_str().parse::<usize>().ok())
        .collect();
    let last = placeholders.iter().copied().max().unwrap_or(0);
    let with_numbered = placeholder_regex().replace_all(template, |captures: &regex::Captures| {
        let position = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<usize>().ok())
            .unwrap_or(0);
        if position == 0 {
            return String::new();
        }
        let arg_index = position - 1;
        if arg_index >= args.len() {
            return String::new();
        }
        if position == last {
            return args[arg_index..].join(" ");
        }
        args[arg_index].clone()
    });

    let uses_arguments_placeholder = template.contains("$ARGUMENTS");
    let mut rendered = with_numbered.replace("$ARGUMENTS", arguments);
    if placeholders.is_empty() && !uses_arguments_placeholder && !arguments.trim().is_empty() {
        rendered.push_str("\n\n");
        rendered.push_str(arguments.trim());
    }
    rendered.trim().to_string()
}

impl CommandInfo {
    fn new(
        name: impl Into<String>,
        template: String,
        description: Option<String>,
        agent: Option<String>,
        model: Option<String>,
        source: Option<String>,
        subtask: Option<bool>,
    ) -> Self {
        let hints = hints(&template);
        Self {
            name: name.into(),
            description,
            agent,
            model,
            source,
            template,
            subtask,
            hints,
        }
    }
}

fn insert_command(commands: &mut BTreeMap<String, CommandInfo>, command: CommandInfo) {
    commands.insert(command.name.clone(), command);
}

fn hints(template: &str) -> Vec<String> {
    let mut result = BTreeSet::new();
    for captures in placeholder_regex().captures_iter(template) {
        if let Some(value) = captures.get(0) {
            result.insert(value.as_str().to_string());
        }
    }
    let mut result = result.into_iter().collect::<Vec<_>>();
    if template.contains("$ARGUMENTS") {
        result.push("$ARGUMENTS".to_string());
    }
    result
}

fn parse_arguments(arguments: &str) -> Vec<String> {
    argument_regex()
        .find_iter(arguments)
        .map(|value| trim_quotes(value.as_str()).to_string())
        .collect()
}

fn trim_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn load_markdown_commands(root: &Path) -> anyhow::Result<Vec<CommandInfo>> {
    let mut commands = Vec::new();
    for relative_root in [
        ".opencode/command",
        ".opencode/commands",
        "command",
        "commands",
    ] {
        let dir = root.join(relative_root);
        if !dir.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&dir)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("md") {
                continue;
            }
            commands.push(markdown_command(&dir, path)?);
        }
    }
    Ok(commands)
}

fn markdown_command(root: &Path, path: &Path) -> anyhow::Result<CommandInfo> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter, body) = parse_frontmatter(&content)?;
    let name = command_name(root, path);
    Ok(CommandInfo::new(
        name,
        body.trim().to_string(),
        frontmatter.description,
        frontmatter.agent,
        frontmatter.model,
        Some("command".to_string()),
        frontmatter.subtask,
    ))
}

fn parse_frontmatter(content: &str) -> anyhow::Result<(MarkdownCommandFrontmatter, String)> {
    if !content.starts_with("---") {
        return Ok((MarkdownCommandFrontmatter::default(), content.to_string()));
    }
    let rest = content
        .strip_prefix("---\r\n")
        .or_else(|| content.strip_prefix("---\n"));
    let Some(rest) = rest else {
        return Ok((MarkdownCommandFrontmatter::default(), content.to_string()));
    };
    let marker = "\n---";
    let Some(end) = rest.find(marker) else {
        return Ok((MarkdownCommandFrontmatter::default(), content.to_string()));
    };
    let yaml = &rest[..end];
    let body = rest[end + marker.len()..]
        .strip_prefix("\r\n")
        .or_else(|| rest[end + marker.len()..].strip_prefix('\n'))
        .unwrap_or(&rest[end + marker.len()..]);
    let frontmatter = serde_yaml::from_str(yaml)?;
    Ok((frontmatter, body.to_string()))
}

fn command_name(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let without_extension = relative.with_extension("");
    normalize_path(without_extension)
}

fn normalize_path(path: PathBuf) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn argument_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        RegexBuilder::new(r#"(?:\[Image\s+\d+\]|"[^"]*"|'[^']*'|[^\s"']+)"#)
            .case_insensitive(true)
            .build()
            .expect("valid command argument regex")
    })
}

fn placeholder_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\$(\d+)").expect("valid command placeholder regex"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::{CommandConfig, Config};

    #[test]
    fn render_template_matches_typescript_argument_expansion() {
        let rendered = super::render_template(
            "Review $1, compare $2, all=$ARGUMENTS",
            r#"src/lib.rs "main branch" extra"#,
        );

        assert_eq!(
            rendered,
            "Review src/lib.rs, compare main branch extra, all=src/lib.rs \"main branch\" extra"
        );
    }

    #[test]
    fn render_template_appends_arguments_when_template_has_no_placeholders() {
        assert_eq!(
            super::render_template("Review this", "--cached"),
            "Review this\n\n--cached"
        );
    }

    #[test]
    fn load_commands_merges_builtins_config_and_markdown_commands() {
        let temp = tempfile::tempdir().unwrap();
        let command_dir = temp.path().join(".opencode").join("commands");
        std::fs::create_dir_all(&command_dir).unwrap();
        std::fs::write(
            command_dir.join("audit.md"),
            "---\ndescription: Audit changed files\nagent: reviewer\n---\nAudit $ARGUMENTS\n",
        )
        .unwrap();

        let config = Config {
            command: Some(HashMap::from([(
                "deploy".to_string(),
                CommandConfig {
                    template: Some("Deploy $1 to $2".to_string()),
                    description: Some("Deploy service".to_string()),
                    agent: Some("build".to_string()),
                    model: Some("openai/gpt-4o".to_string()),
                    subtask: Some(false),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        };

        let commands = super::load_commands(temp.path(), Some(&config)).unwrap();

        assert!(commands.iter().any(|command| command.name == "init"));
        let deploy = commands
            .iter()
            .find(|command| command.name == "deploy")
            .expect("config command");
        assert_eq!(deploy.template, "Deploy $1 to $2");
        assert_eq!(deploy.hints, vec!["$1", "$2"]);
        assert_eq!(deploy.model.as_deref(), Some("openai/gpt-4o"));

        let audit = commands
            .iter()
            .find(|command| command.name == "audit")
            .expect("markdown command");
        assert_eq!(audit.description.as_deref(), Some("Audit changed files"));
        assert_eq!(audit.agent.as_deref(), Some("reviewer"));
        assert_eq!(audit.template, "Audit $ARGUMENTS");
    }
}
