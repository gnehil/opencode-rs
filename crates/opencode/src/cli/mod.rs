pub mod args;
mod local;
pub(crate) mod local_process;
pub(crate) mod mcp_cli;
pub(crate) mod provider_auth;

use clap::{CommandFactory, Parser};
use std::io::{IsTerminal, Read};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use crate::acp::ACPServer;
use crate::config::{Config, ProviderConfigEntry};
use crate::provider::{
    AlibabaProvider, AnthropicProvider, AzureProvider, BedrockProvider, CerebrasProvider,
    CohereProvider, DeepInfraProvider, DeepSeekProvider, FireworksProvider, GitHubCopilotProvider,
    GitLabProvider, GoogleProvider, GroqProvider, LMStudioProvider, MistralProvider,
    OllamaProvider, OpenAIProvider, OpenRouterProvider, PerplexityProvider, Provider,
    TogetherAIProvider, VeniceProvider, VercelProvider, VertexProvider, XAIProvider,
};
use crate::session::PromptProcessor;
use crate::session::SessionStore;

pub async fn run_async() {
    let cli = args::Cli::parse();

    if cli.version {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if let Some(shell) = cli.completion {
        let mut command = args::Cli::command();
        clap_complete::generate(shell, &mut command, "opencode", &mut std::io::stdout());
        return;
    }

    let data_dir = directories::ProjectDirs::from("com", "opencode", "opencode")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".opencode"));

    match cli.command {
        Some(args::Commands::Tui(tui_args)) => {
            handle_tui(tui_args, data_dir).await;
        }
        Some(args::Commands::Run(run_args)) => {
            handle_run(run_args, data_dir).await;
        }
        Some(args::Commands::Generate) => {
            exit_on_error(local::handle_generate());
        }
        Some(args::Commands::Console { subcommand }) => {
            exit_on_error(local::handle_console(subcommand));
        }
        Some(args::Commands::Session { subcommand }) => {
            handle_session(subcommand, data_dir).await;
        }
        Some(args::Commands::Agent { subcommand }) => {
            exit_on_error(local::handle_agent(subcommand).await);
        }
        Some(args::Commands::Upgrade(upgrade_args)) => {
            exit_on_error(local::handle_upgrade(upgrade_args));
        }
        Some(args::Commands::Uninstall(uninstall_args)) => {
            exit_on_error(local::handle_uninstall(uninstall_args, data_dir));
        }
        Some(args::Commands::Models(models_args)) => {
            handle_models(models_args, data_dir).await;
        }
        Some(args::Commands::Serve(network_args)) => {
            handle_serve(network_args, data_dir, false).await;
        }
        Some(args::Commands::Web(network_args)) => {
            handle_serve(network_args, data_dir, true).await;
        }
        Some(args::Commands::Stats(stats_args)) => {
            exit_on_error(local::handle_stats(stats_args, data_dir).await);
        }
        Some(args::Commands::Debug { subcommand }) => {
            exit_on_error(local::handle_debug(subcommand).await);
        }
        Some(args::Commands::Mcp { subcommand }) => {
            exit_on_error(local::handle_mcp(subcommand, data_dir).await);
        }
        Some(args::Commands::Github { subcommand }) => {
            exit_on_error(local::handle_github(subcommand));
        }
        Some(args::Commands::Export(export_args)) => {
            exit_on_error(local::handle_export(export_args, data_dir).await);
        }
        Some(args::Commands::Import(import_args)) => {
            exit_on_error(local::handle_import(import_args, data_dir).await);
        }
        Some(args::Commands::Pr(pr_args)) => {
            exit_on_error(local::handle_pr(pr_args));
        }
        Some(args::Commands::Providers { subcommand }) => {
            handle_providers(subcommand, data_dir).await;
        }
        Some(args::Commands::Plugin(plugin_args)) => {
            exit_on_error(local::handle_plugin(plugin_args));
        }
        Some(args::Commands::Db { subcommand }) => {
            exit_on_error(local::handle_db(subcommand, data_dir).await);
        }
        Some(args::Commands::Acp(acp_args)) => {
            handle_acp(acp_args, data_dir).await;
        }
        Some(args::Commands::Attach(attach_args)) => {
            exit_on_error(local::handle_attach(attach_args).await);
        }
        None => {
            handle_tui(
                args::TuiArgs {
                    project: None,
                    r#continue: false,
                    session: None,
                    fork: false,
                    prompt: None,
                    model: None,
                    agent: None,
                    network: args::NetworkArgsMinimal {
                        port: None,
                        hostname: None,
                        mdns: false,
                        mdns_domain: None,
                        cors: Vec::new(),
                    },
                },
                data_dir,
            )
            .await;
        }
    }
}

