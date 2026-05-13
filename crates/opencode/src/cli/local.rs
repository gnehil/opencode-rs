use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::{Column, Row};

use crate::cli::{args, local_process, mcp_cli};
use crate::mcp::{McpAuthStore, McpManager, McpServerStatus};
use crate::message::{Message, Part};
use crate::session::SessionStore;
use crate::storage::{MessageRow, PartRow, SessionRow};

pub(crate) fn limit_rows<T>(rows: Vec<T>, max: Option<usize>) -> Vec<T> {
    match max {
        Some(max) => rows.into_iter().take(max).collect(),
        None => rows,
    }
}

pub(crate) fn format_query_rows(
    columns: &[String],
    rows: &[Vec<String>],
    format: args::DbFormat,
) -> Result<String> {
    match format {
        args::DbFormat::Tsv => {
            let mut out = String::new();
            out.push_str(&columns.join("\t"));
            out.push('\n');
            for row in rows {
                let clean = row
                    .iter()
                    .map(|value| value.replace(['\t', '\n', '\r'], " "))
                    .collect::<Vec<_>>();
                out.push_str(&clean.join("\t"));
                out.push('\n');
            }
            Ok(out)
        }
        args::DbFormat::Json => {
            let objects = rows
                .iter()
                .map(|row| {
                    let mut object = serde_json::Map::new();
                    for (idx, column) in columns.iter().enumerate() {
                        object.insert(
                            column.clone(),
                            serde_json::Value::String(row.get(idx).cloned().unwrap_or_default()),
                        );
                    }
                    serde_json::Value::Object(object)
                })
                .collect::<Vec<_>>();
            Ok(format!("{}\n", serde_json::to_string_pretty(&objects)?))
        }
    }
}

pub(crate) fn agent_markdown(
    description: &str,
    mode: Option<args::AgentMode>,
    model: Option<&str>,
    permissions: Option<&str>,
) -> String {
    let mode = match mode.unwrap_or(args::AgentMode::All) {
        args::AgentMode::All => "all",
        args::AgentMode::Primary => "primary",
        args::AgentMode::Subagent => "subagent",
    };

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("description: {}\n", description));
    out.push_str(&format!("mode: {}\n", mode));
    if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
        out.push_str(&format!("model: {}\n", model.trim()));
    }
    if let Some(permissions) = permissions.filter(|p| !p.trim().is_empty()) {
        out.push_str("permission:\n");
        for permission in permissions
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            out.push_str(&format!("  {}: allow\n", permission));
        }
    }
    out.push_str("---\n\n");
    out.push_str(description);
    out.push('\n');
    out
}

pub(crate) fn handle_generate() -> Result<()> {
    let routes = [
        "/health",
        "/session/{id}/message",
        "/event",
        "/config",
        "/provider",
        "/file",
        "/find",
        "/mcp",
        "/agent",
        "/permission",
        "/workspace",
    ];
    let spec = serde_json::json!({
        "openapi": "3.1.0",
        "info": {
            "title": "opencode-rs local API",
            "version": env!("CARGO_PKG_VERSION")
        },
        "paths": routes.iter().map(|route| {
            ((*route).to_string(), serde_json::json!({}))
        }).collect::<serde_json::Map<_, _>>()
    });
    println!("{}", serde_json::to_string_pretty(&spec)?);
    Ok(())
}

pub(crate) fn handle_console(subcommand: args::ConsoleSubcommand) -> Result<()> {
    let name = match subcommand {
        args::ConsoleSubcommand::Login(_) => "login",
        args::ConsoleSubcommand::Logout(_) => "logout",
        args::ConsoleSubcommand::Switch => "switch",
        args::ConsoleSubcommand::Orgs => "orgs",
        args::ConsoleSubcommand::Open => "open",
    };
    println!(
        "Console {} is a cloud feature and is skipped in this local build.",
        name
    );
    Ok(())
}

pub(crate) async fn handle_agent(subcommand: args::AgentSubcommand) -> Result<()> {
    match subcommand {
        args::AgentSubcommand::List => {
            let cwd = std::env::current_dir()?;
            let config = crate::config::load_project_config(&cwd)?;
            let mut agents = crate::agent::list_agents(config.as_ref());
            agents.sort_by(|a, b| {
                let a_native = a.native.unwrap_or(false);
                let b_native = b.native.unwrap_or(false);
                match (a_native, b_native) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                }
            });
            for agent in agents {
                println!("{} ({})", agent.name, agent.mode);
                println!("  {}", serde_json::to_string_pretty(&agent.permission)?);
            }
        }
        args::AgentSubcommand::Create(create) => {
            let description = create
                .description
                .as_deref()
                .unwrap_or("Custom opencode agent");
            let content = agent_markdown(
                description,
                create.mode,
                create.model.as_deref(),
                create.permissions.as_deref(),
            );
            let path = agent_output_path(create.path.as_deref(), description)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if path.exists() {
                anyhow::bail!("agent file already exists: {}", path.display());
            }
            std::fs::write(&path, content)?;
            println!("{}", path.display());
        }
    }
    Ok(())
}

fn agent_output_path(path: Option<&str>, description: &str) -> Result<PathBuf> {
    let slug = slugify(description);
    let path = match path {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(".opencode")
            .join("agent")
            .join(format!("{}.md", slug)),
    };
    if path.extension().is_some() {
        Ok(path)
    } else {
        Ok(path.join(format!("{}.md", slug)))
    }
}

fn slugify(input: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for c in input.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "agent".to_string()
    } else {
        slug.to_string()
    }
}

