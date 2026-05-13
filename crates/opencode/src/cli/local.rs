use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::{Column, Row};

use crate::cli::args;
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
            for name in [
                "build",
                "plan",
                "general",
                "explore",
                "scout",
                "compaction",
                "title",
                "summary",
            ] {
                if let Some(agent) = crate::agent::get_agent(name) {
                    let description = agent.description.unwrap_or_default();
                    println!("{} - {}", agent.name, description);
                }
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
        args::DebugSubcommand::Rg(args) => run_rg(args)?,
        args::DebugSubcommand::File(args) => print_file(&PathBuf::from(args.path))?,
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

fn run_rg(args: args::RgDebugArgs) -> Result<()> {
    let mut command = std::process::Command::new("rg");
    command.arg(args.pattern);
    if args.paths.is_empty() {
        command.arg(".");
    } else {
        command.args(args.paths);
    }
    let status = command.status().context("failed to run rg")?;
    if !status.success() {
        anyhow::bail!("rg exited with {}", status);
    }
    Ok(())
}

fn print_file(path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    for (idx, line) in content.lines().enumerate() {
        println!("{:>6}  {}", idx + 1, line);
    }
    Ok(())
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
        args::DebugSnapshotSubcommand::List => {
            println!("Local snapshot storage is not enabled in opencode-rs.");
        }
        args::DebugSnapshotSubcommand::Show(args) => {
            println!(
                "Local snapshot '{}' is not available in opencode-rs.",
                args.id
            );
        }
    }
    Ok(())
}

pub(crate) async fn handle_mcp(subcommand: args::McpSubcommand, data_dir: PathBuf) -> Result<()> {
    match subcommand {
        args::McpSubcommand::List => {
            let (manager, config) = start_mcp_from_project().await?;
            print_mcp_status(&manager);
            if config.mcp.is_none() {
                println!("No MCP servers configured.");
            }
        }
        args::McpSubcommand::Debug(args) => {
            let (manager, _) = start_mcp_from_project().await?;
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
                println!("Open the configured MCP server OAuth URL for '{}'.", name);
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
            println!("Add MCP servers in opencode.json under the `mcp` key.");
        }
    }
    Ok(())
}

async fn start_mcp_from_project() -> Result<(McpManager, crate::config::Config)> {
    let cwd = std::env::current_dir()?;
    let config = crate::config::load_project_config(&cwd)?.unwrap_or_default();
    let mut manager = McpManager::new();
    manager.start_configured(&config).await;
    Ok((manager, config))
}

fn print_mcp_status(manager: &McpManager) {
    for (name, status) in manager.status() {
        match status {
            McpServerStatus::Connected => println!("{} - connected", name),
            McpServerStatus::Disabled => println!("{} - disabled", name),
            McpServerStatus::Failed { error } => println!("{} - failed: {}", name, error),
        }
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
    let target = args.target.unwrap_or_else(|| "latest".to_string());
    let method = args
        .method
        .map(|method| format!("{:?}", method).to_ascii_lowercase())
        .unwrap_or_else(|| "auto".to_string());
    println!("Upgrade target: {}", target);
    println!("Requested method: {}", method);
    println!("Use your package manager to upgrade opencode-rs; automatic upgrade is disabled.");
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
            println!("GitHub agent files are bundled; configure GitHub tokens via environment variables.");
        }
        args::GithubSubcommand::Run(args) => {
            println!(
                "GitHub agent run requested (event={:?}, token={}).",
                args.event,
                if args.token.is_some() {
                    "provided"
                } else {
                    "env"
                }
            );
        }
    }
    Ok(())
}

pub(crate) fn handle_pr(args: args::PrArgs) -> Result<()> {
    println!(
        "Use `gh pr checkout {}` then run opencode in the checked-out branch.",
        args.number
    );
    Ok(())
}

pub(crate) fn handle_attach(args: args::AttachArgs) -> Result<()> {
    println!(
        "Attach target: {}",
        args.url.as_deref().unwrap_or("http://127.0.0.1:4096")
    );
    if let Some(session) = args.session {
        println!("Session: {}", session);
    } else if args.r#continue {
        println!("Continue: latest session");
    }
    println!("Start `opencode tui` for the local terminal UI.");
    Ok(())
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
}