fn exit_on_error(result: anyhow::Result<()>) {
    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn handle_tui(args: args::TuiArgs, data_dir: PathBuf) {
    use crate::tui::App;

    let mut app = App::new();

    let store = SessionStore::new(data_dir).await.ok();
    if let Some(store) = store {
        let sessions = store.list(None).await.ok();
        if let Some(sessions) = sessions {
            app.load_sessions(sessions);
        }
    }

    if let Some(session_id_str) = &args.session {
        let session_id = crate::id::SessionID::parse(session_id_str).ok();
        if let Some(session_id) = session_id {
            app.select_session(session_id);
        }
    }

    if let Err(e) = app.run() {
        eprintln!("TUI error: {}", e);
    }
}

async fn handle_run(args: Box<args::RunArgs>, data_dir: PathBuf) {
    let project_path = if let Some(dir) = &args.dir {
        let path = PathBuf::from(dir);
        std::env::set_current_dir(&path).unwrap_or_else(|e| {
            eprintln!("Failed to change directory to {}: {}", path.display(), e);
            std::process::exit(1);
        });
        std::env::current_dir().unwrap_or(path)
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    if args.fork && !args.r#continue && args.session.is_none() {
        eprintln!("--fork requires --continue or --session");
        std::process::exit(1);
    }
    if args.interactive && args.command.is_some() {
        eprintln!("--interactive cannot be used with --command");
        std::process::exit(1);
    }
    if args.interactive && matches!(args.format, args::RunFormat::Json) {
        eprintln!("--interactive cannot be used with --format json");
        std::process::exit(1);
    }

    let piped = read_piped_stdin().unwrap_or_else(|e| {
        eprintln!("Failed to read stdin: {}", e);
        std::process::exit(1);
    });
    let mut message = local_process::join_run_message(&args.message);
    message = local_process::resolve_run_input(&message, piped.as_deref()).unwrap_or_default();
    if message.trim().is_empty() && args.command.is_none() && !args.interactive {
        eprintln!("You must provide a message or a command");
        std::process::exit(1);
    }

    let store = SessionStore::new(data_dir.clone())
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to initialize database: {}", e);
            std::process::exit(1);
        });

    let project_config = match crate::config::load_project_config(&project_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Warning: failed to load opencode config: {}", e);
            None
        }
    };
    let run_command = match args.command.as_deref() {
        Some(command) => Some(
            resolve_run_command(command, &message, &project_path, project_config.as_ref())
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Failed to resolve command '{}': {}", command, e);
                    std::process::exit(1);
                }),
        ),
        None => None,
    };
    let prompt_text = run_command
        .as_ref()
        .map(|command| command.prompt.as_str())
        .unwrap_or(message.as_str());
    let session = if let Some(existing) = if let Some(session_id_str) = &args.session {
        let session_id = crate::id::SessionID::parse(session_id_str).unwrap_or_else(|_| {
            eprintln!("Invalid session ID: {}", session_id_str);
            std::process::exit(1);
        });
        Some(
            store
                .get(&session_id)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Failed to get session: {}", e);
                    std::process::exit(1);
                })
                .unwrap_or_else(|| {
                    eprintln!("Session not found: {}", session_id_str);
                    std::process::exit(1);
                }),
        )
    } else if args.r#continue {
        store
            .list(None)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Failed to list sessions: {}", e);
                std::process::exit(1);
            })
            .into_iter()
            .next()
    } else {
        None
    } {
        if args.fork {
            store
                .create(
                    &format!("{} (fork)", existing.title),
                    &existing.project_id,
                    &PathBuf::from(&existing.directory),
                )
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Failed to fork session: {}", e);
                    std::process::exit(1);
                })
        } else {
            existing
        }
    } else {
        let title = run_session_title(args.title.as_deref(), prompt_text);
        store
            .create(&title, "default", &project_path)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Failed to create session: {}", e);
                std::process::exit(1);
            })
    };

    let json_output = matches!(args.format, args::RunFormat::Json);
    if !json_output {
        println!("Session ID: {}", session.id);
        println!("Title: {}", session.title);
    }

    let session_id = crate::id::SessionID::parse(&session.id).unwrap_or_else(|_| {
        eprintln!("Invalid session ID in database: {}", session.id);
        std::process::exit(1);
    });

    let config_default_agent = project_config
        .as_ref()
        .and_then(|config| config.default_agent.as_deref());
    let agent_name = args
        .agent
        .as_deref()
        .or_else(|| {
            run_command
                .as_ref()
                .and_then(|command| command.agent.as_deref())
        })
        .or_else(|| session.agent.as_deref())
        .or(config_default_agent)
        .unwrap_or(crate::agent::DEFAULT_AGENT_NAME);
    let agent_config_model = project_config
        .as_ref()
        .and_then(|config| config.agent.as_ref())
        .and_then(|agents| agents.get(agent_name))
        .and_then(|agent| agent.model.as_deref());
    let env_model = std::env::var("OPENCODE_MODEL").ok();
    let selected_model = args
        .model
        .as_deref()
        .or_else(|| {
            run_command
                .as_ref()
                .and_then(|command| command.model.as_deref())
        })
        .or_else(|| session.model.as_deref())
        .or(agent_config_model)
        .or_else(|| {
            project_config
                .as_ref()
                .and_then(|config| config.model.as_deref())
        })
        .or(env_model.as_deref());

    let credentials = load_provider_credentials(&data_dir);
    let provider = build_provider_from_model_config_auth_or_env(
        selected_model,
        project_config.as_ref(),
        Some(&credentials),
    )
    .unwrap_or_else(|e| {
        eprintln!("Failed to initialize provider: {}", e);
        std::process::exit(1);
    });

    if !json_output {
        println!("Using provider: {}", provider.name());
        println!(
            "Default model: {}",
            provider
                .default_model()
                .map(|m| m.id.as_ref().map(|i| i.to_string()).unwrap_or_default())
                .unwrap_or_default()
        );
    }

    let store = Arc::new(store);
    let mcp_tools = load_mcp_tools_from_project(&project_path, &data_dir).await;
    let mut processor = PromptProcessor::new(store.clone(), provider)
        .with_tools(crate::tool::registry_with(mcp_tools))
        .with_agent(agent_name);
    if let Some(config) = project_config.clone() {
        processor = processor.with_config(config);
    }
    if let Some(model) = selected_model {
        processor = processor.with_model_selection(model);
    }

    if args.agent.is_some() || (session.agent.is_none() && config_default_agent.is_some()) {
        store
            .set_agent(&session_id, agent_name)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Failed to set session agent: {}", e);
                std::process::exit(1);
            });
    }
    if let Some(model) = selected_model.filter(|_| args.model.is_some() || session.model.is_none())
    {
        store
            .set_model(&session_id, model)
            .await
            .unwrap_or_else(|e| {
                eprintln!("Failed to set session model: {}", e);
                std::process::exit(1);
            });
    }

    let user_message_id = crate::id::MessageID::new();
    let user_parts = build_run_user_parts(
        &session_id,
        &user_message_id,
        prompt_text,
        args.command
            .is_none()
            .then_some(args.file.as_deref())
            .flatten(),
        &project_path,
    )
    .unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });

    if !json_output {
        println!("Processing: {}", prompt_text);
    }
    let result = processor
        .process_stream_with_parts(&session_id, prompt_text, user_message_id, user_parts)
        .await;

    match result {
        Ok(events) => {
            if let Err(e) = emit_run_events(&args.format, &session_id, &events) {
                eprintln!("Error processing prompt: {}", e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error processing prompt: {}", e);
            std::process::exit(1);
        }
    }
}

struct RunCommandInput {
    prompt: String,
    agent: Option<String>,
    model: Option<String>,
}

async fn resolve_run_command(
    command_name: &str,
    arguments: &str,
    project_path: &std::path::Path,
    config: Option<&Config>,
) -> anyhow::Result<RunCommandInput> {
    let name = command_name.trim().trim_start_matches('/');
    if name.is_empty() {
        anyhow::bail!("command name cannot be empty");
    }
    let commands = crate::command::load_commands(project_path, config)?;
    let command = commands
        .into_iter()
        .find(|command| command.name == name)
        .ok_or_else(|| anyhow::anyhow!("unknown command"))?;
    let prompt = crate::command::render_template_with_shell(
        &command.template,
        arguments,
        project_path,
        config.and_then(|config| config.shell.as_deref()),
    )
    .await?;
    Ok(RunCommandInput {
        prompt,
        agent: command.agent,
        model: command.model,
    })
}

fn read_piped_stdin() -> anyhow::Result<Option<String>> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    let mut input = String::new();
    stdin.read_to_string(&mut input)?;
    Ok(Some(input))
}

fn run_session_title(title: Option<&str>, prompt: &str) -> String {
    match title {
        Some(title) if !title.is_empty() => title.to_string(),
        Some(_) => truncate_title(prompt),
        None => format!("Session: {}", truncate_title(prompt)),
    }
}

fn truncate_title(prompt: &str) -> String {
    let mut title = prompt.chars().take(50).collect::<String>();
    if prompt.chars().count() > 50 {
        title.push_str("...");
    }
    title
}

fn build_run_user_parts(
    session_id: &crate::id::SessionID,
    message_id: &crate::id::MessageID,
    text: &str,
    files: Option<&[String]>,
    base_dir: &std::path::Path,
) -> anyhow::Result<Vec<crate::message::Part>> {
    let mut parts = Vec::new();
    for file in files.unwrap_or_default() {
        parts.push(crate::message::Part::File(run_file_part(
            session_id, message_id, file, base_dir,
        )?));
    }
    parts.push(crate::message::Part::Text(crate::message::TextPart {
        id: crate::id::PartID::new(),
        session_id: session_id.clone(),
        message_id: message_id.clone(),
        text: text.to_string(),
        synthetic: None,
        ignored: None,
        time: None,
        metadata: None,
    }));
    Ok(parts)
}