pub(crate) async fn handle_stats(args: args::StatsArgs, data_dir: PathBuf) -> Result<()> {
    let store = SessionStore::new(data_dir).await?;
    let cutoff = args
        .days
        .map(|days| chrono::Utc::now().timestamp_millis() - (days as i64 * 24 * 60 * 60 * 1000));
    let project_filter = args
        .project
        .as_deref()
        .and_then(|project| (!project.is_empty()).then_some(project));
    let sessions = store.list(project_filter).await?;
    let sessions = sessions
        .into_iter()
        .filter(|session| {
            cutoff
                .map(|cutoff| session.time_created >= cutoff)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    let mut summary = StatsSummary::default();
    let mut models: BTreeMap<String, ModelStats> = BTreeMap::new();
    let mut tools: BTreeMap<String, usize> = BTreeMap::new();

    summary.sessions = sessions.len();
    for session in &sessions {
        let session_id = crate::id::SessionID::parse(&session.id)
            .map_err(|_| anyhow::anyhow!("invalid session id in database: {}", session.id))?;
        let messages = store.get_messages(&session_id).await?;
        summary.messages += messages.len();
        for message in messages {
            if let Message::Assistant(assistant) = message {
                summary.input_tokens += assistant.tokens.input;
                summary.output_tokens += assistant.tokens.output;
                let model = models.entry(assistant.model_id).or_default();
                model.requests += 1;
                model.input_tokens += assistant.tokens.input;
                model.output_tokens += assistant.tokens.output;
            }
        }
        for parts in store.get_parts_by_session(&session_id).await?.into_values() {
            for part in parts {
                if let Part::Tool(tool) = part {
                    *tools.entry(tool.tool).or_default() += 1;
                    summary.tool_calls += 1;
                }
            }
        }
    }

    println!("Sessions: {}", summary.sessions);
    println!("Messages: {}", summary.messages);
    println!("Input tokens: {}", summary.input_tokens);
    println!("Output tokens: {}", summary.output_tokens);
    println!("Tool calls: {}", summary.tool_calls);
    if args.models.is_some() {
        println!("\nModels:");
        for (model, stats) in models {
            println!(
                "  {} - requests={}, input={}, output={}",
                model, stats.requests, stats.input_tokens, stats.output_tokens
            );
        }
    }
    if let Some(limit) = args.tools {
        println!("\nTools:");
        for (tool, count) in tools.into_iter().take(limit) {
            println!("  {} - {}", tool, count);
        }
    }
    Ok(())
}

#[derive(Default)]
struct StatsSummary {
    sessions: usize,
    messages: usize,
    input_tokens: f64,
    output_tokens: f64,
    tool_calls: usize,
}

#[derive(Default)]
struct ModelStats {
    requests: usize,
    input_tokens: f64,
    output_tokens: f64,
}

pub(crate) async fn handle_debug(subcommand: args::DebugSubcommand) -> Result<()> {
    match subcommand {
        args::DebugSubcommand::Config => {
            let cwd = std::env::current_dir()?;
            match crate::config::load_project_config(&cwd)? {
                Some(config) => println!("{}", serde_json::to_string_pretty(&config)?),
                None => println!("{} has no opencode.json or opencode.jsonc", cwd.display()),
            }
        }
        args::DebugSubcommand::Info => handle_debug_info()?,
        args::DebugSubcommand::Paths => print!("{}", format_debug_paths(&debug_path_rows())),
        args::DebugSubcommand::Wait => std::future::pending::<()>().await,
        args::DebugSubcommand::Skill => handle_debug_skill().await?,
        args::DebugSubcommand::Startup => {
            println!("{}", Instant::now().elapsed().as_secs_f64() * 1000.0)
        }
        args::DebugSubcommand::Agent(args) => handle_debug_agent(args).await?,
        args::DebugSubcommand::Rg { subcommand } => handle_debug_rg(subcommand)?,
        args::DebugSubcommand::File { subcommand } => handle_debug_file(subcommand)?,
        args::DebugSubcommand::Lsp { subcommand } => handle_debug_lsp(subcommand).await?,
        args::DebugSubcommand::Snapshot { subcommand } => handle_debug_snapshot(subcommand)?,
        args::DebugSubcommand::Symbols(args) => {
            handle_debug_lsp(args::DebugLspSubcommand::Symbols(args)).await?
        }
        args::DebugSubcommand::DocumentSymbols(args) => {
            handle_debug_lsp(args::DebugLspSubcommand::DocumentSymbols(args)).await?
        }
        args::DebugSubcommand::Diagnostics(args) => {
            handle_debug_lsp(args::DebugLspSubcommand::Diagnostics(args)).await?
        }
    }
    Ok(())
}

fn handle_debug_info() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = crate::config::load_project_config(&cwd)?.unwrap_or_default();
    let term_program = std::env::var("TERM_PROGRAM").ok().map(|program| {
        std::env::var("TERM_PROGRAM_VERSION")
            .ok()
            .filter(|version| !version.is_empty())
            .map(|version| format!("{} {}", program, version))
            .unwrap_or(program)
    });
    let terminal = [term_program, std::env::var("TERM").ok()]
        .into_iter()
        .flatten()
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");

    println!("opencode version: {}", env!("CARGO_PKG_VERSION"));
    println!(
        "os: {} {} {}",
        std::env::consts::OS,
        os_release(),
        std::env::consts::ARCH
    );
    println!(
        "terminal: {}",
        if terminal.is_empty() {
            "unknown"
        } else {
            terminal.as_str()
        }
    );
    println!("plugins:");
    match config.plugin {
        Some(plugins) if !plugins.is_empty() => {
            for plugin in plugins {
                println!("- {}", plugin_specifier(&plugin));
            }
        }
        _ => println!("none"),
    }
    Ok(())
}

fn os_release() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .and_then(|output| output.status.success().then_some(output.stdout))
        .and_then(|stdout| String::from_utf8(stdout).ok())
        .map(|release| release.trim().to_string())
        .filter(|release| !release.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn plugin_specifier(plugin: &crate::config::PluginSpec) -> String {
    plugin
        .name
        .clone()
        .or_else(|| plugin.path.clone())
        .or_else(|| plugin.url.clone())
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn debug_path_rows() -> Vec<(&'static str, PathBuf)> {
    vec![
        ("home", crate::global::home().to_path_buf()),
        ("data", crate::global::data().to_path_buf()),
        ("config", crate::global::config().to_path_buf()),
        ("cache", crate::global::cache().to_path_buf()),
        ("state", crate::global::state().to_path_buf()),
        ("tmp", crate::global::tmp().to_path_buf()),
        ("bin", crate::global::bin().to_path_buf()),
        ("log", crate::global::log().to_path_buf()),
        ("repos", crate::global::repos().to_path_buf()),
    ]
}

fn format_debug_paths(rows: &[(&str, PathBuf)]) -> String {
    let mut out = String::new();
    for (key, value) in rows {
        out.push_str(&format!("{:<10} {}\n", key, value.display()));
    }
    out
}

async fn handle_debug_skill() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let service = crate::skill::SkillService::new();
    service.discover(&cwd).await?;
    let mut skills = service.all().await;
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    println!("{}", serde_json::to_string_pretty(&skills)?);
    Ok(())
}

async fn handle_debug_agent(debug_args: args::DebugAgentArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = crate::config::load_project_config(&cwd)?;
    let Some(agent) = crate::agent::resolve_agent(&debug_args.name, config.as_ref()) else {
        anyhow::bail!(
            "Agent {} not found, run 'opencode agent list' to get an agent list",
            debug_args.name
        );
    };

    let tools = crate::tool::default_registry();
    let tool_ids = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    let disabled = crate::permission::disabled(&tool_ids, &agent.permission);
    let resolved_tools = tool_ids
        .iter()
        .map(|tool| (tool.clone(), !disabled.contains(tool)))
        .collect::<BTreeMap<_, _>>();

    if let Some(tool_id) = debug_args.tool {
        let Some(tool) = tools.iter().find(|tool| tool.name() == tool_id) else {
            anyhow::bail!("Tool {} not found for agent {}", tool_id, agent.name);
        };
        if disabled.contains(&tool_id) {
            anyhow::bail!("Tool {} is disabled for agent {}", tool_id, agent.name);
        }
        let params = parse_debug_tool_params(debug_args.params.as_deref())?;
        let ctx = crate::tool::ToolContext {
            session_id: crate::id::SessionID::new(),
            working_dir: cwd,
            permission_rules: agent.permission.clone(),
            event_bus: None,
            permission_broker: None,
        };
        let result = tool.execute(params.clone(), ctx).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "tool": tool_id,
                "input": params,
                "result": result,
            }))?
        );
        return Ok(());
    }

    let mut output = serde_json::to_value(&agent)?;
    if let Some(object) = output.as_object_mut() {
        object.insert("tools".to_string(), serde_json::to_value(resolved_tools)?);
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn parse_debug_tool_params(input: Option<&str>) -> Result<serde_json::Value> {
    let Some(input) = input.map(str::trim).filter(|input| !input.is_empty()) else {
        return Ok(serde_json::json!({}));
    };
    let value: serde_json::Value =
        serde_json::from_str(input).with_context(|| "failed to parse --params as JSON object")?;
    if !value.is_object() {
        anyhow::bail!("Tool params must be an object.");
    }
    Ok(value)
}

fn handle_debug_rg(subcommand: args::DebugRgSubcommand) -> Result<()> {
    match subcommand {
        args::DebugRgSubcommand::Tree(args) => {
            let files = rg_files(None, None, None)?;
            println!("{}", format_rg_tree(&files, args.limit));
        }
        args::DebugRgSubcommand::Files(args) => {
            for file in rg_files(args.glob.as_deref(), args.query.as_deref(), args.limit)? {
                println!("{}", file);
            }
        }
        args::DebugRgSubcommand::Search(args) => {
            let items = rg_search(&args.pattern, &args.glob, args.limit)?;
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
    }
    Ok(())
}

fn build_rg_search_args(pattern: &str, globs: &[String], limit: Option<usize>) -> Vec<String> {
    let mut args = vec![
        "--no-config".to_string(),
        "--json".to_string(),
        "--hidden".to_string(),
        "--glob".to_string(),
        "!.git/*".to_string(),
        "--no-messages".to_string(),
    ];
    for glob in globs {
        args.push("--glob".to_string());
        args.push(glob.clone());
    }
    if let Some(limit) = limit {
        args.push("--max-count".to_string());
        args.push(limit.to_string());
    }
    args.push("--".to_string());
    args.push(pattern.to_string());
    args.push(".".to_string());
    args
}

fn build_rg_files_args(glob: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--no-config".to_string(),
        "--files".to_string(),
        "--hidden".to_string(),
        "--glob".to_string(),
        "!.git/*".to_string(),
    ];
    if let Some(glob) = glob.filter(|glob| !glob.is_empty()) {
        args.push("--glob".to_string());
        args.push(glob.to_string());
    }
    args.push(".".to_string());
    args
}

fn rg_files(glob: Option<&str>, query: Option<&str>, limit: Option<usize>) -> Result<Vec<String>> {
    let output = run_command_capture(
        "rg",
        &build_rg_files_args(glob),
        &[0, 1],
        "failed to run rg files",
    )?;
    let query = query.map(|query| query.to_ascii_lowercase());
    let files = output
        .lines()
        .map(clean_rg_path)
        .filter(|file| {
            query
                .as_deref()
                .map(|query| file.to_ascii_lowercase().contains(query))
                .unwrap_or(true)
        })
        .take(limit.unwrap_or(usize::MAX))
        .collect();
    Ok(files)
}

fn rg_search(
    pattern: &str,
    globs: &[String],
    limit: Option<usize>,
) -> Result<Vec<serde_json::Value>> {
    let args = build_rg_search_args(pattern, globs, limit);
    let output = run_command_capture("rg", &args, &[0, 1, 2], "failed to run rg search")?;
    let mut items = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("failed to parse rg JSON line: {}", line))?;
        if value.get("type").and_then(|v| v.as_str()) != Some("match") {
            continue;
        }
        if let Some(mut data) = value.get("data").cloned() {
            clean_rg_match_path(&mut data);
            items.push(data);
            if limit.map(|limit| items.len() >= limit).unwrap_or(false) {
                break;
            }
        }
    }
    Ok(items)
}

