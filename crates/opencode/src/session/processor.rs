use std::sync::Arc;

use crate::bus::{Event, EventBus, MessageRole};
use crate::id::{MessageID, SessionID};
use crate::message::{AssistantMessage, Message, ModelRef, Part, UserMessage, UserTime};
use crate::message::{AssistantTime, CacheUsage, PathInfo, TokenUsage};
use crate::provider::{CompletionMessage, CompletionRequest, Provider, ToolDefinition};
use crate::session::service::ToolPartResult;
use crate::session::SessionStore;
use crate::tool::{Tool, ToolContext};

pub struct PromptProcessor {
    store: Arc<SessionStore>,
    provider: Arc<dyn Provider>,
    tools: Vec<Arc<dyn Tool>>,
    bus: EventBus,
    permission_broker: Option<crate::permission::PermissionBroker>,
    question_broker: Option<crate::question::QuestionBroker>,
    skip_permissions: bool,
    plugin_manager: Option<Arc<crate::plugin::PluginManager>>,
    max_iterations: usize,
    agent_name: String,
    config: Option<crate::config::Config>,
    model_id: Option<String>,
    /// Per-turn provider variant (e.g. `high`, `max`, `minimal`) — stamped on
    /// the user/assistant message ModelRef and propagated to the provider
    /// request so reasoning-enabled models pick up the requested effort.
    variant: Option<String>,
    session_permission_rules: crate::permission::Ruleset,
}

pub enum ProcessEvent {
    TextDelta(String),
    ToolStart(String, serde_json::Value),
    ToolComplete(String, serde_json::Value),
    /// Aggregated reasoning / "thinking" output from the provider, when the
    /// provider returns it as a separate channel (e.g. Claude extended
    /// thinking). Emitted after the assistant turn completes.
    Reasoning(String),
    /// Boundary marker fired at the beginning of each provider turn.
    StepStart,
    /// Boundary marker fired at the end of each provider turn, carrying
    /// the provider's stop reason when known.
    StepFinish {
        stop_reason: Option<String>,
    },
    Done(String),
    Error(String),
}

impl PromptProcessor {
    pub fn new(store: Arc<SessionStore>, provider: Arc<dyn Provider>) -> Self {
        Self {
            store,
            provider,
            tools: crate::tool::default_registry(),
            bus: EventBus::new(),
            permission_broker: None,
            question_broker: None,
            skip_permissions: false,
            plugin_manager: None,
            max_iterations: 10,
            agent_name: "build".to_string(),
            config: None,
            model_id: None,
            variant: None,
            session_permission_rules: Vec::new(),
        }
    }

    /// Set the per-turn variant (`high`, `max`, `minimal`, …) so it can be
    /// stamped on persisted messages and surfaced to providers that vary
    /// reasoning effort.
    pub fn with_variant(mut self, variant: impl Into<String>) -> Self {
        let variant = variant.into();
        if !variant.is_empty() {
            self.variant = Some(variant);
        }
        self
    }

    /// Override the agent. The agent name selects:
    ///   * the permission ruleset applied to tool calls (from
    ///     `agent::get_agent(name).permission`)
    ///   * the value stamped on persisted assistant messages as
    ///     `agent` so the UI can show "you're in plan mode" etc.
    pub fn with_agent(mut self, agent_name: impl Into<String>) -> Self {
        self.agent_name = agent_name.into();
        self
    }

    pub fn with_config(mut self, config: crate::config::Config) -> Self {
        self.config = Some(config);
        self
    }