fn run_file_part(
    session_id: &crate::id::SessionID,
    message_id: &crate::id::MessageID,
    file: &str,
    base_dir: &std::path::Path,
) -> anyhow::Result<crate::message::FilePart> {
    let input = std::path::PathBuf::from(file);
    let resolved = if input.is_absolute() {
        input
    } else {
        base_dir.join(input)
    };
    if !resolved.exists() {
        anyhow::bail!("File not found: {}", file);
    }
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    let mime = if resolved.is_dir() {
        "application/x-directory"
    } else {
        "text/plain"
    };
    let source = if mime == "text/plain" {
        std::fs::read_to_string(&resolved)
            .ok()
            .map(|text| crate::message::FilePartSource::File {
                path: resolved.to_string_lossy().to_string(),
                text: crate::message::SourceText {
                    start: 0.0,
                    end: text.len() as f64,
                    value: text,
                },
            })
    } else {
        None
    };
    Ok(crate::message::FilePart {
        id: crate::id::PartID::new(),
        session_id: session_id.clone(),
        message_id: message_id.clone(),
        mime: mime.to_string(),
        filename: resolved
            .file_name()
            .and_then(|name| name.to_str())
            .map(ToString::to_string),
        url: crate::lsp::diagnostics::path_to_uri(&resolved)?,
        source,
    })
}

fn emit_run_events(
    format: &args::RunFormat,
    session_id: &crate::id::SessionID,
    events: &[crate::session::processor::ProcessEvent],
) -> anyhow::Result<()> {
    if matches!(format, args::RunFormat::Json) {
        for event in events {
            println!("{}", run_event_json(session_id, event));
        }
    } else {
        let mut response = None;
        for event in events {
            match event {
                crate::session::processor::ProcessEvent::Done(text) => response = Some(text),
                crate::session::processor::ProcessEvent::Error(error) => {
                    anyhow::bail!(error.clone());
                }
                _ => {}
            }
        }
        println!("\nResponse:\n{}", response.cloned().unwrap_or_default());
    }
    if let Some(error) = events.iter().find_map(|event| match event {
        crate::session::processor::ProcessEvent::Error(error) => Some(error),
        _ => None,
    }) {
        anyhow::bail!(error.clone());
    }
    Ok(())
}

fn run_event_json(
    session_id: &crate::id::SessionID,
    event: &crate::session::processor::ProcessEvent,
) -> serde_json::Value {
    let timestamp = chrono::Utc::now().timestamp_millis();
    match event {
        crate::session::processor::ProcessEvent::TextDelta(delta) => serde_json::json!({
            "type": "text_delta",
            "timestamp": timestamp,
            "sessionID": session_id.to_string(),
            "delta": delta,
        }),
        crate::session::processor::ProcessEvent::ToolStart(tool, input) => serde_json::json!({
            "type": "tool_start",
            "timestamp": timestamp,
            "sessionID": session_id.to_string(),
            "tool": tool,
            "input": input,
        }),
        crate::session::processor::ProcessEvent::ToolComplete(tool, output) => serde_json::json!({
            "type": "tool_complete",
            "timestamp": timestamp,
            "sessionID": session_id.to_string(),
            "tool": tool,
            "output": output,
        }),
        crate::session::processor::ProcessEvent::Done(text) => serde_json::json!({
            "type": "done",
            "timestamp": timestamp,
            "sessionID": session_id.to_string(),
            "text": text,
        }),
        crate::session::processor::ProcessEvent::Error(error) => serde_json::json!({
            "type": "error",
            "timestamp": timestamp,
            "sessionID": session_id.to_string(),
            "error": error,
        }),
    }
}

async fn handle_session(subcommand: args::SessionSubcommand, data_dir: PathBuf) {
    let store = SessionStore::new(data_dir).await.unwrap_or_else(|e| {
        eprintln!("Failed to initialize database: {}", e);
        std::process::exit(1);
    });

    match subcommand {
        args::SessionSubcommand::List(list_args) => {
            let sessions = store.list(None).await.unwrap_or_else(|e| {
                eprintln!("Failed to list sessions: {}", e);
                std::process::exit(1);
            });
            let sessions = local::limit_rows(sessions, list_args.max_count);

            if sessions.is_empty() {
                println!("No sessions found.");
            } else if matches!(list_args.format, args::OutputFormat::Json) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&sessions).unwrap_or_else(|_| "[]".to_string())
                );
            } else {
                println!("Sessions:");
                for session in sessions {
                    let archived = if session.time_archived.is_some() {
                        " (archived)"
                    } else {
                        ""
                    };
                    println!("  {} - {}{}", session.id, session.title, archived);
                }
            }
        }
        args::SessionSubcommand::Delete(delete_args) => {
            let session_id =
                crate::id::SessionID::parse(&delete_args.session_id).unwrap_or_else(|_| {
                    eprintln!("Invalid session ID: {}", delete_args.session_id);
                    std::process::exit(1);
                });
            store.delete(&session_id).await.unwrap_or_else(|e| {
                eprintln!("Failed to delete session: {}", e);
                std::process::exit(1);
            });
            println!("Session {} deleted.", delete_args.session_id);
        }
    }
}

async fn handle_models(args: args::ModelsArgs, data_dir: PathBuf) {
    let credentials = load_provider_credentials(&data_dir);
    let provider =
        build_provider_from_model_auth_or_env(args.provider.as_deref(), Some(&credentials))
            .unwrap_or_else(|e| {
                eprintln!("Failed to initialize provider: {}", e);
                std::process::exit(1);
            });

    println!("Provider: {}", provider.name());
    println!("Models:");
    for model in provider.models() {
        let model_id = model.id.as_ref().map(|i| i.to_string()).unwrap_or_default();
        let name = model.name.clone().unwrap_or_default();
        println!("  {} - {}", model_id, name);
    }
}