fn clean_rg_path(path: &str) -> String {
    path.trim_start_matches("./").to_string()
}

fn clean_rg_match_path(value: &mut serde_json::Value) {
    let cleaned = value
        .get("path")
        .and_then(|path| path.get("text"))
        .and_then(|text| text.as_str())
        .map(clean_rg_path);
    if let Some(cleaned) = cleaned {
        if let Some(text) = value.get_mut("path").and_then(|path| path.get_mut("text")) {
            *text = serde_json::Value::String(cleaned);
        }
    }
}

fn format_rg_tree(files: &[String], limit: Option<usize>) -> String {
    let mut dirs = BTreeMap::<String, ()>::new();
    for file in files {
        if file.contains(".opencode") {
            continue;
        }
        let path = Path::new(file);
        let mut current = PathBuf::new();
        let components = path.components().collect::<Vec<_>>();
        if components.len() < 2 {
            continue;
        }
        for component in &components[..components.len() - 1] {
            current.push(component.as_os_str());
            dirs.insert(current.to_string_lossy().replace('\\', "/"), ());
        }
    }
    let total = dirs.len();
    let limit = limit.unwrap_or(total);
    let mut lines = dirs.keys().take(limit).cloned().collect::<Vec<_>>();
    if total > lines.len() {
        lines.push(format!("[{} truncated]", total - lines.len()));
    }
    lines.join("\n")
}