    pub fn with_tools(mut self, tools: Vec<Arc<dyn Tool>>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_session_permission_rules(mut self, rules: crate::permission::Ruleset) -> Self {
        self.session_permission_rules = rules;
        self
    }

    pub fn with_model_selection(self, selection: &str) -> Self {
        match model_id_from_selection(selection) {
            Some(model_id) => self.with_model(model_id),
            None => self,
        }
    }

    pub fn with_bus(mut self, bus: EventBus) -> Self {
        self.bus = bus;
        self
    }

    pub fn with_permission_broker(mut self, broker: crate::permission::PermissionBroker) -> Self {
        self.permission_broker = Some(broker);
        self
    }

    pub fn with_question_broker(mut self, broker: crate::question::QuestionBroker) -> Self {
        self.question_broker = Some(broker);
        self
    }

    pub fn with_skip_permissions(mut self, skip: bool) -> Self {
        self.skip_permissions = skip;
        self
    }

    pub fn with_plugin_manager(
        mut self,
        plugin_manager: Arc<crate::plugin::PluginManager>,
    ) -> Self {
        self.plugin_manager = Some(plugin_manager);
        self
    }

    pub fn bus(&self) -> EventBus {
        self.bus.clone()
    }

    pub async fn process(&self, session_id: &SessionID, prompt: &str) -> anyhow::Result<String> {
        let events = self.process_stream(session_id, prompt).await?;

        for event in &events {
            if let ProcessEvent::Done(text) = event {
                return Ok(text.clone());
            }
        }

        Ok("No response generated".to_string())
    }

    pub async fn process_stream(
        &self,
        session_id: &SessionID,
        prompt: &str,
    ) -> anyhow::Result<Vec<ProcessEvent>> {
        let user_message_id = MessageID::new();
        let user_parts = vec![text_part(session_id, &user_message_id, prompt)];
        self.process_stream_with_parts(session_id, prompt, user_message_id, user_parts)
            .await
    }

    pub async fn process_stream_with_parts(
        &self,
        session_id: &SessionID,
        prompt: &str,
        user_message_id: MessageID,
        user_parts: Vec<Part>,
    ) -> anyhow::Result<Vec<ProcessEvent>> {
        let mut events = Vec::new();
        let mut accumulated_content = String::new();
        let mut total_input_tokens: u64 = 0;

        let model_id = self
            .model_id
            .clone()
            .or_else(|| {
                self.provider
                    .default_model()
                    .and_then(|m| m.id.clone())
                    .map(|m| m.to_string())
            })
            .unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string());

        // Build the user message + initial parts in memory, fire the
        // `chat.message` hook so plugins can rewrite EITHER, then persist
        // whichever the hook returned. Doing this before `save_message`
        // means plugins that swap out fields like `agent` or model variant
        // actually have an effect — saving first like the previous
        // implementation made message-level edits a silent no-op.
        let now = chrono::Utc::now().timestamp_millis();
        let mut user_msg = UserMessage {
            id: user_message_id.clone(),
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: UserTime { created: now },
            format: None,
            summary: None,
            agent: self.agent_name.clone(),
            model: ModelRef {
                provider_id: self.provider.name().to_string(),
                model_id: model_id.clone(),
                variant: self.variant.clone(),
            },
            system: None,
            tools: None,
        };

        // Synthesize a text part from a raw prompt so plugins see a uniform
        // `{ message, parts }` payload regardless of caller shape.
        let mut user_parts: Vec<Part> = if user_parts.is_empty() {
            vec![text_part(session_id, &user_message_id, prompt)]
        } else {
            user_parts
        };

        if let Some(plugin_manager) = &self.plugin_manager {
            let user_message_json =
                serde_json::to_value(&user_msg).unwrap_or(serde_json::Value::Null);
            let output = plugin_manager
                .trigger_bridge(
                    "chat.message",
                    serde_json::json!({
                        "sessionID": session_id.to_string(),
                        "agent": self.agent_name,
                        "model": {
                            "providerID": self.provider.name(),
                            "modelID": model_id,
                        },
                        "messageID": user_message_id.to_string(),
                        "variant": self.variant,
                    }),
                    serde_json::json!({
                        "message": user_message_json,
                        "parts": user_parts,
                    }),
                )
                .await;
            if let Some(message) = output
                .get("message")
                .and_then(|m| serde_json::from_value::<UserMessage>(m.clone()).ok())
            {
                user_msg = message;
            }
            if let Some(parts) = output
                .get("parts")
                .and_then(|parts| serde_json::from_value::<Vec<Part>>(parts.clone()).ok())
            {
                user_parts = parts;
            }
        }

        self.store
            .save_message(session_id, &Message::User(user_msg))
            .await?;
        for part in &user_parts {
            self.store.save_part(part).await?;
        }
        let saved_user_parts = user_parts;
        self.bus.publish(Event::message_create(
            session_id.to_string(),
            user_message_id.to_string(),
            MessageRole::User,
        ));

        self.execute_user_subtasks(session_id, &user_message_id, &saved_user_parts, &model_id)
            .await?;

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        for _ in 0..self.max_iterations {
            events.push(ProcessEvent::StepStart);

            // Rebuild the full conversation from persisted state every
            // iteration so each turn sees the canonical view (the model's
            // own prior tool_use calls + our tool_result responses).
            let history =
                crate::session::build_completion_messages(self.store.as_ref(), session_id).await?;

            let request = self
                .build_request_from_history(session_id, &model_id, history)
                .await?;

            let response = match crate::session::complete_with_retry(
                self.provider.as_ref(),
                request,
                crate::session::retry::DEFAULT_MAX_ATTEMPTS,
                std::time::Duration::from_millis(crate::session::retry::DEFAULT_BASE_DELAY_MS),
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    events.push(ProcessEvent::StepFinish {
                        stop_reason: Some("error".to_string()),
                    });
                    events.push(ProcessEvent::Error(e.to_string()));
                    break;
                }
            };

            total_input_tokens += response.usage.input;
            accumulated_content.push_str(&response.content);
            if !response.content.is_empty() {
                events.push(ProcessEvent::TextDelta(response.content.clone()));
            }
            if let Some(reasoning) = response.reasoning.as_deref() {
                if !reasoning.is_empty() {
                    events.push(ProcessEvent::Reasoning(reasoning.to_string()));
                }
            }

            // Persist the assistant turn before running tools so a crash
            // mid-tool leaves a recoverable trace.
            let assistant_message_id = MessageID::new();
            let turn_time = chrono::Utc::now().timestamp_millis();
            let assistant_msg = AssistantMessage {
                id: assistant_message_id.clone(),
                session_id: session_id.clone(),
                role: "assistant".to_string(),
                time: AssistantTime {
                    created: turn_time,
                    completed: Some(turn_time),
                },
                error: None,
                parent_id: user_message_id.to_string(),
                model_id: model_id.clone(),
                provider_id: self.provider.name().to_string(),
                mode: "default".to_string(),
                agent: self.agent_name.clone(),
                path: PathInfo {
                    cwd: cwd.clone(),
                    root: "/".to_string(),
                },
                summary: None,
                cost: 0.0,
                tokens: TokenUsage {
                    input: response.usage.input as f64,
                    output: response.usage.output as f64,
                    reasoning: 0.0,
                    total: None,
                    cache: CacheUsage {
                        read: response.usage.cache_read.unwrap_or(0) as f64,
                        write: response.usage.cache_write.unwrap_or(0) as f64,
                    },
                },
                structured: None,
                variant: self.variant.clone(),
                finish: None,
            };
            self.store
                .save_message(session_id, &Message::Assistant(assistant_msg))
                .await?;
            // Persist any reasoning channel as a ReasoningPart before the
            // text part so history rebuilds keep reasoning ordered ahead of
            // the visible answer.
            if let Some(reasoning) = response.reasoning.as_deref() {
                let reasoning = reasoning.trim();
                if !reasoning.is_empty() {
                    self.store
                        .save_part(&Part::Reasoning(crate::message::part::ReasoningPart {
                            id: crate::id::PartID::new(),
                            session_id: session_id.clone(),
                            message_id: assistant_message_id.clone(),
                            text: reasoning.to_string(),
                            metadata: None,
                            time: crate::message::part::ReasoningTime {
                                start: turn_time,
                                end: Some(turn_time),
                            },
                        }))
                        .await?;
                }
            }
            if !response.content.is_empty() {
                self.store
                    .save_text_part(session_id, &assistant_message_id, &response.content)
                    .await?;
            }
            self.bus.publish(Event::message_create(
                session_id.to_string(),
                assistant_message_id.to_string(),
                MessageRole::Assistant,
            ));

            if response.tool_calls.is_empty() {
                events.push(ProcessEvent::StepFinish {
                    stop_reason: response.stop_reason.clone(),
                });
                events.push(ProcessEvent::Done(accumulated_content));
                break;
            }

            // Execute each tool call and persist a ToolPart on the
            // assistant message so the next history rebuild picks it up.
            self.execute_and_persist_tool_calls(
                session_id,
                &assistant_message_id,
                &response.tool_calls,
                &mut events,
            )
            .await?;
            events.push(ProcessEvent::StepFinish {
                stop_reason: response.stop_reason.clone(),
            });

            // Auto-compact before the next iteration if cumulative input
            // is approaching the model's context ceiling.
            if let Some(model_info) = self.provider.default_model() {
                if crate::session::should_compact(total_input_tokens, model_info) {
                    if let Err(e) = crate::session::compact_session(
                        &self.store,
                        session_id,
                        &self.provider,
                        &model_id,
                    )
                    .await
                    {
                        tracing::warn!("compaction failed (continuing without): {}", e);
                    }
                }
            }
        }