async fn handle_providers(subcommand: args::ProvidersSubcommand, data_dir: PathBuf) {
    let store = provider_auth::ProviderAuthStore::new(data_dir);
    match subcommand {
        args::ProvidersSubcommand::List => {
            let credentials = store.load().unwrap_or_default();
            println!("Available providers:");
            println!("  anthropic - Anthropic (Claude)");
            println!("  openai - OpenAI (GPT)");
            println!("  azure - Azure OpenAI");
            println!("  google - Google Gemini");
            println!("  bedrock - AWS Bedrock");
            println!("  groq - Groq");
            println!("  mistral - Mistral");
            println!("  xai - xAI");
            println!("  ollama - Ollama (local)");
            println!();
            println!("Credential store: {}", store.auth_path().display());
            if credentials.is_empty() {
                println!("Stored credentials: none");
            } else {
                println!("Stored credentials:");
                for (provider, credential) in credentials {
                    let kind = match credential {
                        provider_auth::ProviderCredential::Api { .. } => "api-key",
                        provider_auth::ProviderCredential::Wellknown { .. } => "well-known",
                        provider_auth::ProviderCredential::Oauth { .. } => "oauth",
                    };
                    println!("  {} ({})", provider, kind);
                }
            }
        }
        args::ProvidersSubcommand::Login {
            url,
            provider,
            method,
        } => {
            if let Some(method) = method.as_deref() {
                eprintln!("Login method: {}", method);
            }
            let mode =
                provider_auth::choose_provider_login_mode(url.as_deref(), provider.as_deref())
                    .unwrap_or_else(|e| {
                        eprintln!("Invalid provider login target: {}", e);
                        std::process::exit(1);
                    });
            let mut credentials = store.load().unwrap_or_default();
            match mode {
                provider_auth::ProviderLoginMode::ApiKey { provider } => {
                    let provider_id = normalize_provider_id(&provider);
                    let key = provider_login_api_key(&provider_id).unwrap_or_else(|| {
                        eprintln!(
                            "No API key found for {}. Set OPENCODE_PROVIDER_API_KEY or one of: {}",
                            provider_id,
                            provider_api_key_env_names(&provider_id).join(", ")
                        );
                        std::process::exit(1);
                    });
                    let credential = provider_auth::api_key_credential(&key).unwrap_or_else(|e| {
                        eprintln!("Invalid provider API key: {}", e);
                        std::process::exit(1);
                    });
                    credentials.insert(provider_id.clone(), credential);
                    store.save(&credentials).unwrap_or_else(|e| {
                        eprintln!("Failed to save provider credentials: {}", e);
                        std::process::exit(1);
                    });
                    println!("Saved credentials for provider '{}'.", provider_id);
                }
                provider_auth::ProviderLoginMode::WellKnownUrl { base_url } => {
                    let client = reqwest::Client::new();
                    let metadata = provider_auth::fetch_well_known_metadata(&client, &base_url)
                        .await
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to fetch provider metadata from {}: {}", base_url, e);
                            std::process::exit(1);
                        });
                    let token = provider_auth::run_well_known_auth_command(&metadata.auth.command)
                        .await
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to run provider auth command: {}", e);
                            std::process::exit(1);
                        });
                    let credential =
                        provider_auth::well_known_credential(&metadata.auth.env, &token)
                            .unwrap_or_else(|e| {
                                eprintln!("Invalid provider auth response: {}", e);
                                std::process::exit(1);
                            });
                    credentials.insert(base_url.clone(), credential);
                    store.save(&credentials).unwrap_or_else(|e| {
                        eprintln!("Failed to save provider credentials: {}", e);
                        std::process::exit(1);
                    });
                    println!("Saved well-known provider credentials for {}.", base_url);
                }
                provider_auth::ProviderLoginMode::SelectProvider => {
                    eprintln!("Specify a provider with --provider, or pass a provider login URL.");
                    std::process::exit(1);
                }
            }
        }
        args::ProvidersSubcommand::Logout { provider } => {
            let mut credentials = store.load().unwrap_or_default();
            if let Some(provider) = provider {
                let provider_id = normalize_provider_id(&provider);
                credentials.remove(&provider_id);
                credentials.remove(provider.trim());
                store.save(&credentials).unwrap_or_else(|e| {
                    eprintln!("Failed to save provider credentials: {}", e);
                    std::process::exit(1);
                });
                println!("Removed stored credentials for provider '{}'.", provider_id);
            } else {
                credentials.clear();
                store.save(&credentials).unwrap_or_else(|e| {
                    eprintln!("Failed to save provider credentials: {}", e);
                    std::process::exit(1);
                });
                println!("Removed all stored provider credentials.");
            }
        }
    }
}

async fn handle_serve(args: args::NetworkArgs, data_dir: PathBuf, open_web: bool) {
    if std::env::var_os("OPENCODE_SERVER_PASSWORD").is_none() {
        eprintln!("Warning: OPENCODE_SERVER_PASSWORD is not set; server is unsecured.");
    }

    let hostname = args.hostname.unwrap_or_else(|| {
        if args.mdns {
            "0.0.0.0".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    });
    let port = args.port.unwrap_or(0);
    let addr: SocketAddr = format!("{}:{}", hostname, port)
        .parse()
        .unwrap_or_else(|e| {
            eprintln!("Invalid listen address {}:{}: {}", hostname, port, e);
            std::process::exit(1);
        });

    let credential_dir = data_dir.clone();
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut plugin_manager = crate::plugin::PluginManager::new();
    for error in plugin_manager.register_internal_plugins().await {
        eprintln!("Warning: failed to load internal plugin: {}", error);
    }
    let plugin_manager = Arc::new(plugin_manager);
    let mut state = crate::server::AppState::new(data_dir)
        .with_workspace_root(workspace_root)
        .with_plugin_manager(plugin_manager.clone());

    let project_config = match crate::config::load_project_config(&state.workspace_root) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Warning: failed to load opencode config: {}", e);
            None
        }
    };

    // Bind the listener up front so the real port is known before external
    // plugins are loaded — their SDK client needs a reachable server URL.
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to bind server on {}: {}", addr, e);
            std::process::exit(1);
        });
    let local_addr = listener.local_addr().unwrap_or(addr);
    // Plugins connect back over the loopback interface regardless of the
    // bind host (a `0.0.0.0` bind is not itself a connectable address).
    let plugin_server_url = format!("http://127.0.0.1:{}", local_addr.port());

    if let Some(config) = &project_config {
        state = state.with_config_defaults(config);
        if let Err(error) = plugin_manager
            .trigger_config_change(crate::plugin::ConfigChangeInput {
                config_type: "project".to_string(),
                old_value: serde_json::Value::Null,
                new_value: serde_json::to_value(config).unwrap_or(serde_json::Value::Null),
            })
            .await
        {
            eprintln!("Warning: plugin config hook failed: {}", error);
        }

        // Load external JS/TS plugins declared in config through the
        // subprocess bridge, then hand them their initial `config` payload.
        if let Some(specs) = &config.plugin {
            let input = crate::plugin::bridge::PluginInputData {
                directory: state.workspace_root.to_string_lossy().to_string(),
                worktree: state.workspace_root.to_string_lossy().to_string(),
                project: serde_json::json!({}),
                server_url: plugin_server_url.clone(),
            };
            match crate::plugin::bridge::load_external_plugins(specs, input).await {
                Ok(Some(bridge)) => {
                    plugin_manager.set_bridge(Arc::new(bridge));
                    plugin_manager
                        .notify_bridge(
                            "config",
                            serde_json::to_value(config)
                                .unwrap_or(serde_json::Value::Null),
                        )
                        .await;
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!("Warning: failed to load external plugins: {}", error);
                }
            }
        }
        let mut manager = state.mcp_manager.write().await;
        manager.start_configured(&config).await;
        let connected = manager
            .status()
            .values()
            .filter(|status| matches!(status, crate::mcp::McpServerStatus::Connected))
            .count();
        if connected > 0 {
            eprintln!("MCP servers connected: {}", connected);
        }
    }

    let selected_model = project_config
        .as_ref()
        .and_then(|config| config.model.as_deref());
    let credentials = load_provider_credentials(&credential_dir);
    match try_build_provider_from_config_auth_or_env(
        selected_model,
        project_config.as_ref(),
        Some(&credentials),
    ) {
        Ok(Some(provider)) => {
            eprintln!("Using provider: {}", provider.name());
            state = state.with_provider(provider);
        }
        Ok(None) => {
            eprintln!(
                "Warning: no provider credentials found; prompt routes will return 503 until a provider is configured."
            );
        }
        Err(e) => {
            eprintln!(
                "Warning: failed to initialize provider from environment: {}; prompt routes will return 503.",
                e
            );
        }
    }

    let router = crate::server::create_router_with_state(Arc::new(state));
    let display_host = if hostname == "0.0.0.0" {
        "localhost"
    } else {
        hostname.as_str()
    };
    let url = format!("http://{}:{}", display_host, local_addr.port());

    if open_web {
        println!("opencode web server listening on {}", url);
        open_browser(&url);
    } else {
        println!("opencode server listening on {}", url);
    }

    if let Err(e) = axum::serve(listener, router).await {
        eprintln!("Server error: {}", e);
        std::process::exit(1);
    }
}