fn handle_debug_file(subcommand: args::DebugFileSubcommand) -> Result<()> {
    match subcommand {
        args::DebugFileSubcommand::Read(args) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&read_debug_file(&PathBuf::from(args.path))?)?
            );
        }
        args::DebugFileSubcommand::Status => {
            println!("{}", serde_json::to_string_pretty(&debug_file_status()?)?);
        }
        args::DebugFileSubcommand::List(args) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&list_debug_files(&PathBuf::from(args.path))?)?
            );
        }
        args::DebugFileSubcommand::Search(args) => {
            let files = rg_files(None, Some(&args.query), Some(100))?;
            for file in files {
                println!("{}", file);
            }
        }
        args::DebugFileSubcommand::Tree(args) => {
            let tree = collect_file_tree(&PathBuf::from(args.dir), Some(200))?;
            println!("{}", format_file_tree(&tree)?);
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct DebugFileContent {
    #[serde(rename = "type")]
    kind: &'static str,
    content: String,
}

fn read_debug_file(path: &Path) -> Result<DebugFileContent> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    match String::from_utf8(bytes) {
        Ok(content) => Ok(DebugFileContent {
            kind: "text",
            content: content.trim().to_string(),
        }),
        Err(_) => Ok(DebugFileContent {
            kind: "binary",
            content: String::new(),
        }),
    }
}

#[derive(Serialize)]
struct DebugFileInfo {
    path: String,
    added: usize,
    removed: usize,
    status: &'static str,
}

fn debug_file_status() -> Result<Vec<DebugFileInfo>> {
    if run_command_capture(
        "git",
        &["rev-parse".into(), "--is-inside-work-tree".into()],
        &[0],
        "failed to check git status",
    )
    .is_err()
    {
        return Ok(Vec::new());
    }

    let mut changed = Vec::new();
    let diff = run_command_capture(
        "git",
        &[
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.quotepath=false".into(),
            "diff".into(),
            "--numstat".into(),
            "HEAD".into(),
        ],
        &[0],
        "failed to read git diff status",
    )?;
    for line in diff.lines().filter(|line| !line.trim().is_empty()) {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() < 3 {
            continue;
        }
        changed.push(DebugFileInfo {
            path: parts[2].to_string(),
            added: parts[0].parse().unwrap_or(0),
            removed: parts[1].parse().unwrap_or(0),
            status: "modified",
        });
    }

    let untracked = run_command_capture(
        "git",
        &[
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.quotepath=false".into(),
            "ls-files".into(),
            "--others".into(),
            "--exclude-standard".into(),
        ],
        &[0],
        "failed to read git untracked status",
    )?;
    for file in untracked.lines().filter(|line| !line.trim().is_empty()) {
        let added = std::fs::read_to_string(file)
            .map(|content| content.lines().count())
            .unwrap_or(0);
        changed.push(DebugFileInfo {
            path: file.to_string(),
            added,
            removed: 0,
            status: "added",
        });
    }

    let deleted = run_command_capture(
        "git",
        &[
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.quotepath=false".into(),
            "diff".into(),
            "--name-only".into(),
            "--diff-filter=D".into(),
            "HEAD".into(),
        ],
        &[0],
        "failed to read git deleted status",
    )?;
    for file in deleted.lines().filter(|line| !line.trim().is_empty()) {
        changed.push(DebugFileInfo {
            path: file.to_string(),
            added: 0,
            removed: 0,
            status: "deleted",
        });
    }

    Ok(changed)
}

#[derive(Serialize)]
struct DebugFileNode {
    name: String,
    path: String,
    absolute: String,
    #[serde(rename = "type")]
    kind: &'static str,
    ignored: bool,
}

fn list_debug_files(path: &Path) -> Result<Vec<DebugFileNode>> {
    let cwd = std::env::current_dir()?;
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut nodes = Vec::new();
    for entry in
        std::fs::read_dir(&full).with_context(|| format!("failed to list {}", full.display()))?
    {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".git" || name == ".DS_Store" {
            continue;
        }
        let absolute = entry.path();
        let relative = absolute.strip_prefix(&cwd).unwrap_or(&absolute);
        let is_dir = entry.file_type()?.is_dir();
        nodes.push(DebugFileNode {
            name,
            path: relative.to_string_lossy().to_string(),
            absolute: absolute.to_string_lossy().to_string(),
            kind: if is_dir { "directory" } else { "file" },
            ignored: false,
        });
    }
    nodes.sort_by(|a, b| {
        b.kind
            .cmp(a.kind)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(nodes)
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum FileTreeKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize)]
struct FileTreeEntry {
    path: PathBuf,
    #[serde(rename = "type")]
    kind: FileTreeKind,
}

fn collect_file_tree(dir: &Path, limit: Option<usize>) -> Result<Vec<FileTreeEntry>> {
    let cwd = std::env::current_dir()?;
    let root = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        cwd.join(dir)
    };
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_entry(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| name != ".git" && name != ".DS_Store")
                .unwrap_or(true)
        })
        .filter_map(|entry| entry.ok())
        .skip(1)
    {
        let relative = entry
            .path()
            .strip_prefix(&root)
            .unwrap_or(entry.path())
            .to_path_buf();
        entries.push(FileTreeEntry {
            path: relative,
            kind: if entry.file_type().is_dir() {
                FileTreeKind::Directory
            } else {
                FileTreeKind::File
            },
        });
        if limit.map(|limit| entries.len() >= limit).unwrap_or(false) {
            break;
        }
    }
    Ok(entries)
}

fn format_file_tree(tree: &[FileTreeEntry]) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(tree)?))
}

fn run_command_capture(
    program: &str,
    args: &[String],
    ok_codes: &[i32],
    context: &str,
) -> Result<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| context.to_string())?;
    let code = output.status.code().unwrap_or(-1);
    if !ok_codes.contains(&code) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "{} exited with {}{}",
            program,
            code,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn handle_debug_lsp(subcommand: args::DebugLspSubcommand) -> Result<()> {
    let root = std::env::current_dir()?;
    match subcommand {
        args::DebugLspSubcommand::Diagnostics(args) => {
            let path = PathBuf::from(args.file);
            for item in crate::lsp::diagnostics::fetch(&path, &root).await? {
                println!("{}", item.format_line(&path));
            }
        }
        args::DebugLspSubcommand::DocumentSymbols(args) => {
            let path = path_from_uri_or_path(&args.uri);
            for symbol in crate::lsp::ops::document_symbols(&path, &root).await? {
                println!("{}", symbol.format());
            }
        }
        args::DebugLspSubcommand::Symbols(args) => {
            let seed = find_lsp_seed(&root).ok_or_else(|| {
                anyhow::anyhow!("no LSP-supported file found under {}", root.display())
            })?;
            for symbol in crate::lsp::ops::workspace_symbols(&root, &seed, &args.query).await? {
                println!("{}", symbol.format());
            }
        }
    }
    Ok(())
}

fn path_from_uri_or_path(input: &str) -> PathBuf {
    input
        .strip_prefix("file://")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(input))
}

fn find_lsp_seed(root: &Path) -> Option<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .find(|path| crate::lsp::server_for_path(path).is_some())
}

