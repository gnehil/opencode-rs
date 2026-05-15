use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use anyhow::Result;
use serde_json::Value;

use crate::acp::session::ACPSessionManager;
use crate::acp::types::*;
use crate::bus::{Event, EventBus};
use crate::provider::{Provider, ProviderID};
use crate::session::SessionStore;

pub struct ACPAgent {
    session_manager: Arc<ACPSessionManager>,
    store: Arc<SessionStore>,
    provider: Arc<dyn Provider>,
    provider_id: ProviderID,
    version: String,
    event_bus: EventBus,
    notification_tx: mpsc::Sender<JsonRpcNotification>,
    event_started: Arc<RwLock<bool>>,
    shell_snapshots: Arc<RwLock<HashMap<String, String>>>,
    tool_starts: Arc<RwLock<HashSet<String>>>,
    permission_queues: Arc<RwLock<HashMap<String, tokio::sync::Mutex<()>>>>,
    // Per-session cancellation: handle_cancel notifies any in-flight prompt
    // running for the given session_id, which causes handle_prompt to return
    // a StopReason::Cancelled response and skip persisting the partial
    // assistant message.
    cancel_signals: Arc<RwLock<HashMap<String, Arc<tokio::sync::Notify>>>>,
    // Tool registry. The provider sees these as available functions and can
    // request invocation via tool_calls; handle_prompt then executes them
    // locally and feeds results back as role=tool messages.
    tools: Vec<Arc<dyn crate::tool::Tool>>,
    /// Maximum tool-use iterations per prompt before we force EndTurn to
    /// prevent doom loops.
    max_iterations: usize,
}

pub struct JsonRpcNotification {
    pub method: String,
    pub params: Value,
}

impl ACPAgent {
    pub fn new(
        session_manager: Arc<ACPSessionManager>,
        store: Arc<SessionStore>,
        provider: Arc<dyn Provider>,
        provider_id: ProviderID,
        event_bus: EventBus,
        notification_tx: mpsc::Sender<JsonRpcNotification>,
    ) -> Self {
        let tools = crate::tool::default_registry();
        Self {
            session_manager,
            store,
            provider,
            provider_id,
            version: env!("CARGO_PKG_VERSION").to_string(),
            event_bus,
            notification_tx,
            event_started: Arc::new(RwLock::new(false)),
            shell_snapshots: Arc::new(RwLock::new(HashMap::new())),
            tool_starts: Arc::new(RwLock::new(HashSet::new())),
            permission_queues: Arc::new(RwLock::new(HashMap::new())),
            cancel_signals: Arc::new(RwLock::new(HashMap::new())),
            tools,
            max_iterations: 10,
        }
    }

    pub fn with_tools(mut self, tools: Vec<Arc<dyn crate::tool::Tool>>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n;
        self
    }