fn provider_id_from_model(model: Option<&str>) -> Option<String> {
    let raw = model?.trim();
    if raw.is_empty() {
        return None;
    }

    let lower = raw.to_ascii_lowercase();
    let known_provider = match lower.as_str() {
        "alibaba" | "anthropic" | "azure" | "bedrock" | "cerebras" | "cohere" | "copilot"
        | "deepinfra" | "deepseek" | "fireworks" | "gitlab" | "google" | "groq" | "lmstudio"
        | "mistral" | "ollama" | "openai" | "openrouter" | "perplexity" | "together"
        | "togetherai" | "venice" | "vercel" | "vertex" | "xai" => Some(lower.clone()),
        "gemini" => Some("google".to_string()),
        "grok" => Some("xai".to_string()),
        "github-copilot" => Some("copilot".to_string()),
        "lm-studio" => Some("lmstudio".to_string()),
        _ => None,
    };
    if known_provider.is_some() {
        return known_provider;
    }

    if let Some((provider, _)) = lower.split_once('/') {
        if !provider.is_empty() {
            return Some(provider.to_string());
        }
    }

    if lower.contains("gpt")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.starts_with("codex-")
    {
        return Some("openai".to_string());
    }
    if lower.contains("claude") {
        return Some("anthropic".to_string());
    }
    if lower.contains("gemini") {
        return Some("google".to_string());
    }
    if lower.contains("grok") {
        return Some("xai".to_string());
    }

    None
}

fn infer_provider_id(model: Option<&str>, has_anthropic_key: bool, has_openai_key: bool) -> String {
    if let Some(provider_id) = provider_id_from_model(model) {
        return provider_id;
    }
    if has_anthropic_key {
        return "anthropic".to_string();
    }
    if has_openai_key {
        return "openai".to_string();
    }
    "anthropic".to_string()
}

type ProviderCredentials = std::collections::BTreeMap<String, provider_auth::ProviderCredential>;

fn load_provider_credentials(data_dir: &PathBuf) -> ProviderCredentials {
    provider_auth::ProviderAuthStore::new(data_dir.clone())
        .load()
        .unwrap_or_default()
}

fn build_provider_from_model_or_env(model: Option<&str>) -> anyhow::Result<Arc<dyn Provider>> {
    build_provider_from_model_auth_or_env(model, None)
}

fn build_provider_from_model_auth_or_env(
    model: Option<&str>,
    credentials: Option<&ProviderCredentials>,
) -> anyhow::Result<Arc<dyn Provider>> {
    build_provider_from_model_config_auth_or_env(model, None, credentials)
}

fn build_provider_from_model_config_or_env(
    model: Option<&str>,
    config: Option<&Config>,
) -> anyhow::Result<Arc<dyn Provider>> {
    build_provider_from_model_config_auth_or_env(model, config, None)
}

fn build_provider_from_model_config_auth_or_env(
    model: Option<&str>,
    config: Option<&Config>,
    credentials: Option<&ProviderCredentials>,
) -> anyhow::Result<Arc<dyn Provider>> {
    let env_model = std::env::var("OPENCODE_MODEL").ok();
    let model = model.or(env_model.as_deref());
    let provider_override = std::env::var("OPENCODE_PROVIDER").ok();
    let provider_id = provider_override
        .as_deref()
        .and_then(|p| provider_id_from_model(Some(p)))
        .or_else(|| provider_id_from_model(model))
        .or_else(|| provider_id_from_config(config))
        .or_else(|| provider_id_from_credentials(credentials))
        .or_else(provider_id_from_configured_env)
        .unwrap_or_else(|| {
            infer_provider_id(
                None,
                std::env::var_os("ANTHROPIC_API_KEY").is_some(),
                std::env::var_os("OPENAI_API_KEY").is_some(),
            )
        });
    if let Some(provider) = build_provider_from_config_with_auth(&provider_id, config, credentials)?
    {
        return Ok(provider);
    }
    if let Some(provider) = build_provider_from_auth(&provider_id, credentials)? {
        return Ok(provider);
    }
    build_provider_from_env(&provider_id)
}

fn provider_id_from_config(config: Option<&Config>) -> Option<String> {
    let config = config?;
    let providers = config.provider.as_ref()?;
    if providers.is_empty() {
        return None;
    }

    if let Some(enabled) = &config.enabled_providers {
        for provider in enabled {
            let normalized = normalize_provider_id(provider);
            if providers
                .iter()
                .any(|(id, entry)| configured_provider_matches(id, entry, &normalized))
            {
                return Some(normalized);
            }
        }
    }

    let disabled = config.disabled_providers.as_deref().unwrap_or(&[]);
    let mut candidates = providers.iter().filter_map(|(id, entry)| {
        if disabled
            .iter()
            .any(|item| normalize_provider_id(item) == normalize_provider_id(id))
        {
            return None;
        }
        if !provider_entry_has_runtime_config(entry) {
            return None;
        }
        Some(provider_runtime_id(id, entry))
    });

    let first = candidates.next()?;
    if candidates.next().is_some() {
        None
    } else {
        Some(first)
    }
}

fn build_provider_from_config(
    provider_id: &str,
    config: Option<&Config>,
) -> anyhow::Result<Option<Arc<dyn Provider>>> {
    build_provider_from_config_with_auth(provider_id, config, None)
}

fn build_provider_from_config_with_auth(
    provider_id: &str,
    config: Option<&Config>,
    credentials: Option<&ProviderCredentials>,
) -> anyhow::Result<Option<Arc<dyn Provider>>> {
    let provider_id = normalize_provider_id(provider_id);
    let Some((entry_id, entry)) = configured_provider_entry(config, &provider_id) else {
        return Ok(None);
    };
    let runtime_provider_id = provider_runtime_id(entry_id, entry);
    let configured_provider_id = normalize_provider_id(entry_id);

    let api_key = provider_api_key_from_config_entry(entry)
        .or_else(|| provider_api_key_from_credentials(&provider_id, credentials))
        .or_else(|| provider_api_key_from_credentials(&configured_provider_id, credentials));
    let base_url = provider_base_url_from_config_entry(entry);
    Ok(build_provider_from_parts(
        runtime_provider_id.as_str(),
        configured_provider_id,
        api_key,
        base_url,
    ))
}

fn build_provider_from_auth(
    provider_id: &str,
    credentials: Option<&ProviderCredentials>,
) -> anyhow::Result<Option<Arc<dyn Provider>>> {
    let provider_id = normalize_provider_id(provider_id);
    let Some(api_key) = provider_api_key_from_credentials(&provider_id, credentials) else {
        return Ok(None);
    };
    Ok(build_provider_from_parts(
        &provider_id,
        provider_id.clone(),
        Some(api_key),
        None,
    ))
}