fn handle_debug_snapshot(subcommand: args::DebugSnapshotSubcommand) -> Result<()> {
    match subcommand {
        args::DebugSnapshotSubcommand::Track => {
            let out = run_command_capture(
                "git",
                &["rev-parse".into(), "HEAD".into()],
                &[0],
                "failed to track local git snapshot",
            )?;
            print!("{}", out);
        }
        args::DebugSnapshotSubcommand::Patch(args) => {
            let out = run_command_capture(
                "git",
                &[
                    "show".into(),
                    "--format=medium".into(),
                    "--patch".into(),
                    args.hash,
                ],
                &[0],
                "failed to show local git snapshot patch",
            )?;
            print!("{}", out);
        }
        args::DebugSnapshotSubcommand::Diff(args) => {
            let out = run_command_capture(
                "git",
                &["diff".into(), args.hash],
                &[0],
                "failed to show local git snapshot diff",
            )?;
            print!("{}", out);
        }
        args::DebugSnapshotSubcommand::List => {
            println!("Local snapshot storage is not enabled in opencode-rs.");
        }
        args::DebugSnapshotSubcommand::Show(args) => {
            println!(
                "Local snapshot '{}' is not available in opencode-rs.",
                args.hash
            );
        }
    }
    Ok(())
}

pub(crate) async fn handle_mcp(subcommand: args::McpSubcommand, data_dir: PathBuf) -> Result<()> {
    match subcommand {
        args::McpSubcommand::List => {
            let (manager, config) = start_mcp_from_project(&data_dir).await?;
            print_mcp_status(&manager);
            if config.mcp.is_none() {
                println!("No MCP servers configured.");
            }
        }
        args::McpSubcommand::Debug(args) => {
            let (manager, _) = start_mcp_from_project(&data_dir).await?;
            let Some(client) = manager.get_client(&args.name) else {
                anyhow::bail!("MCP server '{}' is not connected", args.name);
            };
            println!("Tools:");
            for tool in client.list_tools().await? {
                println!("  {} - {}", tool.name, tool.description);
            }
            println!("Resources:");
            for resource in client.list_resources().await? {
                println!("  {} - {}", resource.uri, resource.name);
            }
        }
        args::McpSubcommand::Auth(auth) => {
            if matches!(auth.subcommand, Some(args::McpAuthSubcommand::List)) || auth.name.is_none()
            {
                let store = McpAuthStore::new(data_dir);
                store.load().await?;
                for (name, entry) in store.all().await {
                    let status = if entry.tokens.is_some() {
                        "authenticated"
                    } else {
                        "pending"
                    };
                    println!("{} - {}", name, status);
                }
            } else if let Some(name) = auth.name {
                let cwd = std::env::current_dir()?;
                let config = crate::config::load_project_config(&cwd)?.unwrap_or_default();
                let target = mcp_cli::mcp_auth_target(&config, &name)?;
                let client = reqwest::Client::new();
                let metadata = mcp_cli::discover_oauth_metadata(&client, &target.url).await?;
                let session = mcp_cli::start_mcp_oauth(data_dir, target, metadata).await?;
                println!("Open this URL to authenticate MCP server '{}':", name);
                println!("{}", session.start.authorization_url);
                open_url(&session.start.authorization_url);
                let code = session
                    .callback_server
                    .wait_for_callback(&session.start.state, &name)
                    .await?;
                mcp_cli::complete_mcp_oauth(&session, &code).await?;
                println!("Saved MCP OAuth credentials for {}.", name);
            }
        }
        args::McpSubcommand::Logout(args) => {
            let store = McpAuthStore::new(data_dir);
            store.load().await?;
            if let Some(name) = args.name {
                store.remove(&name).await?;
                println!("Removed MCP credentials for {}", name);
            } else {
                for name in store.all().await.keys().cloned().collect::<Vec<_>>() {
                    store.remove(&name).await?;
                }
                println!("Removed all MCP credentials");
            }
        }
        args::McpSubcommand::Add => {
            let name = prompt_required("MCP server name")?;
            let scope = prompt_default("Scope [local/global]", "local")?;
            let global = scope.eq_ignore_ascii_case("global");
            let kind = prompt_default("Type [local/remote]", "local")?;
            let spec = if kind.eq_ignore_ascii_case("remote") {
                let url = prompt_required("Remote URL")?;
                let oauth = match prompt_default("OAuth [default/none/dynamic/client]", "default")?
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "none" | "disabled" | "false" => mcp_cli::McpOAuthChoice::Disabled,
                    "dynamic" => mcp_cli::McpOAuthChoice::Dynamic,
                    "client" => {
                        let client_id = prompt_required("OAuth client id")?;
                        let client_secret = prompt_optional("OAuth client secret")?;
                        mcp_cli::McpOAuthChoice::Client {
                            client_id,
                            client_secret,
                        }
                    }
                    _ => mcp_cli::McpOAuthChoice::Default,
                };
                mcp_cli::McpAddSpec::Remote { url, oauth }
            } else {
                let command = prompt_required("Command")?;
                let command = command
                    .split_whitespace()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>();
                mcp_cli::McpAddSpec::Local { command }
            };
            let entry = mcp_cli::mcp_config_value(spec)?;
            let base_dir = if global {
                directories::ProjectDirs::from("com", "opencode", "opencode")
                    .map(|dirs| dirs.config_dir().to_path_buf())
                    .ok_or_else(|| anyhow::anyhow!("could not resolve global config directory"))?
            } else {
                std::env::current_dir()?
            };
            let path = mcp_cli::resolve_mcp_config_path(&base_dir, global);
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            let updated = mcp_cli::upsert_mcp_config_text(&existing, &name, entry)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, updated)?;
            println!("Added MCP server '{}' to {}.", name, path.display());
        }
    }
    Ok(())
}

async fn start_mcp_from_project(data_dir: &Path) -> Result<(McpManager, crate::config::Config)> {
    let cwd = std::env::current_dir()?;
    let config = crate::config::load_project_config(&cwd)?.unwrap_or_default();
    let auth_store = std::sync::Arc::new(McpAuthStore::new(data_dir.to_path_buf()));
    let mut manager = McpManager::new().with_auth_store(auth_store);
    manager.start_configured(&config).await;
    Ok((manager, config))
}

fn print_mcp_status(manager: &McpManager) {
    for (name, status) in manager.status() {
        match status {
            McpServerStatus::Connected => println!("{} - connected", name),
            McpServerStatus::Disabled => println!("{} - disabled", name),
            McpServerStatus::Failed { error } => println!("{} - failed: {}", name, error),
            McpServerStatus::NeedsAuth => println!("{} - needs authentication", name),
            McpServerStatus::NeedsClientRegistration { error } => {
                println!("{} - needs client registration: {}", name, error)
            }
        }
    }
}