        Ok(events)
    }

    async fn execute_user_subtasks(
        &self,
        session_id: &SessionID,
        user_message_id: &MessageID,
        user_parts: &[Part],
        model_id: &str,
    ) -> anyhow::Result<()> {
        let subtasks = user_parts
            .iter()
            .filter_map(|part| match part {
                Part::Subtask(task) => Some(task.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if subtasks.is_empty() {
            return Ok(());
        }

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let working_dir = std::env::current_dir()?;

        for task in subtasks {
            let assistant_message_id = MessageID::new();
            let turn_time = chrono::Utc::now().timestamp_millis();
            let assistant_msg = AssistantMessage {
                id: assistant_message_id.clone(),
                session_id: session_id.clone(),
                role: "assistant".to_string(),
                time: AssistantTime {
                    created: turn_time,
                    completed: Some(turn_time),
                },
                error: None,
                parent_id: user_message_id.to_string(),
                model_id: task
                    .model
                    .as_ref()
                    .map(|model| model.model_id.clone())
                    .unwrap_or_else(|| model_id.to_string()),
                provider_id: task
                    .model
                    .as_ref()
                    .map(|model| model.provider_id.clone())
                    .unwrap_or_else(|| self.provider.name().to_string()),
                mode: task.agent.clone(),
                agent: task.agent.clone(),
                path: PathInfo {
                    cwd: cwd.clone(),
                    root: "/".to_string(),
                },
                summary: None,
                cost: 0.0,
                tokens: TokenUsage {
                    input: 0.0,
                    output: 0.0,
                    reasoning: 0.0,
                    total: None,
                    cache: CacheUsage {
                        read: 0.0,
                        write: 0.0,
                    },
                },
                structured: None,
                variant: None,
                finish: Some("tool-calls".to_string()),
            };
            self.store
                .save_message(session_id, &Message::Assistant(assistant_msg))
                .await?;

            let call_id = uuid::Uuid::new_v4().to_string();
            let input = serde_json::json!({
                "prompt": task.prompt,
                "description": task.description,
                "subagent_type": task.agent,
                "command": task.command,
            });
            let started = std::time::Instant::now();
            let original_input = input.clone();
            let (input, prehook_outcome) = match self
                .apply_tool_execute_before(session_id, "task", &call_id, input)
                .await
            {
                Ok(input) => (input, None),
                Err(outcome) => (original_input, Some(outcome)),
            };
            self.bus.publish(Event::tool_start(
                session_id.to_string(),
                "task",
                input.clone(),
            ));

            let outcome = if let Some(outcome) = prehook_outcome {
                outcome
            } else {
                let outcome = match self.tools.iter().find(|tool| tool.name() == "task") {
                    Some(tool) => {
                        let ctx = ToolContext {
                            session_id: session_id.clone(),
                            working_dir: working_dir.clone(),
                            permission_rules: self.agent_permission_rules(&self.agent_name),
                            event_bus: Some(self.bus.clone()),
                            permission_broker: self.permission_broker.clone(),
                            provider: Some(self.provider.clone()),
                            store: Some(self.store.clone()),
                            config: self.config.clone(),
                            agent_name: Some(self.agent_name.clone()),
                            model_id: Some(model_id.to_string()),
                            plugin_manager: self.plugin_manager.clone(),
                            question_broker: self.question_broker.clone(),
                            skip_permissions: self.skip_permissions,
                        };
                        match tool.execute(input.clone(), ctx).await {
                            Ok(result) => ToolPartResult::Completed {
                                output: result.output,
                                attachments: self
                                    .normalize_attachments(result.attachments.unwrap_or_default()),
                            },
                            Err(error) => ToolPartResult::Error {
                                error: error.to_string(),
                            },
                        }
                    }
                    None => ToolPartResult::Error {
                        error: "Unknown tool: task".to_string(),
                    },
                };
                self.apply_tool_execute_after(
                    session_id,
                    "task",
                    &call_id,
                    &input,
                    outcome,
                    started.elapsed(),
                )
                .await
            };

            self.store
                .save_tool_part(
                    session_id,
                    &assistant_message_id,
                    "task",
                    &call_id,
                    &input,
                    outcome.clone(),
                )
                .await?;

            match &outcome {
                ToolPartResult::Completed { output, .. } => {
                    self.bus.publish(Event::tool_complete(
                        session_id.to_string(),
                        "task",
                        serde_json::json!({ "result": output }),
                    ));
                }
                ToolPartResult::Error { error } => {
                    self.bus.publish(Event::tool_error(
                        session_id.to_string(),
                        "task",
                        error.clone(),
                    ));
                }
            }
            self.bus.publish(Event::message_create(
                session_id.to_string(),
                assistant_message_id.to_string(),
                MessageRole::Assistant,
            ));

            if task.command.is_some() {
                let summary_message_id = MessageID::new();
                let now = chrono::Utc::now().timestamp_millis();
                let summary_msg = UserMessage {
                    id: summary_message_id.clone(),
                    session_id: session_id.clone(),
                    role: "user".to_string(),
                    time: UserTime { created: now },
                    format: None,
                    summary: None,
                    agent: self.agent_name.clone(),
                    model: ModelRef {
                        provider_id: self.provider.name().to_string(),
                        model_id: model_id.to_string(),
                        variant: None,
                    },
                    system: None,
                    tools: None,
                };
                self.store
                    .save_message(session_id, &Message::User(summary_msg))
                    .await?;
                self.store
                    .save_part(&Part::Text(crate::message::part::TextPart {
                        id: crate::id::PartID::new(),
                        session_id: session_id.clone(),
                        message_id: summary_message_id.clone(),
                        text: "Summarize the task tool output above and continue with your task."
                            .to_string(),
                        synthetic: Some(true),
                        ignored: None,
                        time: None,
                        metadata: None,
                    }))
                    .await?;
                self.bus.publish(Event::message_create(
                    session_id.to_string(),
                    summary_message_id.to_string(),
                    MessageRole::User,
                ));
            }
        }

        Ok(())
    }

    async fn apply_tool_execute_before(
        &self,
        session_id: &SessionID,
        tool_name: &str,
        call_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, ToolPartResult> {
        let Some(plugin_manager) = &self.plugin_manager else {
            return Ok(input);
        };
        let hook_output = plugin_manager
            .trigger_tool_start(crate::plugin::ToolStartInput {
                session_id: session_id.to_string(),
                tool_name: tool_name.to_string(),
                tool_input: input,
                call_id: call_id.to_string(),
            })
            .await
            .map_err(|error| ToolPartResult::Error {
                error: format!("Plugin hook tool.execute.before failed: {error}"),
            })?;
        if !hook_output.approved {
            return Err(ToolPartResult::Error {
                error: format!("Tool '{tool_name}' rejected by plugin hook"),
            });
        }
        let mut tool_input = hook_output
            .modified_input
            .unwrap_or_else(|| serde_json::json!({}));

        // External JS/TS plugins see the same hook with the TS-shaped
        // payload: input `{ tool, sessionID, callID }`, output `{ args }`.
        let bridge_output = plugin_manager
            .trigger_bridge(
                "tool.execute.before",
                serde_json::json!({
                    "tool": tool_name,
                    "sessionID": session_id.to_string(),
                    "callID": call_id,
                }),
                serde_json::json!({ "args": tool_input }),
            )
            .await;
        if let Some(args) = bridge_output.get("args") {
            tool_input = args.clone();
        }
        Ok(tool_input)
    }

    async fn apply_tool_execute_after(
        &self,
        session_id: &SessionID,
        tool_name: &str,
        call_id: &str,
        input: &serde_json::Value,
        outcome: ToolPartResult,
        duration: std::time::Duration,
    ) -> ToolPartResult {
        let Some(plugin_manager) = &self.plugin_manager else {
            return outcome;
        };
        let outcome = match plugin_manager
            .trigger_tool_complete(crate::plugin::ToolCompleteInput {
                session_id: session_id.to_string(),
                tool_name: tool_name.to_string(),
                tool_output: tool_part_result_to_hook_output(&outcome),
                call_id: call_id.to_string(),
                duration_ms: duration.as_millis().try_into().unwrap_or(u64::MAX),
            })
            .await
        {
            Ok(hook_output) => hook_output
                .modified_output
                .map(|value| apply_modified_tool_output(outcome.clone(), value))
                .unwrap_or(outcome),
            Err(error) => {
                return ToolPartResult::Error {
                    error: format!(
                        "Plugin hook tool.execute.after failed for '{tool_name}' with input {}: {error}",
                        input
                    ),
                }
            }
        };

        // External JS/TS plugins see the same hook: input `{ tool, sessionID,
        // callID, args }`, output the tool result payload they may rewrite.
        let bridge_output = plugin_manager
            .trigger_bridge(
                "tool.execute.after",
                serde_json::json!({
                    "tool": tool_name,
                    "sessionID": session_id.to_string(),
                    "callID": call_id,
                    "args": input,
                }),
                tool_part_result_to_hook_output(&outcome),
            )
            .await;
        apply_modified_tool_output(outcome, bridge_output)
    }

    async fn execute_and_persist_tool_calls(
        &self,
        session_id: &SessionID,
        message_id: &MessageID,
        tool_calls: &[crate::provider::ToolCall],
        events: &mut Vec<ProcessEvent>,
    ) -> anyhow::Result<()> {
        let working_dir = std::env::current_dir()?;

        for tool_call in tool_calls {
            let params: serde_json::Value =
                serde_json::from_str(&tool_call.arguments).unwrap_or(serde_json::json!({}));
            let started = std::time::Instant::now();
            let original_params = params.clone();
            let (params, prehook_outcome) = match self
                .apply_tool_execute_before(session_id, &tool_call.name, &tool_call.id, params)
                .await
            {
                Ok(params) => (params, None),
                Err(outcome) => (original_params, Some(outcome)),
            };

            self.bus.publish(Event::tool_start(
                session_id.to_string(),
                tool_call.name.clone(),
                params.clone(),
            ));
            events.push(ProcessEvent::ToolStart(
                tool_call.name.clone(),
                params.clone(),
            ));

            let tool = self.tools.iter().find(|t| t.name() == tool_call.name);

            let outcome = if let Some(outcome) = prehook_outcome {
                outcome
            } else {
                let outcome = match tool {
                    Some(tool) => {
                        let ctx = ToolContext {
                            session_id: session_id.clone(),
                            working_dir: working_dir.clone(),
                            // Use the configured agent rules; unknown agents
                            // keep the previous permissive empty ruleset.
                            permission_rules: self.agent_permission_rules(&self.agent_name),
                            event_bus: Some(self.bus.clone()),
                            permission_broker: self.permission_broker.clone(),
                            provider: Some(self.provider.clone()),
                            store: Some(self.store.clone()),
                            config: self.config.clone(),
                            agent_name: Some(self.agent_name.clone()),
                            model_id: self.model_id.clone(),
                            plugin_manager: self.plugin_manager.clone(),
                            question_broker: self.question_broker.clone(),
                            skip_permissions: self.skip_permissions,
                        };
                        match tool.execute(params.clone(), ctx).await {
                            Ok(tool_result) => ToolPartResult::Completed {
                                output: tool_result.output,
                                attachments: self.normalize_attachments(
                                    tool_result.attachments.unwrap_or_default(),
                                ),
                            },
                            Err(e) => ToolPartResult::Error {
                                error: e.to_string(),
                            },
                        }
                    }
                    None => ToolPartResult::Error {
                        error: format!("Unknown tool: {}", tool_call.name),
                    },
                };
                self.apply_tool_execute_after(
                    session_id,
                    &tool_call.name,
                    &tool_call.id,
                    &params,
                    outcome,
                    started.elapsed(),
                )
                .await
            };

            // Persist BEFORE publishing the complete event so a subscriber
            // racing to read history doesn't miss it.
            if tool_call.name == "todowrite" {
                if let ToolPartResult::Completed { output, .. } = &outcome {
                    if let Ok(todos) = serde_json::from_str::<Vec<crate::tool::TodoItem>>(output) {
                        self.store.replace_todos(session_id, &todos).await?;
                    }
                }
            }
            self.store
                .save_tool_part(
                    session_id,
                    message_id,
                    &tool_call.name,
                    &tool_call.id,
                    &params,
                    outcome.clone(),
                )
                .await?;

            match &outcome {
                ToolPartResult::Completed { output, .. } => {
                    let value = serde_json::json!({ "result": output });
                    self.bus.publish(Event::tool_complete(
                        session_id.to_string(),
                        tool_call.name.clone(),
                        value.clone(),
                    ));
                    events.push(ProcessEvent::ToolComplete(tool_call.name.clone(), value));
                }
                ToolPartResult::Error { error } => {
                    self.bus.publish(Event::tool_error(
                        session_id.to_string(),
                        tool_call.name.clone(),
                        error.clone(),
                    ));
                }
            }
        }
        Ok(())
    }

    async fn build_request_from_history(
        &self,
        session_id: &SessionID,
        model_id: &str,
        messages: Vec<CompletionMessage>,
    ) -> anyhow::Result<CompletionRequest> {
        // Match TS `registry.ts`: GPT-5/o3/o4 family models use the
        // `apply_patch` tool exclusively; everyone else gets the standard
        // `edit`/`write` pair. This keeps Codex-trained models from
        // double-binding two file-editing interfaces.
        let use_patch = is_apply_patch_model(model_id);
        let mut tools: Vec<ToolDefinition> = self
            .tools
            .iter()
            .filter(|t| match t.name() {
                "apply_patch" => use_patch,
                "edit" | "write" => !use_patch,
                _ => true,
            })
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect();

        // External plugins can rewrite each tool's description/parameters
        // before they are sent to the model (TS `tool.definition` hook).
        if let Some(plugin_manager) = &self.plugin_manager {
            for tool in &mut tools {
                let updated = plugin_manager
                    .trigger_bridge(
                        "tool.definition",
                        serde_json::json!({ "toolID": tool.name }),
                        serde_json::json!({
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }),
                    )
                    .await;
                if let Some(description) = updated.get("description").and_then(|v| v.as_str()) {
                    tool.description = description.to_string();
                }
                if let Some(parameters) = updated.get("parameters") {
                    tool.parameters = parameters.clone();
                }
            }
        }

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let agent = self.agent_info(&self.agent_name);
        let system = crate::acp::agent::build_system_prompt_for_agent_info(
            &cwd,
            &self.tools,
            agent.as_ref(),
        );

        let mut request = CompletionRequest {
            model: crate::provider::ModelID::new(model_id),
            messages,
            system: Some(system),
            tools,
            max_tokens: Some(4096),
            temperature: None,
            top_p: None,
            stop_sequences: None,
            extra_headers: std::collections::HashMap::new(),
        };

        // External plugins can tune sampling parameters per turn
        // (TS `chat.params` hook). The input mirrors the TS contract so
        // plugins authored against `@opencode-ai/plugin` see the same
        // shape: provider context, the active user message, and the
        // active variant alongside session/agent/model identifiers.
        if let Some(plugin_manager) = &self.plugin_manager {
            let last_user_message = self
                .last_user_message(session_id)
                .await
                .unwrap_or(serde_json::Value::Null);
            let params = plugin_manager
                .trigger_bridge(
                    "chat.params",
                    serde_json::json!({
                        "sessionID": session_id.to_string(),
                        "agent": self.agent_name,
                        "model": { "providerID": self.provider.name(), "modelID": model_id },
                        "provider": { "id": self.provider.name() },
                        "variant": self.variant,
                        "message": last_user_message,
                    }),
                    serde_json::json!({
                        "temperature": request.temperature,
                        "topP": request.top_p,
                        "topK": serde_json::Value::Null,
                        "maxOutputTokens": request.max_tokens,
                        "options": {},
                    }),
                )
                .await;
            if let Some(temperature) = params.get("temperature").and_then(|v| v.as_f64()) {
                request.temperature = Some(temperature);
            }
            if let Some(top_p) = params.get("topP").and_then(|v| v.as_f64()) {
                request.top_p = Some(top_p);
            }
            if let Some(max_tokens) = params.get("maxOutputTokens").and_then(|v| v.as_u64()) {
                request.max_tokens = Some(max_tokens);
            }
            // `topK` and `options` are accepted by the hook for shape parity
            // even though `CompletionRequest` does not surface them today;
            // ignoring values does not break plugins that always set them.

            // `chat.headers` lets plugins inject auth/beta headers per turn.
            let headers_out = plugin_manager
                .trigger_bridge(
                    "chat.headers",
                    serde_json::json!({
                        "sessionID": session_id.to_string(),
                        "agent": self.agent_name,
                        "model": { "providerID": self.provider.name(), "modelID": model_id },
                        "provider": { "id": self.provider.name() },
                        "variant": self.variant,
                    }),
                    serde_json::json!({ "headers": {} }),
                )
                .await;
            if let Some(map) = headers_out
                .get("headers")
                .and_then(|h| h.as_object())
                .cloned()
            {
                for (k, v) in map {
                    if let Some(text) = v.as_str() {
                        request.extra_headers.insert(k, text.to_string());
                    }
                }
            }
        }

        Ok(request)
    }

    /// Fetch the most recently persisted user message (with parts) and shape
    /// it like the TS `chat.params` hook expects. Returns `None` when the
    /// session has no user message yet (synthetic build turns, mid-stream
    /// rebuilds where history was just reset, etc.).
    async fn last_user_message(&self, session_id: &SessionID) -> Option<serde_json::Value> {
        let messages = self.store.get_messages_with_parts(session_id).await.ok()?;
        let with_parts = messages
            .into_iter()
            .rev()
            .find(|with_parts| matches!(with_parts.info, Message::User(_)))?;
        serde_json::to_value(with_parts).ok()
    }

    fn agent_info(&self, name: &str) -> Option<crate::agent::AgentInfo> {
        crate::agent::resolve_agent(name, self.config.as_ref())
    }

    fn agent_permission_rules(&self, name: &str) -> crate::permission::Ruleset {
        let mut rules = self
            .agent_info(name)
            .map(|agent| agent.permission)
            .unwrap_or_default();
        rules.extend(self.session_permission_rules.clone());
        rules
    }

    /// Resize/recompress image attachments before they enter session history,
    /// so oversized images don't get rejected by the provider. Non-image
    /// attachments pass through; an image that can't be brought under the
    /// configured limits is dropped (mirroring the TypeScript processor).
    fn normalize_attachments(
        &self,
        attachments: Vec<crate::message::part::FilePart>,
    ) -> Vec<crate::message::part::FilePart> {
        attachments
            .into_iter()
            .filter_map(|attachment| {
                if !attachment.mime.starts_with("image/") {
                    return Some(attachment);
                }
                match crate::image::normalize(&attachment, self.config.as_ref()) {
                    Ok(normalized) => Some(normalized),
                    Err(error) => {
                        tracing::warn!(
                            "dropping image attachment {:?}: {}",
                            attachment.filename,
                            error
                        );
                        None
                    }
                }
            })
            .collect()
    }
}

fn text_part(session_id: &SessionID, message_id: &MessageID, text: &str) -> Part {
    Part::Text(crate::message::part::TextPart {
        id: crate::id::PartID::new(),
        session_id: session_id.clone(),
        message_id: message_id.clone(),
        text: text.to_string(),
        synthetic: None,
        ignored: None,
        time: None,
        metadata: None,
    })
}

fn tool_part_result_to_hook_output(outcome: &ToolPartResult) -> serde_json::Value {
    match outcome {
        ToolPartResult::Completed {
            output,
            attachments,
        } => serde_json::json!({
            "output": output,
            "attachments": attachments,
        }),
        ToolPartResult::Error { error } => serde_json::json!({
            "error": error,
        }),
    }
}

fn apply_modified_tool_output(
    outcome: ToolPartResult,
    modified: serde_json::Value,
) -> ToolPartResult {
    match outcome {
        ToolPartResult::Completed {
            mut output,
            mut attachments,
        } => {
            match modified {
                serde_json::Value::String(value) => output = value,
                serde_json::Value::Object(mut object) => {
                    if let Some(error) = object.get("error").and_then(|value| value.as_str()) {
                        return ToolPartResult::Error {
                            error: error.to_string(),
                        };
                    }
                    if let Some(value) = object
                        .remove("output")
                        .or_else(|| object.remove("result"))
                        .and_then(|value| value.as_str().map(ToString::to_string))
                    {
                        output = value;
                    }
                    if let Some(value) = object.remove("attachments") {
                        if let Ok(parsed) = serde_json::from_value(value) {
                            attachments = parsed;
                        }
                    }
                }
                _ => {}
            }
            ToolPartResult::Completed {
                output,
                attachments,
            }
        }
        ToolPartResult::Error { mut error } => {
            match modified {
                serde_json::Value::String(value) => error = value,
                serde_json::Value::Object(object) => {
                    if let Some(value) = object.get("error").and_then(|value| value.as_str()) {
                        error = value.to_string();
                    }
                }
                _ => {}
            }
            ToolPartResult::Error { error }
        }
    }
}

/// Match the TS `registry.ts` heuristic for `apply_patch` exposure: GPT-5/
/// o3/o4 family models prefer `apply_patch`; the older `gpt-4` family and
/// open-weight variants stay on `edit`/`write`.
pub fn is_apply_patch_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    id.contains("gpt-") && !id.contains("oss") && !id.contains("gpt-4")
}

pub fn model_id_from_selection(selection: &str) -> Option<String> {
    let raw = selection.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some((_, model_id)) = raw.split_once('/') {
        let model_id = model_id.trim();
        return (!model_id.is_empty()).then(|| model_id.to_string());
    }

    let lower = raw.to_ascii_lowercase();
    let provider_only = matches!(
        lower.as_str(),
        "alibaba"
            | "anthropic"
            | "azure"
            | "bedrock"
            | "cerebras"
            | "cohere"
            | "copilot"
            | "deepinfra"
            | "deepseek"
            | "fireworks"
            | "gitlab"
            | "google"
            | "groq"
            | "lmstudio"
            | "mistral"
            | "ollama"
            | "openai"
            | "openrouter"
            | "perplexity"
            | "together"
            | "togetherai"
            | "venice"
            | "vercel"
            | "vertex"
            | "xai"
    );
    (!provider_only).then(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{model_id_from_selection, PromptProcessor};
    use crate::id::{MessageID, SessionID};
    use crate::message::{Message, Part};
    use crate::provider::{
        CompletionMessage, CompletionRequest, CompletionResponse, EventStream, ModelID, ModelInfo,
        Provider, ProviderError, ProviderResult, TokenUsage,
    };
    use crate::session::SessionStore;
    use crate::tool::{Tool, ToolContext, ToolResult};

    #[test]
    fn model_id_from_selection_strips_provider_prefix() {
        assert_eq!(
            model_id_from_selection("openai/gpt-4o").as_deref(),
            Some("gpt-4o")
        );
        assert_eq!(
            model_id_from_selection("openrouter/anthropic/claude-3.5-sonnet").as_deref(),
            Some("anthropic/claude-3.5-sonnet")
        );
    }

    #[test]
    fn is_apply_patch_model_matches_ts_heuristic() {
        use super::is_apply_patch_model;
        // GPT-5 family prefers apply_patch.
        assert!(is_apply_patch_model("gpt-5"));
        assert!(is_apply_patch_model("gpt-5-mini"));
        assert!(is_apply_patch_model("gpt-5-nano"));
        // GPT-4 family and open-weights stay on edit/write.
        assert!(!is_apply_patch_model("gpt-4"));
        assert!(!is_apply_patch_model("gpt-4o"));
        assert!(!is_apply_patch_model("gpt-oss-20b"));
        // Non-OpenAI models also stay on edit/write — the TS check keys off
        // the literal "gpt-" prefix, so non-OpenAI ids never match.
        assert!(!is_apply_patch_model("claude-3-5-sonnet-20241022"));
        assert!(!is_apply_patch_model("gemini-2.0-flash"));
        assert!(!is_apply_patch_model("o3"));
    }

    #[test]
    fn model_id_from_selection_ignores_provider_only_values() {
        assert_eq!(model_id_from_selection("openai"), None);
        assert_eq!(model_id_from_selection("anthropic"), None);
        assert_eq!(
            model_id_from_selection("claude-3-5-sonnet"),
            Some("claude-3-5-sonnet".to_string())
        );
    }

    struct FakeProvider {
        model: ModelInfo,
        seen: Arc<Mutex<Vec<CompletionMessage>>>,
    }

    #[async_trait::async_trait]
    impl Provider for FakeProvider {
        fn name(&self) -> &str {
            "test"
        }

        async fn complete(&self, request: CompletionRequest) -> ProviderResult<CompletionResponse> {
            *self.seen.lock().unwrap() = request.messages;
            Ok(CompletionResponse {
                content: "summary".to_string(),
                tool_calls: Vec::new(),
                stop_reason: Some("stop".to_string()),
                usage: TokenUsage {
                    input: 1,
                    output: 1,
                    cache_read: None,
                    cache_write: None,
                },
                model: "test-model".to_string(),
                reasoning: None,
            })
        }

        fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> {
            Ok(Box::pin(futures::stream::empty::<
                Result<crate::provider::StreamEvent, ProviderError>,
            >()))
        }

        fn models(&self) -> &[ModelInfo] {
            std::slice::from_ref(&self.model)
        }

        fn default_model(&self) -> Option<&ModelInfo> {
            Some(&self.model)
        }
    }

    #[tokio::test]
    async fn config_agent_drives_system_prompt_and_permission_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen,
        });
        let config: crate::config::Config = serde_json::from_value(serde_json::json!({
            "agent": {
                "reviewer": {
                    "prompt": "CUSTOM REVIEW PERSONA",
                    "permission": {
                        "bash": "deny",
                        "read": {
                            "*.env": "ask"
                        }
                    }
                }
            }
        }))
        .unwrap();
        let processor = PromptProcessor::new(store, provider)
            .with_config(config)
            .with_agent("reviewer");

        let request = processor
            .build_request_from_history(&SessionID::new(), "test-model", Vec::new())
            .await
            .unwrap();
        assert!(request.system.unwrap().contains("CUSTOM REVIEW PERSONA"));

        let rules = processor.agent_permission_rules("reviewer");
        assert!(rules.iter().any(|rule| {
            rule.permission == "bash"
                && rule.pattern == "*"
                && rule.action == crate::permission::Action::Deny
        }));
        assert!(rules.iter().any(|rule| {
            rule.permission == "read"
                && rule.pattern == "*.env"
                && rule.action == crate::permission::Action::Ask
        }));
    }

    struct FakeTaskTool;

    impl Tool for FakeTaskTool {
        fn name(&self) -> &str {
            "task"
        }

        fn description(&self) -> &str {
            "fake task"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        fn execute(
            &self,
            params: serde_json::Value,
            _ctx: ToolContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = anyhow::Result<ToolResult>> + Send + '_>,
        > {
            Box::pin(async move {
                Ok(ToolResult::with_metadata(
                    format!("task done: {}", params["prompt"].as_str().unwrap_or("")),
                    serde_json::json!({"status": "completed"}),
                ))
            })
        }
    }

    struct ToolHookPlugin;

    #[async_trait::async_trait]
    impl crate::plugin::Plugin for ToolHookPlugin {
        fn meta(&self) -> crate::plugin::PluginMeta {
            crate::plugin::PluginMeta {
                id: "tool-hook-test".to_string(),
                name: "Tool Hook Test".to_string(),
                version: "0.0.0".to_string(),
                description: None,
                author: None,
            }
        }

        async fn initialize(
            &self,
            _config: crate::plugin::PluginConfig,
        ) -> anyhow::Result<crate::plugin::Hooks> {
            Ok(crate::plugin::Hooks {
                on_tool_start: Some(Arc::new(|input: crate::plugin::ToolStartInput| {
                    Box::pin(async move {
                        let mut modified = input.tool_input.clone();
                        modified["prompt"] = serde_json::json!("hooked inspect");
                        Ok(crate::plugin::ToolStartOutput {
                            approved: true,
                            modified_input: Some(modified),
                        })
                    })
                })),
                on_tool_complete: Some(Arc::new(|input: crate::plugin::ToolCompleteInput| {
                    Box::pin(async move {
                        let output = input
                            .tool_output
                            .get("output")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default();
                        Ok(crate::plugin::ToolCompleteOutput {
                            modified_output: Some(serde_json::json!({
                                "output": format!("after {output}"),
                                "attachments": []
                            })),
                        })
                    })
                })),
                ..crate::plugin::Hooks::default()
            })
        }
    }

    #[tokio::test]
    async fn process_stream_executes_subtask_parts_before_provider_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen: seen.clone(),
        });
        let processor =
            PromptProcessor::new(store.clone(), provider).with_tools(vec![Arc::new(FakeTaskTool)]);
        let user_message_id = MessageID::new();
        let user_parts = vec![Part::Subtask(crate::message::part::SubtaskPart {
            id: crate::id::PartID::new(),
            session_id: session_id.clone(),
            message_id: user_message_id,
            prompt: "inspect auth".to_string(),
            description: "Review auth".to_string(),
            agent: "general".to_string(),
            model: None,
            command: Some("review".to_string()),
        })];

        let events = processor
            .process_stream_with_parts(&session_id, "", user_message_id, user_parts)
            .await
            .unwrap();

        assert!(events
            .iter()
            .any(|event| matches!(event, super::ProcessEvent::Done(text) if text == "summary")));

        // Every iteration of the provider loop emits a matched step_start +
        // step_finish pair, in order, before the terminal Done.
        let starts = events
            .iter()
            .filter(|e| matches!(e, super::ProcessEvent::StepStart))
            .count();
        let finishes = events
            .iter()
            .filter(|e| matches!(e, super::ProcessEvent::StepFinish { .. }))
            .count();
        assert!(starts > 0, "expected at least one StepStart event");
        assert_eq!(starts, finishes, "step start/finish events must pair up");

        let messages = store.get_messages_with_parts(&session_id).await.unwrap();
        assert!(messages.iter().any(|message| message
            .parts
            .iter()
            .any(|part| { matches!(part, Part::Tool(tool) if tool.tool == "task") })));
        assert!(messages.iter().any(|message| {
            matches!(message.info, Message::User(_))
                && message.parts.iter().any(|part| {
                    matches!(part, Part::Text(text) if text.synthetic == Some(true)
                        && text.text.contains("Summarize the task tool output"))
                })
        }));

        let seen = seen.lock().unwrap();
        assert!(seen
            .iter()
            .any(|message| message.role == "tool" && message.content.contains("task done")));
    }

    #[tokio::test]
    async fn variant_is_stamped_on_persisted_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen: Arc::new(Mutex::new(Vec::new())),
        });

        let processor = PromptProcessor::new(store.clone(), provider).with_variant("high");
        let user_message_id = MessageID::new();
        processor
            .process_stream_with_parts(&session_id, "hi", user_message_id, Vec::new())
            .await
            .unwrap();

        let messages = store.get_messages_with_parts(&session_id).await.unwrap();
        let user_variant = messages
            .iter()
            .find_map(|with_parts| match &with_parts.info {
                Message::User(user) => Some(user.model.variant.clone()),
                _ => None,
            });
        assert_eq!(user_variant, Some(Some("high".to_string())));

        let assistant_variant = messages
            .iter()
            .find_map(|with_parts| match &with_parts.info {
                Message::Assistant(asst) => Some(asst.variant.clone()),
                _ => None,
            });
        assert_eq!(assistant_variant, Some(Some("high".to_string())));
    }

    /// Provider that always returns a reasoning payload, so we can test
    /// reasoning persistence through the processor without needing a live
    /// thinking model.
    struct ReasoningProvider {
        model: ModelInfo,
    }

    #[async_trait::async_trait]
    impl Provider for ReasoningProvider {
        fn name(&self) -> &str {
            "test"
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> ProviderResult<CompletionResponse> {
            Ok(CompletionResponse {
                content: "answer".to_string(),
                tool_calls: Vec::new(),
                stop_reason: Some("stop".to_string()),
                usage: TokenUsage {
                    input: 1,
                    output: 1,
                    cache_read: None,
                    cache_write: None,
                },
                model: "test-model".to_string(),
                reasoning: Some("step one\nstep two".to_string()),
            })
        }

        fn stream(&self, _request: CompletionRequest) -> ProviderResult<EventStream> {
            Ok(Box::pin(futures::stream::empty::<
                Result<crate::provider::StreamEvent, ProviderError>,
            >()))
        }

        fn models(&self) -> &[ModelInfo] {
            std::slice::from_ref(&self.model)
        }

        fn default_model(&self) -> Option<&ModelInfo> {
            Some(&self.model)
        }
    }

    #[tokio::test]
    async fn reasoning_payload_persists_as_reasoning_part() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let provider = Arc::new(ReasoningProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
        });

        let processor = PromptProcessor::new(store.clone(), provider);
        let user_message_id = MessageID::new();
        processor
            .process_stream_with_parts(&session_id, "hi", user_message_id, Vec::new())
            .await
            .unwrap();

        let messages = store.get_messages_with_parts(&session_id).await.unwrap();
        let reasoning_text = messages.iter().find_map(|with_parts| {
            matches!(&with_parts.info, Message::Assistant(_)).then(|| {
                with_parts.parts.iter().find_map(|part| match part {
                    Part::Reasoning(r) => Some(r.text.clone()),
                    _ => None,
                })
            })?
        });
        assert_eq!(reasoning_text.as_deref(), Some("step one\nstep two"));
    }

    #[tokio::test]
    async fn process_stream_applies_plugin_tool_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen: seen.clone(),
        });
        let mut plugin_manager = crate::plugin::PluginManager::new();
        plugin_manager
            .register(
                Arc::new(ToolHookPlugin),
                crate::plugin::PluginConfig {
                    enabled: true,
                    options: Default::default(),
                },
            )
            .await
            .unwrap();
        let processor = PromptProcessor::new(store.clone(), provider)
            .with_tools(vec![Arc::new(FakeTaskTool)])
            .with_plugin_manager(Arc::new(plugin_manager));
        let user_message_id = MessageID::new();
        let user_parts = vec![Part::Subtask(crate::message::part::SubtaskPart {
            id: crate::id::PartID::new(),
            session_id: session_id.clone(),
            message_id: user_message_id,
            prompt: "inspect auth".to_string(),
            description: "Review auth".to_string(),
            agent: "general".to_string(),
            model: None,
            command: Some("review".to_string()),
        })];

        processor
            .process_stream_with_parts(&session_id, "", user_message_id, user_parts)
            .await
            .unwrap();

        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|message| {
            message.role == "tool" && message.content.contains("after task done: hooked inspect")
        }));
    }

    #[tokio::test]
    async fn build_request_applies_external_plugin_definition_and_params_hooks() {
        use crate::plugin::bridge;
        let Some((runtime, _)) = bridge::detect_js_runtime() else {
            eprintln!("skipping: no JS runtime available");
            return;
        };

        let tmp = tempfile::tempdir().unwrap();
        let plugin_path = tmp.path().join("plugin.mjs");
        std::fs::write(
            &plugin_path,
            r#"export default async function () {
                return {
                    "tool.definition": async (input, output) => {
                        output.description = "rewritten: " + input.toolID
                    },
                    "chat.params": async (input, output) => {
                        output.temperature = 0.42
                        output.maxOutputTokens = 1234
                    },
                }
            }"#,
        )
        .unwrap();

        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen: Arc::new(Mutex::new(Vec::new())),
        });

        let plugin_bridge = bridge::PluginBridge::spawn(
            runtime,
            vec![bridge::PluginToLoad {
                spec: "./plugin.mjs".to_string(),
                entry: format!("file://{}", plugin_path.to_string_lossy()),
                options: None,
            }],
            bridge::PluginInputData {
                directory: "/tmp".to_string(),
                worktree: "/tmp".to_string(),
                project: serde_json::json!({}),
                server_url: "http://localhost:4096".to_string(),
            },
        )
        .await
        .unwrap();
        let plugin_manager = crate::plugin::PluginManager::new();
        plugin_manager.set_bridge(Arc::new(plugin_bridge));

        let processor = PromptProcessor::new(store, provider)
            .with_tools(vec![Arc::new(FakeTaskTool)])
            .with_plugin_manager(Arc::new(plugin_manager));

        let request = processor
            .build_request_from_history(&SessionID::new(), "test-model", Vec::new())
            .await
            .unwrap();

        assert_eq!(request.temperature, Some(0.42));
        assert_eq!(request.max_tokens, Some(1234));
        assert!(request
            .tools
            .iter()
            .any(|tool| tool.description == "rewritten: task"));
    }

    #[tokio::test]
    async fn process_stream_applies_external_plugin_chat_message_hook() {
        use crate::plugin::bridge;
        let Some((runtime, _)) = bridge::detect_js_runtime() else {
            eprintln!("skipping: no JS runtime available");
            return;
        };

        let tmp = tempfile::tempdir().unwrap();
        let plugin_path = tmp.path().join("plugin.mjs");
        std::fs::write(
            &plugin_path,
            r#"export default async function () {
                return {
                    "chat.message": async (input, output) => {
                        for (const part of output.parts) {
                            if (part.type === "text") {
                                part.text = "rewritten: " + part.text
                            }
                        }
                    },
                }
            }"#,
        )
        .unwrap();

        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen: Arc::new(Mutex::new(Vec::new())),
        });

        let plugin_bridge = bridge::PluginBridge::spawn(
            runtime,
            vec![bridge::PluginToLoad {
                spec: "./plugin.mjs".to_string(),
                entry: format!("file://{}", plugin_path.to_string_lossy()),
                options: None,
            }],
            bridge::PluginInputData {
                directory: "/tmp".to_string(),
                worktree: "/tmp".to_string(),
                project: serde_json::json!({}),
                server_url: "http://localhost:4096".to_string(),
            },
        )
        .await
        .unwrap();
        let plugin_manager = crate::plugin::PluginManager::new();
        plugin_manager.set_bridge(Arc::new(plugin_bridge));

        let processor = PromptProcessor::new(store.clone(), provider)
            .with_plugin_manager(Arc::new(plugin_manager));
        let user_message_id = MessageID::new();

        processor
            .process_stream_with_parts(&session_id, "hello world", user_message_id, Vec::new())
            .await
            .unwrap();

        let messages = store.get_messages_with_parts(&session_id).await.unwrap();
        let user_text = messages.iter().find_map(|with_parts| {
            matches!(&with_parts.info, Message::User(_)).then(|| {
                with_parts.parts.iter().find_map(|part| match part {
                    Part::Text(text) => Some(text.text.clone()),
                    _ => None,
                })
            })?
        });
        assert_eq!(user_text.as_deref(), Some("rewritten: hello world"));
    }

    #[tokio::test]
    async fn chat_message_hook_can_rewrite_the_message_body_itself() {
        use crate::plugin::bridge;
        let Some((runtime, _)) = bridge::detect_js_runtime() else {
            eprintln!("skipping: no JS runtime available");
            return;
        };

        let tmp = tempfile::tempdir().unwrap();
        let plugin_path = tmp.path().join("plugin.mjs");
        std::fs::write(
            &plugin_path,
            r#"export default async function () {
                return {
                    "chat.message": async (input, output) => {
                        output.message.agent = "rewritten-agent"
                    },
                }
            }"#,
        )
        .unwrap();

        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen: Arc::new(Mutex::new(Vec::new())),
        });

        let plugin_bridge = bridge::PluginBridge::spawn(
            runtime,
            vec![bridge::PluginToLoad {
                spec: "./plugin.mjs".to_string(),
                entry: format!("file://{}", plugin_path.to_string_lossy()),
                options: None,
            }],
            bridge::PluginInputData {
                directory: "/tmp".to_string(),
                worktree: "/tmp".to_string(),
                project: serde_json::json!({}),
                server_url: "http://localhost:4096".to_string(),
            },
        )
        .await
        .unwrap();
        let plugin_manager = crate::plugin::PluginManager::new();
        plugin_manager.set_bridge(Arc::new(plugin_bridge));

        let processor = PromptProcessor::new(store.clone(), provider)
            .with_plugin_manager(Arc::new(plugin_manager));
        let user_message_id = MessageID::new();

        processor
            .process_stream_with_parts(&session_id, "hi", user_message_id, Vec::new())
            .await
            .unwrap();

        let messages = store.get_messages_with_parts(&session_id).await.unwrap();
        let user_agent = messages
            .iter()
            .find_map(|with_parts| match &with_parts.info {
                Message::User(user) => Some(user.agent.clone()),
                _ => None,
            });
        // The plugin rewriting message.agent should be visible in persisted state.
        assert_eq!(user_agent.as_deref(), Some("rewritten-agent"));
    }

    #[tokio::test]
    async fn process_stream_applies_external_plugin_tool_hooks() {
        use crate::plugin::bridge;
        let Some((runtime, _)) = bridge::detect_js_runtime() else {
            eprintln!("skipping: no JS runtime available");
            return;
        };

        let tmp = tempfile::tempdir().unwrap();
        let plugin_path = tmp.path().join("plugin.mjs");
        std::fs::write(
            &plugin_path,
            r#"export default async function () {
                return {
                    "tool.execute.before": async (input, output) => {
                        output.args.prompt = "bridge inspect"
                    },
                    "tool.execute.after": async (input, output) => {
                        output.output = "bridge after: " + output.output
                    },
                }
            }"#,
        )
        .unwrap();

        let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()).await.unwrap());
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(FakeProvider {
            model: ModelInfo {
                id: Some(ModelID::new("test-model")),
                name: None,
                family: None,
                release_date: None,
                attachment: None,
                reasoning: None,
                temperature: None,
                tool_call: None,
                interleaved: None,
                cost: None,
                limit: None,
                modalities: None,
                experimental: None,
                status: None,
                provider: None,
                options: None,
                headers: None,
                variants: None,
            },
            seen: seen.clone(),
        });

        let plugin_bridge = bridge::PluginBridge::spawn(
            runtime,
            vec![bridge::PluginToLoad {
                spec: "./plugin.mjs".to_string(),
                entry: format!("file://{}", plugin_path.to_string_lossy()),
                options: None,
            }],
            bridge::PluginInputData {
                directory: "/tmp".to_string(),
                worktree: "/tmp".to_string(),
                project: serde_json::json!({}),
                server_url: "http://localhost:4096".to_string(),
            },
        )
        .await
        .unwrap();
        let plugin_manager = crate::plugin::PluginManager::new();
        plugin_manager.set_bridge(Arc::new(plugin_bridge));

        let processor = PromptProcessor::new(store.clone(), provider)
            .with_tools(vec![Arc::new(FakeTaskTool)])
            .with_plugin_manager(Arc::new(plugin_manager));
        let user_message_id = MessageID::new();
        let user_parts = vec![Part::Subtask(crate::message::part::SubtaskPart {
            id: crate::id::PartID::new(),
            session_id: session_id.clone(),
            message_id: user_message_id,
            prompt: "inspect auth".to_string(),
            description: "Review auth".to_string(),
            agent: "general".to_string(),
            model: None,
            command: Some("review".to_string()),
        })];

        processor
            .process_stream_with_parts(&session_id, "", user_message_id, user_parts)
            .await
            .unwrap();

        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|message| {
            message.role == "tool"
                && message
                    .content
                    .contains("bridge after: task done: bridge inspect")
        }));
    }
}