fn build_provider_from_parts(
    runtime_provider_id: &str,
    configured_provider_id: String,
    api_key: Option<String>,
    base_url: Option<String>,
) -> Option<Arc<dyn Provider>> {
    match runtime_provider_id {
        "alibaba" => api_key.map(|key| Arc::new(AlibabaProvider::new(key)) as Arc<dyn Provider>),
        "anthropic" | "claude" => {
            api_key.map(|key| Arc::new(AnthropicProvider::new(key, None)) as Arc<dyn Provider>)
        }
        "cerebras" => api_key.map(|key| Arc::new(CerebrasProvider::new(key)) as Arc<dyn Provider>),
        "cohere" => api_key.map(|key| Arc::new(CohereProvider::new(key)) as Arc<dyn Provider>),
        "copilot" | "github-copilot" => {
            api_key.map(|key| Arc::new(GitHubCopilotProvider::new(key)) as Arc<dyn Provider>)
        }
        "deepinfra" => {
            api_key.map(|key| Arc::new(DeepInfraProvider::new(key)) as Arc<dyn Provider>)
        }
        "deepseek" => api_key.map(|key| Arc::new(DeepSeekProvider::new(key)) as Arc<dyn Provider>),
        "fireworks" => {
            api_key.map(|key| Arc::new(FireworksProvider::new(key)) as Arc<dyn Provider>)
        }
        "gitlab" => {
            api_key.map(|key| Arc::new(GitLabProvider::new(base_url, key)) as Arc<dyn Provider>)
        }
        "google" | "gemini" => {
            api_key.map(|key| Arc::new(GoogleProvider::new(key)) as Arc<dyn Provider>)
        }
        "groq" => api_key.map(|key| Arc::new(GroqProvider::new(key)) as Arc<dyn Provider>),
        "lmstudio" | "lm-studio" => {
            Some(Arc::new(LMStudioProvider::new(base_url)) as Arc<dyn Provider>)
        }
        "mistral" => api_key.map(|key| Arc::new(MistralProvider::new(key)) as Arc<dyn Provider>),
        "ollama" => Some(Arc::new(OllamaProvider::new(base_url)) as Arc<dyn Provider>),
        "openai" => api_key
            .or_else(|| base_url.as_ref().map(|_| String::new()))
            .map(|key| {
                let api_key = if key.is_empty() { None } else { Some(key) };
                Arc::new(OpenAIProvider::new_with_name(
                    configured_provider_id,
                    api_key,
                    base_url,
                    None,
                )) as Arc<dyn Provider>
            }),
        "openrouter" => {
            api_key.map(|key| Arc::new(OpenRouterProvider::new(key)) as Arc<dyn Provider>)
        }
        "perplexity" => {
            api_key.map(|key| Arc::new(PerplexityProvider::new(key)) as Arc<dyn Provider>)
        }
        "together" | "togetherai" => {
            api_key.map(|key| Arc::new(TogetherAIProvider::new(key)) as Arc<dyn Provider>)
        }
        "venice" => api_key.map(|key| Arc::new(VeniceProvider::new(key)) as Arc<dyn Provider>),
        "vercel" => api_key.map(|key| Arc::new(VercelProvider::new(key)) as Arc<dyn Provider>),
        "xai" | "grok" => api_key.map(|key| Arc::new(XAIProvider::new(key)) as Arc<dyn Provider>),
        _ => None,
    }
}

fn normalize_provider_id(provider_id: &str) -> String {
    provider_id_from_model(Some(provider_id))
        .unwrap_or_else(|| provider_id.trim().to_ascii_lowercase())
}

fn configured_provider_entry<'a>(
    config: Option<&'a Config>,
    provider_id: &str,
) -> Option<(&'a String, &'a ProviderConfigEntry)> {
    config?
        .provider
        .as_ref()?
        .iter()
        .find(|(id, entry)| configured_provider_matches(id, entry, provider_id))
}

fn configured_provider_matches(id: &str, entry: &ProviderConfigEntry, provider_id: &str) -> bool {
    normalize_provider_id(id) == provider_id
        || entry.id.as_deref().map(normalize_provider_id).as_deref() == Some(provider_id)
        || entry.api.as_deref().map(normalize_provider_id).as_deref() == Some(provider_id)
        || entry
            .npm
            .as_deref()
            .and_then(provider_id_from_npm)
            .as_deref()
            == Some(provider_id)
}

fn provider_runtime_id(id: &str, entry: &ProviderConfigEntry) -> String {
    entry
        .api
        .as_deref()
        .map(normalize_provider_id)
        .or_else(|| entry.npm.as_deref().and_then(provider_id_from_npm))
        .or_else(|| entry.id.as_deref().map(normalize_provider_id))
        .unwrap_or_else(|| normalize_provider_id(id))
}

fn provider_id_from_npm(npm: &str) -> Option<String> {
    match npm {
        "@ai-sdk/anthropic" => Some("anthropic".to_string()),
        "@ai-sdk/openai" | "@ai-sdk/openai-compatible" => Some("openai".to_string()),
        "@ai-sdk/google" => Some("google".to_string()),
        "@ai-sdk/xai" => Some("xai".to_string()),
        "@ai-sdk/mistral" => Some("mistral".to_string()),
        "@ai-sdk/groq" => Some("groq".to_string()),
        "@ai-sdk/deepinfra" => Some("deepinfra".to_string()),
        "@ai-sdk/cerebras" => Some("cerebras".to_string()),
        "@ai-sdk/cohere" => Some("cohere".to_string()),
        "@ai-sdk/togetherai" => Some("togetherai".to_string()),
        "@ai-sdk/perplexity" => Some("perplexity".to_string()),
        "@ai-sdk/vercel" => Some("vercel".to_string()),
        "@ai-sdk/alibaba" => Some("alibaba".to_string()),
        "@ai-sdk/github-copilot" => Some("copilot".to_string()),
        "@openrouter/ai-sdk-provider" => Some("openrouter".to_string()),
        "gitlab-ai-provider" => Some("gitlab".to_string()),
        "venice-ai-sdk-provider" => Some("venice".to_string()),
        _ => None,
    }
}

fn provider_entry_has_runtime_config(entry: &ProviderConfigEntry) -> bool {
    provider_api_key_from_config_entry(entry).is_some()
        || provider_base_url_from_config_entry(entry).is_some()
}

fn provider_api_key_from_config_entry(entry: &ProviderConfigEntry) -> Option<String> {
    entry
        .options
        .as_ref()
        .and_then(|options| options.api_key.clone())
        .filter(|key| !key.trim().is_empty())
        .or_else(|| {
            entry.env.as_ref().and_then(|env| {
                env.iter()
                    .find_map(|key| std::env::var(key).ok())
                    .filter(|value| !value.trim().is_empty())
            })
        })
}

fn provider_base_url_from_config_entry(entry: &ProviderConfigEntry) -> Option<String> {
    entry
        .options
        .as_ref()
        .and_then(|options| {
            options
                .base_url
                .clone()
                .or_else(|| options.enterprise_url.clone())
        })
        .filter(|url| !url.trim().is_empty())
}

fn provider_id_from_credentials(credentials: Option<&ProviderCredentials>) -> Option<String> {
    let credentials = credentials?;
    let mut ids = credentials
        .iter()
        .filter_map(|(id, credential)| match credential {
            provider_auth::ProviderCredential::Api { .. }
            | provider_auth::ProviderCredential::Oauth { .. } => Some(normalize_provider_id(id)),
            provider_auth::ProviderCredential::Wellknown { .. } => None,
        });
    let first = ids.next()?;
    if ids.next().is_some() {
        None
    } else {
        Some(first)
    }
}

fn provider_api_key_from_credentials(
    provider_id: &str,
    credentials: Option<&ProviderCredentials>,
) -> Option<String> {
    let credentials = credentials?;
    let normalized = normalize_provider_id(provider_id);
    credentials.iter().find_map(|(id, credential)| {
        if normalize_provider_id(id) != normalized {
            return None;
        }
        match credential {
            provider_auth::ProviderCredential::Api { key, .. } => {
                Some(key.clone()).filter(|key| !key.trim().is_empty())
            }
            // A stored OAuth credential exposes its short-lived `access`
            // token, which doubles as the bearer key for OpenAI-compatible
            // providers. Refreshing an expired token is handled separately.
            provider_auth::ProviderCredential::Oauth { access, .. } => {
                Some(access.clone()).filter(|access| !access.trim().is_empty())
            }
            provider_auth::ProviderCredential::Wellknown { .. } => None,
        }
    })
}