fn prompt_required(label: &str) -> Result<String> {
    let value = prompt_default(label, "")?;
    if value.trim().is_empty() {
        anyhow::bail!("{label} is required");
    }
    Ok(value)
}

fn prompt_optional(label: &str) -> Result<Option<String>> {
    let value = prompt_default(label, "")?;
    Ok((!value.trim().is_empty()).then_some(value))
}

fn prompt_default(label: &str, default: &str) -> Result<String> {
    print!("{}: ", label);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_string();
    if input.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input)
    }
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let command = ("open", vec![url]);
    #[cfg(target_os = "linux")]
    let command = ("xdg-open", vec![url]);
    #[cfg(target_os = "windows")]
    let command = ("cmd", vec!["/C", "start", url]);

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        let _ = Command::new(command.0).args(command.1).spawn();
    }
}

pub(crate) async fn handle_export(args: args::ExportArgs, data_dir: PathBuf) -> Result<()> {
    let store = SessionStore::new(data_dir).await?;
    let session = match args.session_id {
        Some(id) => {
            let session_id = crate::id::SessionID::parse(&id)?;
            store
                .get(&session_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("session not found: {}", id))?
        }
        None => store
            .list(None)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no sessions to export"))?,
    };
    let session_id = crate::id::SessionID::parse(&session.id)?;
    let messages = sqlx::query_as::<_, MessageRow>(
        "SELECT * FROM message WHERE session_id = ?1 ORDER BY time_created ASC, id ASC",
    )
    .bind(&session.id)
    .fetch_all(store.pool.as_ref())
    .await?;
    let parts = sqlx::query_as::<_, PartRow>(
        "SELECT * FROM part WHERE session_id = ?1 ORDER BY time_created ASC, id ASC",
    )
    .bind(session_id.to_string())
    .fetch_all(store.pool.as_ref())
    .await?;
    let bundle = ExportBundle {
        session,
        messages,
        parts,
        sanitized: args.sanitize,
    };
    println!("{}", serde_json::to_string_pretty(&bundle)?);
    Ok(())
}

#[derive(Serialize, serde::Deserialize)]
struct ExportBundle {
    session: SessionRow,
    messages: Vec<MessageRow>,
    parts: Vec<PartRow>,
    sanitized: bool,
}

pub(crate) async fn handle_import(args: args::ImportArgs, data_dir: PathBuf) -> Result<()> {
    let text = std::fs::read_to_string(&args.file)?;
    let bundle: ExportBundle = serde_json::from_str(&text)?;
    let store = SessionStore::new(data_dir).await?;
    sqlx::query(
        "INSERT OR IGNORE INTO project (id, worktree, name, time_created, time_updated, sandboxes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&bundle.session.project_id)
    .bind(&bundle.session.directory)
    .bind(
        Path::new(&bundle.session.directory)
            .file_name()
            .and_then(|n| n.to_str()),
    )
    .bind(bundle.session.time_created)
    .bind(bundle.session.time_updated)
    .bind("[]")
    .execute(store.pool.as_ref())
    .await?;
    sqlx::query(
        "INSERT OR REPLACE INTO session
         (id, project_id, workspace_id, parent_id, slug, directory, path, title, version, share_url,
          summary_additions, summary_deletions, summary_files, summary_diffs, revert, permission,
          agent, model, time_created, time_updated, time_compacting, time_archived)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
    )
    .bind(&bundle.session.id)
    .bind(&bundle.session.project_id)
    .bind(&bundle.session.workspace_id)
    .bind(&bundle.session.parent_id)
    .bind(&bundle.session.slug)
    .bind(&bundle.session.directory)
    .bind(&bundle.session.path)
    .bind(&bundle.session.title)
    .bind(&bundle.session.version)
    .bind(&bundle.session.share_url)
    .bind(bundle.session.summary_additions)
    .bind(bundle.session.summary_deletions)
    .bind(bundle.session.summary_files)
    .bind(&bundle.session.summary_diffs)
    .bind(&bundle.session.revert)
    .bind(&bundle.session.permission)
    .bind(&bundle.session.agent)
    .bind(&bundle.session.model)
    .bind(bundle.session.time_created)
    .bind(bundle.session.time_updated)
    .bind(bundle.session.time_compacting)
    .bind(bundle.session.time_archived)
    .execute(store.pool.as_ref())
    .await?;
    for message in bundle.messages {
        sqlx::query(
            "INSERT OR REPLACE INTO message (id, session_id, time_created, time_updated, data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(message.id)
        .bind(message.session_id)
        .bind(message.time_created)
        .bind(message.time_updated)
        .bind(message.data)
        .execute(store.pool.as_ref())
        .await?;
    }
    for part in bundle.parts {
        sqlx::query(
            "INSERT OR REPLACE INTO part (id, message_id, session_id, time_created, time_updated, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(part.id)
        .bind(part.message_id)
        .bind(part.session_id)
        .bind(part.time_created)
        .bind(part.time_updated)
        .bind(part.data)
        .execute(store.pool.as_ref())
        .await?;
    }
    println!("Imported session {}", bundle.session.id);
    Ok(())
}

pub(crate) fn handle_plugin(args: args::PluginArgs) -> Result<()> {
    let path = if args.global {
        directories::ProjectDirs::from("com", "opencode", "opencode")
            .map(|dirs| dirs.config_dir().join("opencode.json"))
            .unwrap_or_else(|| PathBuf::from("opencode.json"))
    } else {
        PathBuf::from("opencode.json")
    };
    let mut config = if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        serde_json::from_str::<serde_json::Value>(&text)?
    } else {
        serde_json::json!({})
    };
    let plugin = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("config root must be an object"))?
        .entry("plugin")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let plugins = plugin
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("config `plugin` must be an array"))?;
    let value = plugin_config_value(&args.module);
    if plugins.contains(&value) && !args.force {
        anyhow::bail!("plugin already exists: {}", args.module);
    }
    if !plugins.contains(&value) {
        plugins.push(value);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&config)?),
    )?;
    println!("Updated {}", path.display());
    Ok(())
}

fn plugin_config_value(module: &str) -> serde_json::Value {
    if module.starts_with("http://") || module.starts_with("https://") {
        serde_json::json!({ "url": module })
    } else if module.starts_with('.')
        || module.starts_with('/')
        || module.ends_with(".js")
        || module.ends_with(".ts")
    {
        serde_json::json!({ "path": module })
    } else {
        serde_json::json!({ "name": module })
    }
}

