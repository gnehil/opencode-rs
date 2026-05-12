use std::io::{BufRead, Write, stdin, stdout};
use std::sync::Arc;
use tokio::sync::mpsc;

use anyhow::Result;
use serde_json::Value;

use crate::acp::agent::{ACPAgent, JsonRpcNotification};
use crate::acp::session::ACPSessionManager;
use crate::acp::types::*;
use crate::session::SessionStore;
use crate::provider::{Provider, ProviderID};
use crate::provider::anthropic::AnthropicProvider;
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
        let stdin = stdin();
        let mut stdout = stdout();

        let (request_tx, mut request_rx) = mpsc::channel::<String>(64);

        tokio::spawn(async move {
            let stdin = stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        if request_tx.send(l).await.is_err() {
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
                    let request: JsonRpcRequest = serde_json::from_str(&line)
                        .map_err(|e| {
                            let response = error_response(None, PARSE_ERROR, format!("Parse error: {}", e));
                            let _ = writeln!(stdout, "{}", serde_json::to_string(&response).unwrap());
                            e
                        })?;

                    let response = self.handle_request(request).await;

                    let json_response = serde_json::to_string(&response)?;
                    writeln!(stdout, "{}", json_response)?;
                    stdout.flush()?;
                }

                Some(notification) = self.notification_rx.recv() => {
                    let notification_json = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": notification.method,
                        "params": notification.params
                    });
                    writeln!(stdout, "{}", serde_json::to_string(&notification_json)?)?;
                    stdout.flush()?;
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