fn provider_api_key_env_names(provider_id: &str) -> Vec<String> {
    match normalize_provider_id(provider_id).as_str() {
        "alibaba" => vec!["ALIBABA_API_KEY".to_string()],
        "anthropic" => vec!["ANTHROPIC_API_KEY".to_string()],
        "azure" => vec!["AZURE_OPENAI_API_KEY".to_string()],
        "cerebras" => vec!["CEREBRAS_API_KEY".to_string()],
        "cohere" => vec!["COHERE_API_KEY".to_string()],
        "copilot" => vec!["GITHUB_TOKEN".to_string()],
        "deepinfra" => vec!["DEEPINFRA_API_KEY".to_string()],
        "deepseek" => vec!["DEEPSEEK_API_KEY".to_string()],
        "fireworks" => vec!["FIREWORKS_API_KEY".to_string()],
        "gitlab" => vec!["GITLAB_TOKEN".to_string()],
        "google" => vec!["GOOGLE_API_KEY".to_string(), "GEMINI_API_KEY".to_string()],
        "groq" => vec!["GROQ_API_KEY".to_string()],
        "mistral" => vec!["MISTRAL_API_KEY".to_string()],
        "openai" => vec!["OPENAI_API_KEY".to_string()],
        "openrouter" => vec!["OPENROUTER_API_KEY".to_string()],
        "perplexity" => vec!["PERPLEXITY_API_KEY".to_string()],
        "together" | "togetherai" => vec!["TOGETHER_API_KEY".to_string()],
        "venice" => vec!["VENICE_API_KEY".to_string()],
        "vercel" => vec!["VERCEL_API_KEY".to_string()],
        "xai" => vec!["XAI_API_KEY".to_string()],
        other => vec![format!(
            "{}_API_KEY",
            other.to_ascii_uppercase().replace('-', "_")
        )],
    }
}

fn provider_login_api_key(provider_id: &str) -> Option<String> {
    std::env::var("OPENCODE_PROVIDER_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            provider_api_key_env_names(provider_id)
                .into_iter()
                .find_map(|name| std::env::var(name).ok())
                .filter(|value| !value.trim().is_empty())
        })
}

fn provider_id_from_configured_env() -> Option<String> {
    let candidates = [
        ("ANTHROPIC_API_KEY", "anthropic"),
        ("OPENAI_API_KEY", "openai"),
        ("GOOGLE_API_KEY", "google"),
        ("AZURE_OPENAI_API_KEY", "azure"),
        ("AWS_ACCESS_KEY_ID", "bedrock"),
        ("GROQ_API_KEY", "groq"),
        ("MISTRAL_API_KEY", "mistral"),
        ("XAI_API_KEY", "xai"),
        ("OPENROUTER_API_KEY", "openrouter"),
        ("DEEPSEEK_API_KEY", "deepseek"),
        ("COHERE_API_KEY", "cohere"),
        ("PERPLEXITY_API_KEY", "perplexity"),
        ("TOGETHER_API_KEY", "together"),
        ("DEEPINFRA_API_KEY", "deepinfra"),
        ("CEREBRAS_API_KEY", "cerebras"),
        ("FIREWORKS_API_KEY", "fireworks"),
        ("ALIBABA_API_KEY", "alibaba"),
        ("VERCEL_API_KEY", "vercel"),
        ("VENICE_API_KEY", "venice"),
        ("GITLAB_TOKEN", "gitlab"),
        ("GITHUB_TOKEN", "copilot"),
        ("GOOGLE_ACCESS_TOKEN", "vertex"),
        ("OLLAMA_BASE_URL", "ollama"),
        ("LMSTUDIO_BASE_URL", "lmstudio"),
    ];

    candidates
        .iter()
        .find_map(|(env, provider)| std::env::var_os(env).map(|_| (*provider).to_string()))
}

fn try_build_provider_from_config_or_env(
    model: Option<&str>,
    config: Option<&Config>,
) -> anyhow::Result<Option<Arc<dyn Provider>>> {
    try_build_provider_from_config_auth_or_env(model, config, None)
}

fn try_build_provider_from_config_auth_or_env(
    model: Option<&str>,
    config: Option<&Config>,
    credentials: Option<&ProviderCredentials>,
) -> anyhow::Result<Option<Arc<dyn Provider>>> {
    let has_any_provider_hint = std::env::var_os("OPENCODE_PROVIDER").is_some()
        || std::env::var_os("OPENCODE_MODEL").is_some()
        || model.is_some()
        || provider_id_from_config(config).is_some()
        || provider_id_from_credentials(credentials).is_some()
        || provider_id_from_configured_env().is_some();

    if !has_any_provider_hint {
        return Ok(None);
    }

    build_provider_from_model_config_auth_or_env(model, config, credentials).map(Some)
}

fn build_provider_from_env(provider_id: &str) -> anyhow::Result<Arc<dyn Provider>> {
    let normalized = provider_id.to_ascii_lowercase();
    let provider: Arc<dyn Provider> = match normalized.as_str() {
        "alibaba" => Arc::new(AlibabaProvider::from_env()?),
        "anthropic" | "claude" => Arc::new(AnthropicProvider::from_env()?),
        "azure" | "azure-openai" => Arc::new(AzureProvider::from_env()?),
        "bedrock" | "aws-bedrock" => Arc::new(BedrockProvider::from_env()?),
        "cerebras" => Arc::new(CerebrasProvider::from_env()?),
        "cohere" => Arc::new(CohereProvider::from_env()?),
        "copilot" | "github-copilot" => Arc::new(GitHubCopilotProvider::from_env()?),
        "deepinfra" => Arc::new(DeepInfraProvider::from_env()?),
        "deepseek" => Arc::new(DeepSeekProvider::from_env()?),
        "fireworks" => Arc::new(FireworksProvider::from_env()?),
        "gitlab" => Arc::new(GitLabProvider::from_env()?),
        "google" | "gemini" => Arc::new(GoogleProvider::from_env()?),
        "groq" => Arc::new(GroqProvider::from_env()?),
        "lmstudio" | "lm-studio" => Arc::new(LMStudioProvider::from_env()?),
        "mistral" => Arc::new(MistralProvider::from_env()?),
        "ollama" => Arc::new(OllamaProvider::from_env()?),
        "openai" => Arc::new(OpenAIProvider::from_env()?),
        "openrouter" => Arc::new(OpenRouterProvider::from_env()?),
        "perplexity" => Arc::new(PerplexityProvider::from_env()?),
        "together" | "togetherai" => Arc::new(TogetherAIProvider::from_env()?),
        "venice" => Arc::new(VeniceProvider::from_env()?),
        "vercel" => Arc::new(VercelProvider::from_env()?),
        "vertex" | "google-vertex" | "gcp-vertex" => Arc::new(VertexProvider::from_env()?),
        "xai" | "grok" => Arc::new(XAIProvider::from_env()?),
        _ => anyhow::bail!("unknown provider '{}'", provider_id),
    };
    Ok(provider)
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let command = ("open", vec![url]);
    #[cfg(target_os = "linux")]
    let command = ("xdg-open", vec![url]);
    #[cfg(target_os = "windows")]
    let command = ("cmd", vec!["/C", "start", url]);

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        let _ = std::process::Command::new(command.0)
            .args(command.1)
            .spawn();
    }
}