    pub async fn start_event_subscription(&self) {
        let mut started = self.event_started.write().await;
        if *started {
            return;
        }
        *started = true;

        let bus = self.event_bus.clone();
        let tx = self.notification_tx.clone();
        let agent = Arc::new(self.clone());

        tokio::spawn(async move {
            let mut rx = bus.listener();
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Err(e) = agent.handle_event(event, &tx).await {
                            tracing::error!("Failed to handle event: {}", e);
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Event bus lagged by {} messages", n);
                        continue;
                    }
                }
            }
        });
    }

    async fn handle_event(
        &self,
        event: Event,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        match event {
            Event::PermissionAsked(perm) => {
                self.handle_permission_asked(perm, tx).await?;
            }
            Event::MessagePartUpdated(part) => {
                self.handle_message_part_updated(part, tx).await?;
            }
            Event::MessagePartDelta(delta) => {
                self.handle_message_part_delta(delta, tx).await?;
            }
            Event::ToolStart(tool) => {
                self.handle_tool_start(tool, tx).await?;
            }
            Event::ToolComplete(tool) => {
                self.handle_tool_complete(tool, tx).await?;
            }
            Event::ToolError(tool) => {
                self.handle_tool_error(tool, tx).await?;
            }
            Event::MessageStream(stream) => {
                self.handle_message_stream(stream, tx).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_permission_asked(
        &self,
        perm: crate::bus::event::PermissionAskedEvent,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let session = self.session_manager.try_get(&perm.session_id).await;
        if session.is_none() {
            return Ok(());
        }

        let options = vec![
            PermissionOption {
                option_id: "once".to_string(),
                kind: PermissionKind::AllowOnce,
                name: "Allow once".to_string(),
            },
            PermissionOption {
                option_id: "always".to_string(),
                kind: PermissionKind::AllowAlways,
                name: "Always allow".to_string(),
            },
            PermissionOption {
                option_id: "reject".to_string(),
                kind: PermissionKind::RejectOnce,
                name: "Reject".to_string(),
            },
        ];

        let tool_call = ToolCallInfo {
            tool_call_id: perm
                .tool_call_id
                .clone()
                .unwrap_or_else(|| perm.permission_id.clone()),
            status: ToolCallStatus::Pending,
            title: perm.permission_type.clone(),
            kind: Some(to_tool_kind(&perm.permission_type)),
            locations: vec![],
            raw_input: perm.metadata.clone(),
        };

        let notification = JsonRpcNotification {
            method: "requestPermission".to_string(),
            params: serde_json::to_value(RequestPermissionRequest {
                session_id: perm.session_id.clone(),
                tool_call,
                options,
            })?,
        };

        tx.send(notification).await?;

        Ok(())
    }

    async fn handle_message_part_updated(
        &self,
        part: crate::bus::event::MessagePartUpdatedEvent,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let session = self.session_manager.try_get(&part.session_id).await;
        if session.is_none() {
            return Ok(());
        }

        let session_id = session.unwrap().id;

        if part.part_type == "tool" {
            let call_id = part.part.call_id.clone().unwrap_or_default();
            let tool_name = part.part.tool.clone().unwrap_or_default();
            let status = part.part.state.status.clone();

            match status.as_str() {
                "pending" => {
                    self.shell_snapshots.write().await.remove(&call_id);
                }
                "running" => {
                    let output = part.part.state.metadata.and_then(|m| {
                        m.get("output")
                            .and_then(|o| o.as_str())
                            .map(|s| s.to_string())
                    });

                    if let Some(output) = output {
                        let hash = fast_hash(&output);
                        let snapshots = self.shell_snapshots.read().await;
                        let prev_hash = snapshots.get(&call_id);

                        if prev_hash.map(|h| h == &hash).unwrap_or(false) {
                            let update = SessionUpdateType::ToolCallUpdate {
                                tool_call_id: call_id.clone(),
                                status: ToolCallStatus::InProgress,
                                kind: Some(to_tool_kind(&tool_name)),
                                title: Some(tool_name.clone()),
                                locations: Some(vec![]),
                                raw_input: Some(
                                    part.part.state.input.clone().unwrap_or(Value::Null),
                                ),
                                raw_output: None,
                                content: vec![ToolCallContent::Content {
                                    content: ContentBlock::Text {
                                        text: output.clone(),
                                        annotations: None,
                                    },
                                }],
                            };
                            self.send_session_update(session_id, update, tx).await?;
                            return Ok(());
                        }
                        self.shell_snapshots
                            .write()
                            .await
                            .insert(call_id.clone(), hash);
                    }

                    let update = SessionUpdateType::ToolCallUpdate {
                        tool_call_id: call_id.clone(),
                        status: ToolCallStatus::InProgress,
                        kind: Some(to_tool_kind(&tool_name)),
                        title: Some(tool_name.clone()),
                        locations: Some(vec![]),
                        raw_input: Some(part.part.state.input.clone().unwrap_or(Value::Null)),
                        raw_output: None,
                        content: vec![],
                    };
                    self.send_session_update(session_id, update, tx).await?;
                }
                "completed" => {
                    self.tool_starts.write().await.remove(&call_id);
                    self.shell_snapshots.write().await.remove(&call_id);

                    let kind = to_tool_kind(&tool_name);
                    let content = build_tool_content(&part.part, &kind);

                    if tool_name == "todowrite" {
                        if let Some(output) = &part.part.state.output {
                            if let Ok(todos) = serde_json::from_str::<Vec<TodoEntry>>(output) {
                                let entries = todos
                                    .iter()
                                    .map(|t| PlanEntry {
                                        priority: t
                                            .priority
                                            .clone()
                                            .unwrap_or_else(|| "medium".to_string()),
                                        status: if t.status == "cancelled" {
                                            "completed"
                                        } else {
                                            &t.status
                                        }
                                        .to_string(),
                                        content: t.content.clone(),
                                    })
                                    .collect();

                                let update = SessionUpdateType::Plan { entries };
                                self.send_session_update(session_id.clone(), update, tx)
                                    .await?;
                            }
                        }
                    }

                    let update = SessionUpdateType::ToolCallUpdate {
                        tool_call_id: call_id.clone(),
                        status: ToolCallStatus::Completed,
                        kind: Some(kind.clone()),
                        title: Some(part.part.state.title.clone().unwrap_or_default()),
                        locations: Some(vec![]),
                        raw_input: Some(part.part.state.input.clone().unwrap_or(Value::Null)),
                        raw_output: Some(serde_json::json!({
                            "output": part.part.state.output,
                            "metadata": part.part.state.metadata
                        })),
                        content,
                    };
                    self.send_session_update(session_id, update, tx).await?;
                }
                "error" => {
                    self.tool_starts.write().await.remove(&call_id);
                    self.shell_snapshots.write().await.remove(&call_id);

                    let update = SessionUpdateType::ToolCallUpdate {
                        tool_call_id: call_id.clone(),
                        status: ToolCallStatus::Failed,
                        kind: Some(to_tool_kind(&tool_name)),
                        title: Some(tool_name.clone()),
                        locations: Some(vec![]),
                        raw_input: Some(part.part.state.input.clone().unwrap_or(Value::Null)),
                        raw_output: Some(serde_json::json!({
                            "error": part.part.state.error,
                            "metadata": part.part.state.metadata
                        })),
                        content: vec![ToolCallContent::Content {
                            content: ContentBlock::Text {
                                text: part.part.state.error.clone().unwrap_or_default(),
                                annotations: None,
                            },
                        }],
                    };
                    self.send_session_update(session_id, update, tx).await?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_message_part_delta(
        &self,
        delta: crate::bus::event::MessagePartDeltaEvent,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let session = self.session_manager.try_get(&delta.session_id).await;
        if session.is_none() {
            return Ok(());
        }

        let session_id = session.unwrap().id;

        let messages = self
            .store
            .get_messages_with_parts(&crate::id::SessionID::parse(&delta.session_id)?)
            .await?;
        let message = messages.iter().find(|m| match &m.info {
            crate::message::Message::Assistant(a) => a.id.to_string() == delta.message_id,
            _ => false,
        });

        if message.is_none() {
            return Ok(());
        }

        let part_type = delta.field.clone();
        if part_type == "text" || part_type == "reasoning" {
            let update_type = if part_type == "reasoning" {
                SessionUpdateType::AgentThoughtChunk {
                    message_id: delta.message_id.clone(),
                    content: TextContent {
                        type_: "text".to_string(),
                        text: delta.delta.clone(),
                        annotations: None,
                    },
                }
            } else {
                SessionUpdateType::AgentMessageChunk {
                    message_id: delta.message_id.clone(),
                    content: TextContent {
                        type_: "text".to_string(),
                        text: delta.delta.clone(),
                        annotations: None,
                    },
                }
            };
            self.send_session_update(session_id, update_type, tx)
                .await?;
        }

        Ok(())
    }

    async fn handle_tool_start(
        &self,
        tool: crate::bus::event::ToolStartEvent,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let session = self.session_manager.try_get(&tool.session_id).await;
        if session.is_none() {
            return Ok(());
        }

        let session_id = session.unwrap().id;
        let call_id = tool.tool_call_id.clone();

        if self.tool_starts.read().await.contains(&call_id) {
            return Ok(());
        }
        self.tool_starts.write().await.insert(call_id.clone());

        let update = SessionUpdateType::ToolCall {
            tool_call_id: call_id.clone(),
            title: tool.tool_name.clone(),
            kind: to_tool_kind(&tool.tool_name),
            status: ToolCallStatus::Pending,
            locations: vec![],
            raw_input: tool.tool_input.clone(),
        };
        self.send_session_update(session_id, update, tx).await?;

        Ok(())
    }

    async fn handle_tool_complete(
        &self,
        tool: crate::bus::event::ToolCompleteEvent,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let session = self.session_manager.try_get(&tool.session_id).await;
        if session.is_none() {
            return Ok(());
        }

        let session_id = session.unwrap().id;
        let call_id = tool.tool_call_id.clone();

        self.tool_starts.write().await.remove(&call_id);

        let output_text = tool.tool_output.as_str().unwrap_or("");
        let kind = to_tool_kind(&tool.tool_name);

        let content = vec![ToolCallContent::Content {
            content: ContentBlock::Text {
                text: output_text.to_string(),
                annotations: None,
            },
        }];

        let update = SessionUpdateType::ToolCallUpdate {
            tool_call_id: call_id.clone(),
            status: ToolCallStatus::Completed,
            kind: Some(kind.clone()),
            title: Some(tool.tool_name.clone()),
            locations: Some(vec![]),
            raw_input: Some(Value::Null),
            raw_output: Some(tool.tool_output.clone()),
            content,
        };
        self.send_session_update(session_id, update, tx).await?;

        Ok(())
    }

    async fn handle_tool_error(
        &self,
        tool: crate::bus::event::ToolErrorEvent,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let session = self.session_manager.try_get(&tool.session_id).await;
        if session.is_none() {
            return Ok(());
        }

        let session_id = session.unwrap().id;
        let call_id = tool.tool_call_id.clone();

        self.tool_starts.write().await.remove(&call_id);

        let update = SessionUpdateType::ToolCallUpdate {
            tool_call_id: call_id.clone(),
            status: ToolCallStatus::Failed,
            kind: Some(to_tool_kind(&tool.tool_name)),
            title: Some(tool.tool_name.clone()),
            locations: Some(vec![]),
            raw_input: Some(Value::Null),
            raw_output: Some(serde_json::json!({ "error": tool.error })),
            content: vec![ToolCallContent::Content {
                content: ContentBlock::Text {
                    text: tool.error.clone(),
                    annotations: None,
                },
            }],
        };
        self.send_session_update(session_id, update, tx).await?;

        Ok(())
    }

    async fn handle_message_stream(
        &self,
        stream: crate::bus::event::MessageStreamEvent,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let session = self.session_manager.try_get(&stream.session_id).await;
        if session.is_none() {
            return Ok(());
        }

        let session_id = session.unwrap().id;

        let update = SessionUpdateType::AgentMessageChunk {
            message_id: stream.message_id.clone(),
            content: TextContent {
                type_: "text".to_string(),
                text: stream.delta.clone(),
                annotations: None,
            },
        };
        self.send_session_update(session_id, update, tx).await?;

        Ok(())
    }

    async fn send_session_update(
        &self,
        session_id: String,
        update: SessionUpdateType,
        tx: &mpsc::Sender<JsonRpcNotification>,
    ) -> Result<()> {
        let notification = JsonRpcNotification {
            method: "session/update".to_string(),
            params: serde_json::to_value(SessionUpdate { session_id, update })?,
        };
        tx.send(notification).await?;
        Ok(())
    }

    pub async fn handle_initialize(&self, params: Value) -> Result<Value> {
        let request: InitializeRequest = serde_json::from_value(params)?;

        let auth_method = AuthMethod {
            id: "opencode-login".to_string(),
            name: "Login with opencode".to_string(),
            description: "Run `opencode auth login` in the terminal".to_string(),
            _meta: if request
                .client_capabilities
                .as_ref()
                .and_then(|c| c._meta.as_ref())
                .and_then(|m| m.get("terminal-auth"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                Some(HashMap::from([(
                    "terminal-auth".to_string(),
                    serde_json::json!({
                        "command": "opencode",
                        "args": ["auth", "login"],
                        "label": "OpenCode Login"
                    }),
                )]))
            } else {
                None
            },
        };

        self.start_event_subscription().await;

        let response = InitializeResponse {
            protocol_version: 1,
            agent_capabilities: AgentCapabilities {
                load_session: true,
                mcp_capabilities: McpCapabilities {
                    http: true,
                    sse: true,
                },
                prompt_capabilities: PromptCapabilities {
                    embedded_context: true,
                    image: true,
                },
                session_capabilities: SessionCapabilities {
                    close: SessionCloseCapabilities {},
                    fork: SessionForkCapabilities {},
                    list: SessionListCapabilities {},
                    resume: SessionResumeCapabilities {},
                },
            },
            auth_methods: vec![auth_method],
            agent_info: AgentInfo {
                name: "OpenCode".to_string(),
                version: self.version.clone(),
            },
        };

        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_new_session(&self, params: Value) -> Result<Value> {
        let request: NewSessionRequest = serde_json::from_value(params)?;

        let model = self.get_default_model()?;
        let state = self
            .session_manager
            .create(
                request.cwd.clone(),
                request.mcp_servers.clone(),
                Some(model.clone()),
            )
            .await?;

        let providers = self.get_available_providers();
        let available_models = build_available_models(&providers);
        let modes = self.get_available_modes();

        let response = NewSessionResponse {
            session_id: state.id.clone(),
            config_options: Some(build_config_options(
                &model,
                None,
                &available_models,
                &modes,
            )),
            models: Some(ModelsInfo {
                current_model_id: format_model_id(&model, None),
                available_models,
            }),
            modes: Some(modes),
            _meta: Some(build_variant_meta(&model, None)),
        };

        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_load_session(&self, params: Value) -> Result<Value> {
        let request: LoadSessionRequest = serde_json::from_value(params)?;

        let model = self.get_default_model()?;
        let state = self
            .session_manager
            .load(
                &request.session_id,
                request.cwd.clone(),
                request.mcp_servers.clone(),
                Some(model.clone()),
            )
            .await?;

        let providers = self.get_available_providers();
        let available_models = build_available_models(&providers);
        let modes = self.get_available_modes();

        let messages = self
            .store
            .get_messages_with_parts(&crate::id::SessionID::parse(&request.session_id)?)
            .await?;

        for msg in messages.iter() {
            self.process_message(msg.clone(), &request.session_id)
                .await?;
        }

        let response = NewSessionResponse {
            session_id: state.id.clone(),
            config_options: Some(build_config_options(
                &model,
                None,
                &available_models,
                &modes,
            )),
            models: Some(ModelsInfo {
                current_model_id: format_model_id(&model, None),
                available_models,
            }),
            modes: Some(modes),
            _meta: Some(build_variant_meta(&model, None)),
        };

        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_list_sessions(&self, params: Value) -> Result<Value> {
        let request: ListSessionsRequest = serde_json::from_value(params)?;
        let sessions = self.session_manager.list(request.cwd.as_deref()).await?;

        let entries: Vec<SessionInfo> = sessions
            .iter()
            .map(|s| SessionInfo {
                session_id: s.id.clone(),
                cwd: s.directory.clone(),
                title: Some(s.title.clone()),
                updated_at: chrono::DateTime::from_timestamp_millis(s.time_updated)
                    .unwrap_or_else(|| chrono::Utc::now())
                    .to_rfc3339(),
            })
            .collect();

        let response = ListSessionsResponse {
            sessions: entries,
            next_cursor: None,
        };

        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_close_session(&self, params: Value) -> Result<Value> {
        let request: CloseSessionRequest = serde_json::from_value(params)?;
        self.session_manager.remove(&request.session_id).await;
        self.permission_queues
            .write()
            .await
            .remove(&request.session_id);

        let response = CloseSessionResponse {};
        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_fork_session(&self, params: Value) -> Result<Value> {
        let request: ForkSessionRequest = serde_json::from_value(params)?;

        let model = self.get_default_model()?;
        let state = self
            .session_manager
            .create(
                request.cwd.clone(),
                request.mcp_servers.clone(),
                Some(model.clone()),
            )
            .await?;

        let providers = self.get_available_providers();
        let available_models = build_available_models(&providers);
        let modes = self.get_available_modes();

        let response = NewSessionResponse {
            session_id: state.id.clone(),
            config_options: Some(build_config_options(
                &model,
                None,
                &available_models,
                &modes,
            )),
            models: Some(ModelsInfo {
                current_model_id: format_model_id(&model, None),
                available_models,
            }),
            modes: Some(modes),
            _meta: Some(build_variant_meta(&model, None)),
        };

        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_resume_session(&self, params: Value) -> Result<Value> {
        let request: ResumeSessionRequest = serde_json::from_value(params)?;

        let model = self.get_default_model()?;
        let state = self
            .session_manager
            .load(
                &request.session_id,
                request.cwd.clone(),
                request.mcp_servers.clone(),
                Some(model.clone()),
            )
            .await?;

        let providers = self.get_available_providers();
        let available_models = build_available_models(&providers);
        let modes = self.get_available_modes();

        let response = NewSessionResponse {
            session_id: state.id.clone(),
            config_options: Some(build_config_options(
                &model,
                None,
                &available_models,
                &modes,
            )),
            models: Some(ModelsInfo {
                current_model_id: format_model_id(&model, None),
                available_models,
            }),
            modes: Some(modes),
            _meta: Some(build_variant_meta(&model, None)),
        };

        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_set_session_model(&self, params: Value) -> Result<Value> {
        let request: SetSessionModelRequest = serde_json::from_value(params)?;

        let selection = parse_model_selection(&request.model_id);
        self.session_manager
            .set_model(&request.session_id, Some(selection.clone()))
            .await?;

        Ok(serde_json::json!({
            "_meta": build_variant_meta(&selection, None)
        }))
    }

    pub async fn handle_set_session_mode(&self, params: Value) -> Result<Value> {
        let request: SetSessionModeRequest = serde_json::from_value(params)?;
        self.session_manager
            .set_mode(&request.session_id, request.mode_id)
            .await?;
        Ok(serde_json::json!(null))
    }

    pub async fn handle_set_session_config_option(&self, params: Value) -> Result<Value> {
        let request: SetSessionConfigOptionRequest = serde_json::from_value(params)?;
        let session = self.session_manager.get(&request.session_id).await?;

        let model = session
            .model
            .clone()
            .unwrap_or_else(|| self.get_default_model().unwrap());

        match request.config_id.as_str() {
            "model" => {
                let model_id = request
                    .value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("model value must be a string"))?;
                let selection = parse_model_selection(model_id);
                self.session_manager
                    .set_model(&request.session_id, Some(selection.clone()))
                    .await?;
            }
            "effort" => {
                let variant = request
                    .value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("effort value must be a string"))?;
                self.session_manager
                    .set_variant(&request.session_id, Some(variant.to_string()))
                    .await?;
            }
            "mode" => {
                let mode_id = request
                    .value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("mode value must be a string"))?;
                self.session_manager
                    .set_mode(&request.session_id, mode_id.to_string())
                    .await?;
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Unknown config option: {}",
                    request.config_id
                ));
            }
        }

        let providers = self.get_available_providers();
        let available_models = build_available_models(&providers);
        let modes = self.get_available_modes();
        let updated_session = self.session_manager.get(&request.session_id).await?;
        let updated_model = updated_session.model.unwrap_or(model);

        let response = SetSessionConfigOptionResponse {
            config_options: build_config_options(
                &updated_model,
                updated_session.variant.as_deref(),
                &available_models,
                &modes,
            ),
        };

        Ok(serde_json::to_value(response)?)
    }

    pub async fn handle_prompt(&self, params: Value) -> Result<Value> {
        let request: PromptRequest = serde_json::from_value(params)?;

        let session = self.session_manager.get(&request.session_id).await?;
        let model = match session.model.clone() {
            Some(m) => m,
            None => self.get_default_model()?,
        };

        let prompt_text = self.extract_prompt_text(&request.prompt);

        let session_id = crate::id::SessionID::parse(&request.session_id)?;
        let cwd = session.cwd.clone();
        let now = chrono::Utc::now().timestamp_millis();

        // 1. Persist the user turn so it shows up in the rebuilt history.
        let user_message_id = crate::id::MessageID::new();
        let user_msg = crate::message::UserMessage {
            id: user_message_id.clone(),
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime { created: now },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: crate::message::ModelRef {
                provider_id: model.provider_id.clone(),
                model_id: model.model_id.clone(),
                variant: session.variant.clone(),
            },
            system: None,
            tools: None,
        };
        self.store
            .save_message(&session_id, &crate::message::Message::User(user_msg))
            .await?;
        self.store
            .save_text_part(&session_id, &user_message_id, &prompt_text)
            .await?;
        self.event_bus.publish(crate::bus::Event::message_create(
            request.session_id.clone(),
            user_message_id.to_string(),
            crate::bus::MessageRole::User,
        ));

        // 2. Register a per-session cancel handle.
        let cancel_notify = Arc::new(tokio::sync::Notify::new());
        {
            let mut signals = self.cancel_signals.write().await;
            signals.insert(request.session_id.clone(), cancel_notify.clone());
        }

        // 3. Build tool defs + system prompt once per prompt; both are
        //    deterministic functions of (tools, session.cwd).
        let tool_defs: Vec<crate::provider::ToolDefinition> = self
            .tools
            .iter()
            .map(|t| crate::provider::ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect();
        // Per-session agent (defaults to "build") drives both the
        // permission ruleset and the system-prompt persona.
        let agent_name = self
            .store
            .get(&session_id)
            .await
            .ok()
            .flatten()
            .and_then(|row| row.agent)
            .unwrap_or_else(|| "build".to_string());
        let system_prompt = build_system_prompt_for_agent(&cwd, &self.tools, &agent_name);

        let mut total_input: u64 = 0;
        let mut total_output: u64 = 0;
        let mut total_cache_read: u64 = 0;
        let mut total_cache_write: u64 = 0;
        let mut stop_reason = StopReason::EndTurn;

        for _ in 0..self.max_iterations {
            // Rebuild history on every turn so the assistant's tool_use and
            // our tool_result are visible to the model.
            let history =
                crate::session::build_completion_messages(self.store.as_ref(), &session_id).await?;

            let completion_request = crate::provider::CompletionRequest {
                model: crate::provider::ModelID::new(&model.model_id),
                messages: history,
                system: Some(system_prompt.clone()),
                tools: tool_defs.clone(),
                max_tokens: Some(4096),
                temperature: None,
                top_p: None,
                stop_sequences: None,
                extra_headers: std::collections::HashMap::new(),
            };

            let response = tokio::select! {
                r = crate::session::complete_with_retry(
                    self.provider.as_ref(),
                    completion_request,
                    crate::session::retry::DEFAULT_MAX_ATTEMPTS,
                    std::time::Duration::from_millis(crate::session::retry::DEFAULT_BASE_DELAY_MS),
                ) => r?,
                _ = cancel_notify.notified() => {
                    self.cancel_signals.write().await.remove(&request.session_id);
                    let prompt_response = PromptResponse {
                        stop_reason: StopReason::Cancelled,
                        usage: None,
                        _meta: HashMap::new(),
                    };
                    return Ok(serde_json::to_value(prompt_response)?);
                }
            };

            total_input += response.usage.input;
            total_output += response.usage.output;
            total_cache_read += response.usage.cache_read.unwrap_or(0);
            total_cache_write += response.usage.cache_write.unwrap_or(0);

            // 4. Persist the assistant turn (text part now, tool parts below).
            let assistant_message_id = crate::id::MessageID::new();
            let turn_time = chrono::Utc::now().timestamp_millis();
            let assistant_msg = crate::message::AssistantMessage {
                id: assistant_message_id.clone(),
                session_id: session_id.clone(),
                role: "assistant".to_string(),
                time: crate::message::AssistantTime {
                    created: turn_time,
                    completed: Some(turn_time),
                },
                error: None,
                parent_id: user_message_id.to_string(),
                model_id: model.model_id.clone(),
                provider_id: model.provider_id.clone(),
                mode: session
                    .mode_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
                agent: "build".to_string(),
                path: crate::message::PathInfo {
                    cwd: cwd.clone(),
                    root: "/".to_string(),
                },
                summary: None,
                cost: 0.0,
                tokens: crate::message::TokenUsage {
                    input: response.usage.input as f64,
                    output: response.usage.output as f64,
                    reasoning: 0.0,
                    total: None,
                    cache: crate::message::CacheUsage {
                        read: response.usage.cache_read.unwrap_or(0) as f64,
                        write: response.usage.cache_write.unwrap_or(0) as f64,
                    },
                },
                structured: None,
                variant: session.variant.clone(),
                finish: None,
            };
            self.store
                .save_message(
                    &session_id,
                    &crate::message::Message::Assistant(assistant_msg),
                )
                .await?;
            if !response.content.is_empty() {
                self.store
                    .save_text_part(&session_id, &assistant_message_id, &response.content)
                    .await?;
            }
            self.event_bus.publish(crate::bus::Event::message_create(
                request.session_id.clone(),
                assistant_message_id.to_string(),
                crate::bus::MessageRole::Assistant,
            ));

            // 5. If the model didn't request any tools, this turn is done.
            if response.tool_calls.is_empty() {
                stop_reason = StopReason::EndTurn;
                break;
            }

            // 6. Otherwise execute each requested tool, persist a ToolPart
            //    per call, and loop. The next iteration's history rebuild
            //    will surface these as role=tool messages to the model.
            self.execute_and_persist_tool_calls(
                &session_id,
                &assistant_message_id,
                &cwd,
                &response.tool_calls,
            )
            .await?;

            // If the model only ever runs tool calls, ToolUse is the
            // terminal stop_reason once max_iterations is reached.
            stop_reason = StopReason::ToolUse;

            // 7. Auto-compact if the running input usage is approaching
            //    the model's context ceiling. We use the cumulative
            //    `input` count from this prompt's iterations; on the
            //    first iteration of a fresh prompt that's
            //    response.usage.input, on later ones the running total.
            if let Some(model_info) = self.provider.default_model() {
                if crate::session::should_compact(total_input, model_info) {
                    if let Err(e) = crate::session::compact_session(
                        &self.store,
                        &session_id,
                        &self.provider,
                        &model.model_id,
                    )
                    .await
                    {
                        tracing::warn!("compaction failed (continuing without): {}", e);
                    }
                }
            }
        }

        self.cancel_signals
            .write()
            .await
            .remove(&request.session_id);

        let usage = Usage {
            total_tokens: Some(total_input as i64 + total_output as i64),
            input_tokens: Some(total_input as i64),
            output_tokens: Some(total_output as i64),
            thought_tokens: None,
            cached_read_tokens: if total_cache_read > 0 {
                Some(total_cache_read as i64)
            } else {
                None
            },
            cached_write_tokens: if total_cache_write > 0 {
                Some(total_cache_write as i64)
            } else {
                None
            },
        };

        let prompt_response = PromptResponse {
            stop_reason,
            usage: Some(usage),
            _meta: HashMap::new(),
        };
        Ok(serde_json::to_value(prompt_response)?)
    }

    /// Execute each tool call the model asked for and persist a ToolPart
    /// per call (Completed on success, Error on failure / unknown tool).
    /// Shared shape with `session::processor`.
    async fn execute_and_persist_tool_calls(
        &self,
        session_id: &crate::id::SessionID,
        message_id: &crate::id::MessageID,
        cwd: &str,
        tool_calls: &[crate::provider::ToolCall],
    ) -> Result<()> {
        use crate::session::service::ToolPartResult;
        use crate::tool::ToolContext;

        let working_dir = std::path::PathBuf::from(cwd);

        // Look up the session's configured agent so its permission
        // ruleset gates the tool calls. Falls back to "build" (the
        // unrestricted default) if the row has no agent set, or if no
        // agent registry entry matches.
        let agent_name = self
            .store
            .get(session_id)
            .await
            .ok()
            .flatten()
            .and_then(|row| row.agent)
            .unwrap_or_else(|| "build".to_string());
        let permission_rules = crate::agent::get_agent(&agent_name)
            .map(|a| a.permission)
            .unwrap_or_default();

        for call in tool_calls {
            let params: Value =
                serde_json::from_str(&call.arguments).unwrap_or(serde_json::json!({}));

            self.event_bus.publish(crate::bus::Event::tool_start(
                session_id.to_string(),
                call.name.clone(),
                params.clone(),
            ));

            let outcome = match self.tools.iter().find(|t| t.name() == call.name) {
                Some(tool) => {
                    let ctx = ToolContext {
                        session_id: session_id.clone(),
                        working_dir: working_dir.clone(),
                        permission_rules: permission_rules.clone(),
                        event_bus: Some(self.event_bus.clone()),
                        permission_broker: None,
                        provider: None,
                        store: None,
                        config: None,
                        agent_name: Some(agent_name.clone()),
                        model_id: None,
                        plugin_manager: None,
                        question_broker: None,
                    };
                    match tool.execute(params.clone(), ctx).await {
                        Ok(r) => ToolPartResult::Completed {
                            output: r.output,
                            attachments: r.attachments.unwrap_or_default(),
                        },
                        Err(e) => ToolPartResult::Error {
                            error: e.to_string(),
                        },
                    }
                }
                None => ToolPartResult::Error {
                    error: format!("Unknown tool: {}", call.name),
                },
            };

            self.store
                .save_tool_part(
                    session_id,
                    message_id,
                    &call.name,
                    &call.id,
                    &params,
                    match &outcome {
                        ToolPartResult::Completed {
                            output,
                            attachments,
                        } => ToolPartResult::Completed {
                            output: output.clone(),
                            attachments: attachments.clone(),
                        },
                        ToolPartResult::Error { error } => ToolPartResult::Error {
                            error: error.clone(),
                        },
                    },
                )
                .await?;

            match outcome {
                ToolPartResult::Completed { output, .. } => {
                    self.event_bus.publish(crate::bus::Event::tool_complete(
                        session_id.to_string(),
                        call.name.clone(),
                        serde_json::json!({"result": output}),
                    ));
                }
                ToolPartResult::Error { error } => {
                    self.event_bus.publish(crate::bus::Event::tool_error(
                        session_id.to_string(),
                        call.name.clone(),
                        error,
                    ));
                }
            }
        }
        Ok(())
    }

    fn extract_prompt_text(&self, prompt: &[PromptContent]) -> String {
        prompt
            .iter()
            .filter_map(|p| match p {
                PromptContent::Text { text, .. } => Some(text.clone()),
                PromptContent::Resource { resource } => match resource {
                    ResourceContent::Text { text, .. } => Some(text.clone()),
                    ResourceContent::Blob { .. } => None,
                },
                _ => None,
            })
            .collect::<Vec<String>>()
            .join("\n")
    }

    pub async fn handle_cancel(&self, params: Value) -> Result<()> {
        let request: CancelNotification = serde_json::from_value(params)?;
        // Validate the session exists; surface "unknown session" to the
        // client rather than silently swallowing.
        let _ = self.session_manager.get(&request.session_id).await?;

        // Take the notify out so subsequent cancels for the same session_id
        // are harmless no-ops until a new prompt starts.
        let signal = self
            .cancel_signals
            .write()
            .await
            .remove(&request.session_id);
        if let Some(notify) = signal {
            notify.notify_waiters();
        }
        Ok(())
    }

    async fn process_message(
        &self,
        msg: crate::message::WithParts,
        session_id: &str,
    ) -> Result<()> {
        let session = self.session_manager.try_get(session_id).await;
        if session.is_none() {
            return Ok(());
        }
        let acp_session_id = session.unwrap().id;

        for part in msg.parts.iter() {
            match part {
                crate::message::Part::Tool(tool_part) => {
                    let call_id = tool_part.call_id.to_string();
                    let tool_name = tool_part.tool.clone();

                    let update = SessionUpdateType::ToolCall {
                        tool_call_id: call_id.clone(),
                        title: tool_name.clone(),
                        kind: to_tool_kind(&tool_name),
                        status: ToolCallStatus::Pending,
                        locations: vec![],
                        raw_input: tool_state_input(&tool_part.state),
                    };

                    let notification_tx = self.notification_tx.clone();
                    self.send_session_update(acp_session_id.clone(), update, &notification_tx)
                        .await?;
                }
                crate::message::Part::Text(text_part) => {
                    let text = &text_part.text;
                    {
                        let update_type = match &msg.info {
                            crate::message::Message::User(_) => {
                                SessionUpdateType::UserMessageChunk {
                                    message_id: match &msg.info {
                                        crate::message::Message::User(u) => u.id.to_string(),
                                        _ => "".to_string(),
                                    },
                                    content: ContentBlock::Text {
                                        text: text.clone(),
                                        annotations: None,
                                    },
                                }
                            }
                            crate::message::Message::Assistant(_) => {
                                SessionUpdateType::AgentMessageChunk {
                                    message_id: match &msg.info {
                                        crate::message::Message::Assistant(a) => a.id.to_string(),
                                        _ => "".to_string(),
                                    },
                                    content: TextContent {
                                        type_: "text".to_string(),
                                        text: text.clone(),
                                        annotations: None,
                                    },
                                }
                            }
                        };

                        let notification_tx = self.notification_tx.clone();
                        self.send_session_update(
                            acp_session_id.clone(),
                            update_type,
                            &notification_tx,
                        )
                        .await?;
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn get_default_model(&self) -> Result<ModelSelection> {
        let default_model = self
            .provider
            .default_model()
            .ok_or_else(|| anyhow::anyhow!("No default model available"))?;

        let model_id = default_model
            .id
            .clone()
            .map(|id| id.to_string())
            .unwrap_or_else(|| "default".to_string());

        Ok(ModelSelection {
            provider_id: self.provider_id.to_string(),
            model_id,
        })
    }

    fn get_available_providers(&self) -> Vec<ProviderEntry> {
        vec![ProviderEntry {
            id: self.provider_id.to_string(),
            name: self.provider.name().to_string(),
            models: HashMap::new(),
        }]
    }

    fn get_available_modes(&self) -> ModesInfo {
        ModesInfo {
            available_modes: vec![ModeOption {
                id: "default".to_string(),
                name: "Default".to_string(),
                description: Some("Default agent mode".to_string()),
            }],
            current_mode_id: "default".to_string(),
        }
    }
}

impl Clone for ACPAgent {
    fn clone(&self) -> Self {
        Self {
            session_manager: self.session_manager.clone(),
            store: self.store.clone(),
            provider: self.provider.clone(),
            provider_id: self.provider_id.clone(),
            version: self.version.clone(),
            event_bus: self.event_bus.clone(),
            notification_tx: self.notification_tx.clone(),
            event_started: self.event_started.clone(),
            shell_snapshots: self.shell_snapshots.clone(),
            tool_starts: self.tool_starts.clone(),
            permission_queues: self.permission_queues.clone(),
            cancel_signals: self.cancel_signals.clone(),
            tools: self.tools.clone(),
            max_iterations: self.max_iterations,
        }
    }
}

struct ProviderEntry {
    id: String,
    name: String,
    models: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TodoEntry {
    content: String,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
}

/// Construct the system prompt the model receives at the start of a
/// turn.
///
/// Composition:
///   * Generic preamble (role, working directory, "prefer reading
///     before writing"-style guidance).
///   * Available tools list.
///   * Agent-specific persona (`agent::get_agent(name).prompt`) if
///     the agent has one. The `explore`, `scout`, `compaction`,
///     `title`, and `summary` agents ship with their own prompts;
///     `build`/`plan`/`general` use the preamble alone.
///
/// Falls back to no agent-specific prompt if the agent name doesn't
/// resolve in the registry, so callers can pass arbitrary strings
/// without crashing.
pub fn build_system_prompt(cwd: &str, tools: &[Arc<dyn crate::tool::Tool>]) -> String {
    build_system_prompt_for_agent(cwd, tools, "build")
}

pub fn build_system_prompt_for_agent(
    cwd: &str,
    tools: &[Arc<dyn crate::tool::Tool>],
    agent_name: &str,
) -> String {
    let agent_info = crate::agent::get_agent(agent_name);
    build_system_prompt_for_agent_info(cwd, tools, agent_info.as_ref())
}

pub fn build_system_prompt_for_agent_info(
    cwd: &str,
    tools: &[Arc<dyn crate::tool::Tool>],
    agent_info: Option<&crate::agent::AgentInfo>,
) -> String {
    let mut s = String::new();
    s.push_str("You are an autonomous coding agent operating inside a developer's project.\n");
    s.push_str(&format!("Working directory: {}\n", cwd));
    s.push_str("Use the provided tools to inspect and modify the project. ");
    s.push_str("Prefer reading before writing, and avoid destructive shell commands unless explicitly asked.\n\n");
    if !tools.is_empty() {
        s.push_str("Available tools:\n");
        for tool in tools {
            s.push_str(&format!("- {}: {}\n", tool.name(), tool.description()));
        }
        s.push('\n');
    }
    if let Some(agent_info) = agent_info {
        if let Some(persona) = agent_info.prompt.as_ref().filter(|p| !p.trim().is_empty()) {
            s.push_str("---\n");
            s.push_str(persona);
            if !persona.ends_with('\n') {
                s.push('\n');
            }
        }
    }
    s
}

fn tool_state_input(state: &crate::message::ToolState) -> Value {
    use crate::message::ToolState;
    let map = match state {
        ToolState::Pending(p) => &p.input,
        ToolState::Running(r) => &r.input,
        ToolState::Completed(c) => &c.input,
        ToolState::Error(e) => &e.input,
    };
    serde_json::to_value(map).unwrap_or(Value::Null)
}

fn build_available_models(providers: &[ProviderEntry]) -> Vec<ModelOption> {
    providers
        .iter()
        .map(|p| ModelOption {
            model_id: format!("{}/*", p.id),
            name: format!("{} (default)", p.name),
        })
        .collect()
}

fn build_config_options(
    model: &ModelSelection,
    variant: Option<&str>,
    available_models: &[ModelOption],
    modes: &ModesInfo,
) -> Vec<SessionConfigOption> {
    vec![
        SessionConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            description: None,
            category: "model".to_string(),
            type_: "select".to_string(),
            current_value: format_model_id(model, variant),
            options: available_models
                .iter()
                .map(|m| ConfigOptionValue {
                    value: m.model_id.clone(),
                    name: m.name.clone(),
                })
                .collect(),
        },
        SessionConfigOption {
            id: "mode".to_string(),
            name: "Session Mode".to_string(),
            description: None,
            category: "mode".to_string(),
            type_: "select".to_string(),
            current_value: modes.current_mode_id.clone(),
            options: modes
                .available_modes
                .iter()
                .map(|m| ConfigOptionValue {
                    value: m.id.clone(),
                    name: m.name.clone(),
                })
                .collect(),
        },
    ]
}

fn format_model_id(model: &ModelSelection, variant: Option<&str>) -> String {
    if let Some(v) = variant {
        format!("{}/{}/{}", model.provider_id, model.model_id, v)
    } else {
        format!("{}/{}", model.provider_id, model.model_id)
    }
}

fn build_variant_meta(
    model: &ModelSelection,
    variant: Option<&str>,
) -> HashMap<String, serde_json::Value> {
    HashMap::from([(
        "opencode".to_string(),
        serde_json::json!({
            "modelId": format!("{}/{}", model.provider_id, model.model_id),
            "variant": variant,
            "availableVariants": []
        }),
    )])
}

fn parse_model_selection(model_id: &str) -> ModelSelection {
    let parts: Vec<&str> = model_id.split('/').collect();
    if parts.len() >= 2 {
        ModelSelection {
            provider_id: parts[0].to_string(),
            model_id: parts[1].to_string(),
        }
    } else {
        ModelSelection {
            provider_id: "anthropic".to_string(),
            model_id: model_id.to_string(),
        }
    }
}

fn to_tool_kind(tool_name: &str) -> ToolKind {
    let tool = tool_name.to_lowercase();
    match tool.as_str() {
        "bash" | "shell" => ToolKind::Execute,
        "webfetch" => ToolKind::Fetch,
        "edit" | "patch" | "write" => ToolKind::Edit,
        "grep" | "glob" | "repo_clone" | "repo_overview" => ToolKind::Search,
        "read" => ToolKind::Read,
        _ => ToolKind::Other,
    }
}

fn fast_hash(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

fn build_tool_content(
    part: &crate::bus::event::MessagePartData,
    kind: &ToolKind,
) -> Vec<ToolCallContent> {
    let mut content = Vec::new();

    if let Some(output) = &part.state.output {
        content.push(ToolCallContent::Content {
            content: ContentBlock::Text {
                text: output.clone(),
                annotations: None,
            },
        });
    }

    if *kind == ToolKind::Edit {
        let input = part.state.input.clone().unwrap_or(Value::Null);
        let filepath = input.get("filePath").and_then(|v| v.as_str()).unwrap_or("");
        let old_text = input
            .get("oldString")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_text = input
            .get("newString")
            .and_then(|v| v.as_str())
            .or_else(|| input.get("content").and_then(|v| v.as_str()))
            .unwrap_or("");

        content.push(ToolCallContent::Diff {
            path: filepath.to_string(),
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
        });
    }

    if let Some(attachments) = &part.state.attachments {
        for att in attachments.iter() {
            if att.mime.starts_with("image/") {
                let data_url = att.url.clone();
                if let Some(data) = data_url
                    .strip_prefix("data:")
                    .and_then(|s| s.split(";base64,").nth(1))
                {
                    content.push(ToolCallContent::Content {
                        content: ContentBlock::Image {
                            mime_type: att.mime.clone(),
                            data: data.to_string(),
                            uri: None,
                        },
                    });
                }
            }
        }
    }

    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_includes_cwd_and_tools() {
        let tools: Vec<Arc<dyn crate::tool::Tool>> = vec![
            Arc::new(crate::tool::BashTool),
            Arc::new(crate::tool::ReadTool),
        ];
        let prompt = build_system_prompt("/repo/x", &tools);
        assert!(prompt.contains("/repo/x"));
        assert!(prompt.contains("bash:"));
        assert!(prompt.contains("read:"));
    }

    #[test]
    fn system_prompt_handles_empty_tools() {
        let prompt = build_system_prompt("/repo/x", &[]);
        assert!(prompt.contains("/repo/x"));
        assert!(!prompt.contains("Available tools:"));
    }

    #[test]
    fn system_prompt_includes_agent_persona() {
        // `explore` agent ships with a persona prompt; verify it gets
        // appended.
        let prompt = build_system_prompt_for_agent("/x", &[], "explore");
        // The explore prompt mentions read/grep/glob tools by name.
        // Look for any token from the explore.txt prompt header.
        let explore_persona = crate::agent::prompts::PROMPT_EXPLORE;
        assert!(
            prompt.contains(explore_persona),
            "explore persona not present"
        );
        assert!(prompt.contains("---")); // separator
    }

    #[test]
    fn system_prompt_omits_persona_for_personaless_agent() {
        // `build` agent has no `prompt` field — the persona section
        // should not appear.
        let prompt = build_system_prompt_for_agent("/x", &[], "build");
        assert!(!prompt.contains("---"));
    }

    #[test]
    fn system_prompt_unknown_agent_falls_back_gracefully() {
        // Unknown agent: still produce the preamble; no panic.
        let prompt = build_system_prompt_for_agent("/x", &[], "nonexistent-agent");
        assert!(prompt.contains("/x"));
        assert!(!prompt.contains("---"));
    }
}
