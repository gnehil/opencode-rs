use std::sync::Arc;

use crate::bus::{Event, EventBus, MessageRole};
use crate::id::{MessageID, SessionID};
use crate::message::{AssistantMessage, Message, ModelRef, UserMessage, UserTime};
use crate::message::{AssistantTime, CacheUsage, PathInfo, TokenUsage};
use crate::provider::{CompletionMessage, CompletionRequest, Provider, ToolDefinition};
use crate::session::SessionStore;
use crate::tool::{Tool, ToolContext};

pub struct PromptProcessor {
    store: Arc<SessionStore>,
    provider: Arc<dyn Provider>,
    tools: Vec<Arc<dyn Tool>>,
    bus: EventBus,
    permission_broker: Option<crate::permission::PermissionBroker>,
    max_iterations: usize,
    agent_name: String,
    model_id: Option<String>,
}

pub enum ProcessEvent {
    TextDelta(String),
    ToolStart(String, serde_json::Value),
    ToolComplete(String, serde_json::Value),
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
            max_iterations: 10,
            agent_name: "build".to_string(),
            model_id: None,
        }
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

    pub fn with_tools(mut self, tools: Vec<Arc<dyn Tool>>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
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

        // Persist the user turn exactly once. Subsequent provider calls in
        // the same `process_stream` invocation are tool-result iterations,
        // not new user messages — they replay the persisted history.
        let user_message_id = MessageID::new();
        let now = chrono::Utc::now().timestamp_millis();
        let user_msg = UserMessage {
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
                variant: None,
            },
            system: None,
            tools: None,
        };
        self.store
            .save_message(session_id, &Message::User(user_msg))
            .await?;
        self.store
            .save_text_part(session_id, &user_message_id, prompt)
            .await?;
        self.bus.publish(Event::message_create(
            session_id.to_string(),
            user_message_id.to_string(),
            MessageRole::User,
        ));

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        for _ in 0..self.max_iterations {
            // Rebuild the full conversation from persisted state every
            // iteration so each turn sees the canonical view (the model's
            // own prior tool_use calls + our tool_result responses).
            let history =
                crate::session::build_completion_messages(self.store.as_ref(), session_id).await?;

            let request = self.build_request_from_history(&model_id, history)?;

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
                    events.push(ProcessEvent::Error(e.to_string()));
                    break;
                }
            };

            total_input_tokens += response.usage.input;
            accumulated_content.push_str(&response.content);
            if !response.content.is_empty() {
                events.push(ProcessEvent::TextDelta(response.content.clone()));
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
                variant: None,
                finish: None,
            };
            self.store
                .save_message(session_id, &Message::Assistant(assistant_msg))
                .await?;
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

    async fn execute_and_persist_tool_calls(
        &self,
        session_id: &SessionID,
        message_id: &MessageID,
        tool_calls: &[crate::provider::ToolCall],
        events: &mut Vec<ProcessEvent>,
    ) -> anyhow::Result<()> {
        use crate::session::service::ToolPartResult;

        let working_dir = std::env::current_dir()?;

        for tool_call in tool_calls {
            let params: serde_json::Value =
                serde_json::from_str(&tool_call.arguments).unwrap_or(serde_json::json!({}));

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

            let outcome = match tool {
                Some(tool) => {
                    let ctx = ToolContext {
                        session_id: session_id.clone(),
                        working_dir: working_dir.clone(),
                        // Use the agent's configured permission rules.
                        // If the agent is unknown (no entry in registry)
                        // we fall back to an empty ruleset, which
                        // permits everything — same as before this
                        // commit but tracked explicitly.
                        permission_rules: crate::agent::get_agent(&self.agent_name)
                            .map(|a| a.permission)
                            .unwrap_or_default(),
                        event_bus: Some(self.bus.clone()),
                        permission_broker: self.permission_broker.clone(),
                    };
                    match tool.execute(params.clone(), ctx).await {
                        Ok(tool_result) => ToolPartResult::Completed {
                            output: tool_result.output,
                            attachments: tool_result.attachments.unwrap_or_default(),
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

            // Persist BEFORE publishing the complete event so a subscriber
            // racing to read history doesn't miss it.
            self.store
                .save_tool_part(
                    session_id,
                    message_id,
                    &tool_call.name,
                    &tool_call.id,
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

    fn build_request_from_history(
        &self,
        model_id: &str,
        messages: Vec<CompletionMessage>,
    ) -> anyhow::Result<CompletionRequest> {
        let tools: Vec<ToolDefinition> = self
            .tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect();

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let system =
            crate::acp::agent::build_system_prompt_for_agent(&cwd, &self.tools, &self.agent_name);

        Ok(CompletionRequest {
            model: crate::provider::ModelID::new(model_id),
            messages,
            system: Some(system),
            tools,
            max_tokens: Some(4096),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        })
    }
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
    use super::model_id_from_selection;

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
    fn model_id_from_selection_ignores_provider_only_values() {
        assert_eq!(model_id_from_selection("openai"), None);
        assert_eq!(model_id_from_selection("anthropic"), None);
        assert_eq!(
            model_id_from_selection("claude-3-5-sonnet"),
            Some("claude-3-5-sonnet".to_string())
        );
    }
}
