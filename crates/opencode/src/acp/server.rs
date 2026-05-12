use std::io::{BufRead, stdin};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use anyhow::Result;
use serde_json::Value;

use crate::acp::agent::{ACPAgent, JsonRpcNotification};
use crate::acp::session::ACPSessionManager;
use crate::acp::types::*;
use crate::session::SessionStore;
use crate::provider::{Provider, ProviderID};
use crate::provider::AnthropicProvider;
use crate::bus::EventBus;

pub struct ACPServer {
    agent: Arc<ACPAgent>,
    notification_rx: mpsc::Receiver<JsonRpcNotification>,
}

impl ACPServer {
    pub async fn new(data_dir: std::path::PathBuf) -> Result<Self> {
        let store = Arc::new(SessionStore::new(data_dir).await?);
        let session_manager = Arc::new(ACPSessionManager::new(store.clone()));
        let event_bus = EventBus::new();

        let (notification_tx, notification_rx) = mpsc::channel::<JsonRpcNotification>(256);

        let provider: Arc<dyn Provider> = Arc::new(
            AnthropicProvider::from_env()
                .map_err(|_| anyhow::anyhow!("Missing ANTHROPIC_API_KEY"))?
        );

        let agent = Arc::new(ACPAgent::new(
            session_manager,
            store,
            provider,
            ProviderID::anthropic(),
            event_bus,
            notification_tx,
        ));

        Ok(Self { agent, notification_rx })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut stdout = tokio::io::stdout();

        let (request_tx, mut request_rx) = mpsc::channel::<String>(64);

        // stdin's only blocking API is BufRead::lines(); we own the FD for the
        // lifetime of the ACP server so it's fine to park a dedicated OS
        // thread on it.
        std::thread::spawn(move || {
            let stdin = stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        if request_tx.blocking_send(l).is_err() {
                            break;
                        }
                    }
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        });

        loop {
            tokio::select! {
                Some(line) = request_rx.recv() => {
                    let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
                        Ok(request) => self.handle_request(request).await,
                        Err(e) => error_response(
                            None,
                            PARSE_ERROR,
                            format!("Parse error: {}", e),
                        ),
                    };

                    let json_response = serde_json::to_string(&response)?;
                    stdout.write_all(json_response.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }

                Some(notification) = self.notification_rx.recv() => {
                    let notification_json = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": notification.method,
                        "params": notification.params
                    });
                    let serialized = serde_json::to_string(&notification_json)?;
                    stdout.write_all(serialized.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }

                else => break,
            }
        }

        Ok(())
    }

    async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let id = request.id.clone();

        let result = match request.method.as_str() {
            "initialize" => {
                self.agent.handle_initialize(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/new" => {
                self.agent.handle_new_session(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/load" => {
                self.agent.handle_load_session(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/list" => {
                self.agent.handle_list_sessions(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/close" => {
                self.agent.handle_close_session(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/fork" => {
                self.agent.handle_fork_session(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/resume" => {
                self.agent.handle_resume_session(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/setModel" => {
                self.agent.handle_set_session_model(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/setMode" => {
                self.agent.handle_set_session_mode(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/setConfigOption" => {
                self.agent.handle_set_session_config_option(request.params.unwrap_or(Value::Null))
                    .await
            }
            "session/prompt" => {
                self.agent.handle_prompt(request.params.unwrap_or(Value::Null))
                    .await
            }
            "cancel" => {
                self.agent.handle_cancel(request.params.unwrap_or(Value::Null))
                    .await
                    .map(|_| Value::Null)
            }
            "shutdown" => {
                Ok(Value::Null)
            }
            _ => {
                Err(anyhow::anyhow!("Method not found: {}", request.method))
            }
        };

        match result {
            Ok(value) => success_response(id, value),
            Err(e) => {
                let message = e.to_string();
                let code = if message.contains("not found") {
                    METHOD_NOT_FOUND
                } else if message.contains("Invalid") || message.contains("must be") {
                    INVALID_PARAMS
                } else {
                    INTERNAL_ERROR
                };
                error_response(id, code, message)
            }
        }
    }
}