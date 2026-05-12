pub mod args;

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

use crate::session::SessionStore;
use crate::session::PromptProcessor;
use crate::provider::{AnthropicProvider, OpenAIProvider, Provider};
use crate::acp::ACPServer;

pub async fn run_async() {
    let cli = args::Cli::parse();

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
        Some(args::Commands::Session { subcommand }) => {
            handle_session(subcommand, data_dir).await;
        }
        Some(args::Commands::Models(models_args)) => {
            handle_models(models_args).await;
        }
        Some(args::Commands::Providers { subcommand }) => {
            handle_providers(subcommand).await;
        }
        Some(args::Commands::Acp(acp_args)) => {
            handle_acp(acp_args, data_dir).await;
        }
        _ => {
            eprintln!("Command not yet implemented. Use 'opencode run <message>' to start a session.");
        }
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
    let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    
    let message = args.message.join(" ");
    
    let store = SessionStore::new(data_dir).await.unwrap_or_else(|e| {
        eprintln!("Failed to initialize database: {}", e);
        std::process::exit(1);
    });

    let session = if let Some(session_id_str) = &args.session {
        let session_id = crate::id::SessionID::parse(session_id_str).unwrap_or_else(|_| {
            eprintln!("Invalid session ID: {}", session_id_str);
            std::process::exit(1);
        });
        store.get(&session_id).await.unwrap_or_else(|e| {
            eprintln!("Failed to get session: {}", e);
            std::process::exit(1);
        }).unwrap_or_else(|| {
            eprintln!("Session not found: {}", session_id_str);
            std::process::exit(1);
        })
    } else {
        let title = format!("Session: {}", message.chars().take(50).collect::<String>());
        store.create(&title, "default", &project_path).await.unwrap_or_else(|e| {
            eprintln!("Failed to create session: {}", e);
            std::process::exit(1);
        })
    };

    println!("Session ID: {}", session.id);
    println!("Title: {}", session.title);

    let session_id = crate::id::SessionID::parse(&session.id).unwrap_or_else(|_| {
        eprintln!("Invalid session ID in database: {}", session.id);
        std::process::exit(1);
    });

    let provider: Arc<dyn Provider> = if let Some(model_str) = &args.model {
        if model_str.contains("gpt") || model_str.contains("o1") {
            Arc::new(OpenAIProvider::from_env().unwrap_or_else(|_| {
                eprintln!("Missing OPENAI_API_KEY environment variable");
                std::process::exit(1);
            }))
        } else {
            Arc::new(AnthropicProvider::from_env().unwrap_or_else(|_| {
                eprintln!("Missing ANTHROPIC_API_KEY environment variable");
                std::process::exit(1);
            }))
        }
    } else {
        Arc::new(AnthropicProvider::from_env().unwrap_or_else(|_| {
            eprintln!("Missing ANTHROPIC_API_KEY environment variable");
            std::process::exit(1);
        }))
    };

    println!("Using provider: {}", provider.name());
    println!("Default model: {}", provider.default_model().map(|m| m.id.as_ref().map(|i| i.to_string()).unwrap_or_default()).unwrap_or_default());

    let store = Arc::new(store);
    let processor = PromptProcessor::new(store, provider);
    
    println!("Processing: {}", message);
    let result = processor.process(&session_id, &message).await;
    
    match result {
        Ok(response) => {
            println!("\nResponse:\n{}", response);
        }
        Err(e) => {
            eprintln!("Error processing prompt: {}", e);
            std::process::exit(1);
        }
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
            
            if sessions.is_empty() {
                println!("No sessions found.");
            } else {
                println!("Sessions:");
                for session in sessions {
                    let archived = if session.time_archived.is_some() { " (archived)" } else { "" };
                    println!("  {} - {}{}", session.id, session.title, archived);
                }
            }
        }
        args::SessionSubcommand::Delete(delete_args) => {
            let session_id = crate::id::SessionID::parse(&delete_args.session_id).unwrap_or_else(|_| {
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

async fn handle_models(args: args::ModelsArgs) {
    let provider: Box<dyn Provider> = if let Some(provider_name) = &args.provider {
        match provider_name.as_str() {
            "openai" => Box::new(OpenAIProvider::from_env().unwrap_or_else(|_| {
                eprintln!("Missing OPENAI_API_KEY");
                std::process::exit(1);
            })),
            "anthropic" => Box::new(AnthropicProvider::from_env().unwrap_or_else(|_| {
                eprintln!("Missing ANTHROPIC_API_KEY");
                std::process::exit(1);
            })),
            _ => {
                eprintln!("Unknown provider: {}", provider_name);
                std::process::exit(1);
            }
        }
    } else {
        Box::new(AnthropicProvider::from_env().unwrap_or_else(|_| {
            eprintln!("Missing ANTHROPIC_API_KEY");
            std::process::exit(1);
        }))
    };

    println!("Provider: {}", provider.name());
    println!("Models:");
    for model in provider.models() {
        let model_id = model.id.as_ref().map(|i| i.to_string()).unwrap_or_default();
        let name = model.name.clone().unwrap_or_default();
        println!("  {} - {}", model_id, name);
    }
}

async fn handle_providers(subcommand: args::ProvidersSubcommand) {
    match subcommand {
        args::ProvidersSubcommand::List => {
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
        }
        args::ProvidersSubcommand::Login { url, provider, method } => {
            let provider_name = provider.unwrap_or_else(|| "unknown".to_string());
            println!("Provider login for {} not yet implemented.", provider_name);
        }
        args::ProvidersSubcommand::Logout { provider } => {
            let provider_name = provider.unwrap_or_else(|| "unknown".to_string());
            println!("Provider logout for {} not yet implemented.", provider_name);
        }
    }
}

async fn handle_acp(args: args::AcpArgs, data_dir: PathBuf) {
    let cwd = args.cwd
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