async fn load_mcp_tools_from_project(
    project_path: &std::path::Path,
    data_dir: &std::path::Path,
) -> Vec<Arc<dyn crate::tool::Tool>> {
    let config = match crate::config::load_project_config(project_path) {
        Ok(Some(config)) => config,
        Ok(None) => return Vec::new(),
        Err(e) => {
            eprintln!("Warning: failed to load opencode config: {}", e);
            return Vec::new();
        }
    };

    let auth_store = Arc::new(crate::mcp::McpAuthStore::new(data_dir.to_path_buf()));
    let mut manager = crate::mcp::McpManager::new().with_auth_store(auth_store);
    manager.start_configured(&config).await;
    manager.runtime_tools().await
}

async fn handle_acp(args: args::AcpArgs, data_dir: PathBuf) {
    let cwd = args
        .cwd
        .map(|p| PathBuf::from(p))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let mut server = ACPServer::new(data_dir).await.unwrap_or_else(|e| {
        eprintln!("Failed to initialize ACP server: {}", e);
        std::process::exit(1);
    });

    if let Err(e) = server.run().await {
        eprintln!("ACP server error: {}", e);
        std::process::exit(1);
    }
}

pub fn run() {
    println!("Use async runtime. Call run_async() instead.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn provider_entry(
        api: Option<&str>,
        npm: Option<&str>,
        api_key: Option<&str>,
        base_url: Option<&str>,
    ) -> ProviderConfigEntry {
        ProviderConfigEntry {
            api: api.map(ToString::to_string),
            name: None,
            env: None,
            id: None,
            npm: npm.map(ToString::to_string),
            whitelist: None,
            blacklist: None,
            options: Some(crate::config::ProviderOptions {
                api_key: api_key.map(ToString::to_string),
                base_url: base_url.map(ToString::to_string),
                enterprise_url: None,
                set_cache_key: None,
                timeout: None,
                chunk_timeout: None,
            }),
            models: None,
        }
    }

    #[test]
    fn provider_id_from_model_prefers_provider_prefix() {
        assert_eq!(
            provider_id_from_model(Some("openai/gpt-4o")).as_deref(),
            Some("openai")
        );
        assert_eq!(
            provider_id_from_model(Some("anthropic/claude-3-5-sonnet-20241022")).as_deref(),
            Some("anthropic")
        );
    }

    #[test]
    fn provider_config_selects_single_configured_provider() {
        let config = Config {
            provider: Some(HashMap::from([(
                "openai".to_string(),
                provider_entry(None, None, Some("config-key"), None),
            )])),
            ..Default::default()
        };

        assert_eq!(
            provider_id_from_config(Some(&config)).as_deref(),
            Some("openai")
        );
        let provider = build_provider_from_model_config_or_env(None, Some(&config)).unwrap();
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn provider_config_maps_custom_openai_compatible_provider() {
        let config = Config {
            provider: Some(HashMap::from([(
                "local".to_string(),
                provider_entry(
                    None,
                    Some("@ai-sdk/openai-compatible"),
                    None,
                    Some("http://127.0.0.1:11434/v1"),
                ),
            )])),
            ..Default::default()
        };

        assert_eq!(
            provider_id_from_config(Some(&config)).as_deref(),
            Some("openai")
        );
        let provider =
            build_provider_from_model_config_or_env(Some("local/qwen2.5-coder"), Some(&config))
                .unwrap();
        assert_eq!(provider.name(), "local");
    }

    #[test]
    fn provider_id_from_model_keeps_legacy_model_name_heuristics() {
        assert_eq!(
            provider_id_from_model(Some("gpt-4o")).as_deref(),
            Some("openai")
        );
        assert_eq!(
            provider_id_from_model(Some("o1-preview")).as_deref(),
            Some("openai")
        );
        assert_eq!(
            provider_id_from_model(Some("claude-3-5-sonnet-20241022")).as_deref(),
            Some("anthropic")
        );
    }

    #[test]
    fn infer_provider_id_uses_model_then_environment_then_anthropic_default() {
        assert_eq!(
            infer_provider_id(Some("openrouter/anthropic/claude-3.5-sonnet"), false, true),
            "openrouter"
        );
        assert_eq!(infer_provider_id(None, false, true), "openai");
        assert_eq!(infer_provider_id(None, true, false), "anthropic");
        assert_eq!(infer_provider_id(None, false, false), "anthropic");
    }

    #[test]
    fn run_session_title_matches_title_flag_rules() {
        assert_eq!(
            run_session_title(Some("Custom"), "ignored prompt"),
            "Custom".to_string()
        );
        assert_eq!(
            run_session_title(Some(""), "short prompt"),
            "short prompt".to_string()
        );
        assert_eq!(
            run_session_title(None, "short prompt"),
            "Session: short prompt".to_string()
        );
    }

    #[test]
    fn run_user_parts_attach_files_before_text() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        std::fs::write(&file, "attached text").unwrap();
        let session_id = crate::id::SessionID::new();
        let message_id = crate::id::MessageID::new();

        let parts = build_run_user_parts(
            &session_id,
            &message_id,
            "prompt text",
            Some(&["note.txt".to_string()]),
            dir.path(),
        )
        .unwrap();

        assert_eq!(parts.len(), 2);
        match &parts[0] {
            crate::message::Part::File(part) => {
                assert_eq!(part.mime, "text/plain");
                assert_eq!(part.filename.as_deref(), Some("note.txt"));
                assert!(part.url.starts_with("file://"));
                match &part.source {
                    Some(crate::message::FilePartSource::File { text, .. }) => {
                        assert_eq!(text.value, "attached text");
                    }
                    _ => panic!("expected file source"),
                }
            }
            _ => panic!("expected file part first"),
        }
        match &parts[1] {
            crate::message::Part::Text(part) => assert_eq!(part.text, "prompt text"),
            _ => panic!("expected text part"),
        }
    }

    #[test]
    fn oauth_credential_resolves_access_token_as_provider_key() {
        let credentials: ProviderCredentials = std::collections::BTreeMap::from([(
            "openai".to_string(),
            provider_auth::ProviderCredential::Oauth {
                refresh: "refresh-token".to_string(),
                access: "access-token".to_string(),
                expires: Some(1),
                extra: Default::default(),
            },
        )]);

        assert_eq!(
            provider_api_key_from_credentials("openai", Some(&credentials)).as_deref(),
            Some("access-token")
        );
    }

    #[test]
    fn oauth_only_credentials_select_the_provider() {
        let credentials: ProviderCredentials = std::collections::BTreeMap::from([(
            "openai".to_string(),
            provider_auth::ProviderCredential::Oauth {
                refresh: "refresh-token".to_string(),
                access: "access-token".to_string(),
                expires: None,
                extra: Default::default(),
            },
        )]);

        assert_eq!(
            provider_id_from_credentials(Some(&credentials)).as_deref(),
            Some("openai")
        );
    }

    #[test]
    fn blank_oauth_access_token_is_ignored() {
        let credentials: ProviderCredentials = std::collections::BTreeMap::from([(
            "openai".to_string(),
            provider_auth::ProviderCredential::Oauth {
                refresh: "refresh-token".to_string(),
                access: "   ".to_string(),
                expires: None,
                extra: Default::default(),
            },
        )]);

        assert_eq!(
            provider_api_key_from_credentials("openai", Some(&credentials)),
            None
        );
    }
}