pub(crate) async fn handle_db(subcommand: args::DbSubcommand, data_dir: PathBuf) -> Result<()> {
    match subcommand {
        args::DbSubcommand::Path => println!("{}", data_dir.join("opencode.db").display()),
        args::DbSubcommand::Migrate => {
            let _ = SessionStore::new(data_dir).await?;
            println!("Database migrated");
        }
        args::DbSubcommand::Query(args) => {
            let store = SessionStore::new(data_dir).await?;
            let Some(query) = args.query else {
                println!("{}", store_path_hint());
                return Ok(());
            };
            let rows = sqlx::query(&query).fetch_all(store.pool.as_ref()).await?;
            let columns = rows
                .first()
                .map(|row| {
                    row.columns()
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let values = rows
                .iter()
                .map(|row| {
                    (0..columns.len())
                        .map(|idx| sqlite_cell_to_string(row, idx))
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?;
            print!("{}", format_query_rows(&columns, &values, args.format)?);
        }
    }
    Ok(())
}

fn store_path_hint() -> &'static str {
    "Pass a SQL query or use `opencode db path` and open the database with sqlite3."
}

fn sqlite_cell_to_string(row: &sqlx::sqlite::SqliteRow, idx: usize) -> Result<String> {
    if let Ok(value) = row.try_get::<Option<String>, _>(idx) {
        return Ok(value.unwrap_or_default());
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(idx) {
        return Ok(value.map(|v| v.to_string()).unwrap_or_default());
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(idx) {
        return Ok(value.map(|v| v.to_string()).unwrap_or_default());
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(idx) {
        return Ok(value.map(|v| format!("{:?}", v)).unwrap_or_default());
    }
    Ok(String::new())
}

pub(crate) fn handle_upgrade(args: args::UpgradeArgs) -> Result<()> {
    let explicit = args.method.map(upgrade_install_method);
    let detected = detect_upgrade_method();
    let plan = local_process::upgrade_plan(args.target.as_deref(), "latest", explicit, detected);
    println!("Upgrade target: {}", plan.target);
    println!("Install method: {:?}", plan.method);
    match plan.action {
        local_process::UpgradeAction::Command(command) => run_command_spec_status(&command)?,
        local_process::UpgradeAction::SelfUpdate { target } => run_self_update(&target)?,
    }
    Ok(())
}

pub(crate) fn handle_uninstall(args: args::UninstallArgs, data_dir: PathBuf) -> Result<()> {
    let config_dir = directories::ProjectDirs::from("com", "opencode", "opencode")
        .map(|dirs| dirs.config_dir().to_path_buf());
    println!("Data dir: {}", data_dir.display());
    if let Some(config_dir) = config_dir {
        println!("Config dir: {}", config_dir.display());
    }
    if args.dry_run || !args.force {
        println!("No files removed. Re-run with --force to remove files.");
        return Ok(());
    }
    if !args.keep_data && data_dir.exists() {
        std::fs::remove_dir_all(&data_dir)?;
    }
    if !args.keep_config {
        if let Some(config_dir) = directories::ProjectDirs::from("com", "opencode", "opencode")
            .map(|dirs| dirs.config_dir().to_path_buf())
        {
            if config_dir.exists() {
                std::fs::remove_dir_all(config_dir)?;
            }
        }
    }
    println!("Removed selected opencode-rs files.");
    Ok(())
}

pub(crate) fn handle_github(subcommand: args::GithubSubcommand) -> Result<()> {
    match subcommand {
        args::GithubSubcommand::Install => {
            let path = PathBuf::from(local_process::github_workflow_file());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let model = std::env::var("OPENCODE_MODEL")
                .unwrap_or_else(|_| "anthropic/claude-3-5-sonnet-20241022".to_string());
            let workflow = local_process::github_workflow_contents(
                &model,
                &["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GITHUB_TOKEN"],
            );
            std::fs::write(&path, workflow)?;
            println!("Installed GitHub workflow at {}.", path.display());
        }
        args::GithubSubcommand::Run(args) => {
            let mut command = Command::new("opencode");
            command.args(["run", "--agent", "github"]);
            if let Some(token) = args.token.or_else(|| std::env::var("GITHUB_TOKEN").ok()) {
                command.env("GITHUB_TOKEN", token);
            }
            if let Some(event) = args.event {
                let message = std::fs::read_to_string(&event).unwrap_or(event);
                command.arg(message);
            } else {
                command.arg("Run the GitHub agent for the current repository context.");
            }
            let status = command
                .status()
                .context("failed to run opencode github agent")?;
            if !status.success() {
                anyhow::bail!("opencode github agent exited with {}", status);
            }
        }
    }
    Ok(())
}

pub(crate) fn handle_pr(args: args::PrArgs) -> Result<()> {
    let checkout = local_process::pr_checkout_command(args.number);
    run_command_spec_status(&checkout)?;

    let view = local_process::pr_view_command(args.number);
    let metadata = run_command_spec_capture(&view).unwrap_or_default();
    let session_id = if let Some(url) = local_process::parse_opencode_session_url(&metadata) {
        let import = local_process::CommandSpec::new("opencode", ["import", url.as_str()]);
        let output = run_command_spec_capture(&import).unwrap_or_default();
        local_process::parse_imported_session_id(&output)
    } else {
        None
    };

    let start = local_process::opencode_start_command(session_id.as_deref());
    run_command_spec_status(&start)?;
    Ok(())
}

pub(crate) async fn handle_attach(args: args::AttachArgs) -> Result<()> {
    if let Some(dir) = args.dir {
        std::env::set_current_dir(&dir)
            .with_context(|| format!("failed to change directory to {dir}"))?;
    }
    let base_url = args
        .url
        .unwrap_or_else(|| "http://127.0.0.1:4096".to_string());
    let client = reqwest::Client::new();
    let mut health = client.get(format!("{}/health", base_url.trim_end_matches('/')));
    let username = args
        .username
        .or_else(|| std::env::var("OPENCODE_SERVER_USERNAME").ok())
        .unwrap_or_else(|| "opencode".to_string());
    let password = args
        .password
        .or_else(|| std::env::var("OPENCODE_SERVER_PASSWORD").ok());
    if let Some(password) = password.as_ref() {
        health = health.basic_auth(&username, Some(password));
    }
    health.send().await?.error_for_status()?;

    if let Some(session) = args.session {
        let endpoint = local_process::attach_select_session_endpoint(&base_url, &session);
        let mut request = client
            .post(endpoint)
            .json(&serde_json::json!({ "session_id": session }));
        if let Some(password) = password.as_ref() {
            request = request.basic_auth(&username, Some(password));
        }
        request.send().await?.error_for_status()?;
        println!("Attached server {} to session.", base_url);
    } else if args.r#continue {
        println!(
            "Connected to {}. Continue mode requires the running TUI to select its latest session.",
            base_url
        );
    } else {
        println!("Connected to {}.", base_url);
    }
    Ok(())
}

fn upgrade_install_method(method: args::InstallMethod) -> local_process::InstallMethod {
    match method {
        args::InstallMethod::Curl => local_process::InstallMethod::Curl,
        args::InstallMethod::Npm => local_process::InstallMethod::Npm,
        args::InstallMethod::Pnpm => local_process::InstallMethod::Pnpm,
        args::InstallMethod::Bun => local_process::InstallMethod::Bun,
        args::InstallMethod::Brew => local_process::InstallMethod::Brew,
        args::InstallMethod::Choco => local_process::InstallMethod::Choco,
        args::InstallMethod::Scoop => local_process::InstallMethod::Scoop,
    }
}

fn detect_upgrade_method() -> local_process::InstallMethod {
    if command_exists("brew") {
        local_process::InstallMethod::Brew
    } else if command_exists("npm") {
        local_process::InstallMethod::Npm
    } else if command_exists("pnpm") {
        local_process::InstallMethod::Pnpm
    } else if command_exists("bun") {
        local_process::InstallMethod::Bun
    } else {
        local_process::InstallMethod::Unknown
    }
}

fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {program} >/dev/null 2>&1")])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_command_spec_status(spec: &local_process::CommandSpec) -> Result<()> {
    let status = Command::new(&spec.program)
        .args(&spec.args)
        .status()
        .with_context(|| format!("failed to run {}", format_command_spec(spec)))?;
    if !status.success() {
        anyhow::bail!("{} exited with {}", format_command_spec(spec), status);
    }
    Ok(())
}

fn run_command_spec_capture(spec: &local_process::CommandSpec) -> Result<String> {
    run_command_capture(
        &spec.program,
        &spec.args,
        &[0],
        &format!("failed to run {}", format_command_spec(spec)),
    )
}

fn format_command_spec(spec: &local_process::CommandSpec) -> String {
    std::iter::once(spec.program.as_str())
        .chain(spec.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_self_update(target: &str) -> Result<()> {
    #[cfg(feature = "self-update")]
    {
        let mut update = self_update::backends::github::Update::configure();
        update
            .repo_owner("sst")
            .repo_name("opencode")
            .bin_name("opencode")
            .show_download_progress(true)
            .current_version(env!("CARGO_PKG_VERSION"));
        if target != "latest" {
            update.target_version_tag(&format!("v{target}"));
        }
        let status = update.build()?.update()?;
        println!("Updated opencode to {}.", status.version());
        Ok(())
    }

    #[cfg(not(feature = "self-update"))]
    {
        anyhow::bail!("self-update support is not enabled in this build")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_rows_keeps_order_and_truncates() {
        let rows = vec!["a", "b", "c", "d"];
        assert_eq!(limit_rows(rows.clone(), Some(2)), vec!["a", "b"]);
        assert_eq!(limit_rows(rows.clone(), Some(0)), Vec::<&str>::new());
        assert_eq!(limit_rows(rows.clone(), None), rows);
    }

    #[test]
    fn format_query_rows_supports_tsv_and_json() {
        let columns = vec!["id".to_string(), "title".to_string()];
        let rows = vec![
            vec!["one".to_string(), "hello\tworld".to_string()],
            vec!["two".to_string(), "line\nbreak".to_string()],
        ];

        assert_eq!(
            format_query_rows(&columns, &rows, crate::cli::args::DbFormat::Tsv).unwrap(),
            "id\ttitle\none\thello world\ntwo\tline break\n"
        );

        let json = format_query_rows(&columns, &rows, crate::cli::args::DbFormat::Json).unwrap();
        assert!(json.contains("\"id\": \"one\""));
        assert!(json.contains("\"title\": \"line\\nbreak\""));
    }

    #[test]
    fn agent_markdown_includes_frontmatter_and_prompt() {
        let content = agent_markdown(
            "Review pull requests",
            Some(crate::cli::args::AgentMode::Subagent),
            Some("openai/gpt-4o"),
            Some("read,grep"),
        );

        assert!(content.contains("description: Review pull requests"));
        assert!(content.contains("mode: subagent"));
        assert!(content.contains("model: openai/gpt-4o"));
        assert!(content.contains("read: allow"));
        assert!(content.contains("grep: allow"));
        assert!(content.contains("Review pull requests"));
    }

    #[test]
    fn plugin_config_value_matches_config_schema() {
        assert_eq!(
            plugin_config_value("opencode-plugin-example"),
            serde_json::json!({ "name": "opencode-plugin-example" })
        );
        assert_eq!(
            plugin_config_value("./plugin.ts"),
            serde_json::json!({ "path": "./plugin.ts" })
        );
        assert_eq!(
            plugin_config_value("https://example.com/plugin.js"),
            serde_json::json!({ "url": "https://example.com/plugin.js" })
        );
    }

    #[test]
    fn format_debug_paths_aligns_names() {
        let rows = vec![
            ("data", PathBuf::from("/tmp/opencode/data")),
            ("config", PathBuf::from("/tmp/opencode/config")),
        ];

        assert_eq!(
            format_debug_paths(&rows),
            "data       /tmp/opencode/data\nconfig     /tmp/opencode/config\n"
        );
    }

    #[test]
    fn build_rg_search_args_include_globs_and_limit() {
        let args = build_rg_search_args(
            "needle",
            &["*.rs".to_string(), "!target/**".to_string()],
            Some(3),
        );

        assert_eq!(
            args,
            vec![
                "--no-config",
                "--json",
                "--hidden",
                "--glob",
                "!.git/*",
                "--no-messages",
                "--glob",
                "*.rs",
                "--glob",
                "!target/**",
                "--max-count",
                "3",
                "--",
                "needle",
                ".",
            ]
        );
    }

    #[test]
    fn format_file_tree_uses_json_shape() {
        let tree = vec![
            FileTreeEntry {
                path: PathBuf::from("src/main.rs"),
                kind: FileTreeKind::File,
            },
            FileTreeEntry {
                path: PathBuf::from("src/bin"),
                kind: FileTreeKind::Directory,
            },
        ];

        let formatted = format_file_tree(&tree).unwrap();
        assert!(formatted.contains("\"path\": \"src/main.rs\""));
        assert!(formatted.contains("\"type\": \"file\""));
        assert!(formatted.contains("\"path\": \"src/bin\""));
        assert!(formatted.contains("\"type\": \"directory\""));
    }
}
