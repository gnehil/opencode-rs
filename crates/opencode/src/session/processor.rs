use std::sync::Arc;

use crate::id::{SessionID, MessageID};
use crate::message::{Message, UserMessage, AssistantMessage, UserTime, ModelRef};
use crate::message::{AssistantTime, TokenUsage, CacheUsage, PathInfo};
use crate::provider::{CompletionMessage, CompletionRequest, Provider, ToolDefinition};
use crate::session::SessionStore;
use crate::tool::{Tool, ToolContext, BashTool, ReadTool, WriteTool, EditTool, GlobTool, GrepTool};
use crate::bus::{EventBus, Event, MessageRole};

pub struct PromptProcessor {
    store: Arc<SessionStore>,
    provider: Arc<dyn Provider>,
    tools: Vec<Arc<dyn Tool>>,
    bus: EventBus,
    max_iterations: usize,
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
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(BashTool),
            Arc::new(ReadTool),
            Arc::new(WriteTool),
            Arc::new(EditTool),
            Arc::new(GlobTool),
            Arc::new(GrepTool),
        ];
        Self { 
            store, 
            provider, 
            tools,
            bus: EventBus::new(),
            max_iterations: 10,
        }
    }

    pub fn with_bus(mut self, bus: EventBus) -> Self {
        self.bus = bus;
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

    pub async fn process_stream(&self, session_id: &SessionID, prompt: &str) -> anyhow::Result<Vec<ProcessEvent>> {
        let mut events = Vec::new();
        let mut accumulated_content = String::new();
        let mut total_input_tokens: u64 = 0;

        let model_id = self.provider.default_model()
            .and_then(|m| m.id.clone())
            .map(|m| m.to_string())
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
            agent: "build".to_string(),
            model: ModelRef {
                provider_id: self.provider.name().to_string(),
                model_id: model_id.clone(),
                variant: None,
            },
            system: None,
            tools: None,
        };
        self.store.save_message(session_id, &Message::User(user_msg)).await?;
        self.store.save_text_part(session_id, &user_message_id, prompt).await?;
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
            let history = crate::session::build_completion_messages(
                self.store.as_ref(),
                session_id,
            ).await?;

            let request = self.build_request_from_history(&model_id, history)?;

            let response = match crate::session::complete_with_retry(
                self.provider.as_ref(),
                request,
                crate::session::retry::DEFAULT_MAX_ATTEMPTS,
                std::time::Duration::from_millis(crate::session::retry::DEFAULT_BASE_DELAY_MS),
            ).await {
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
                time: AssistantTime { created: turn_time, completed: Some(turn_time) },
                error: None,
                parent_id: user_message_id.to_string(),
                model_id: model_id.clone(),
                provider_id: self.provider.name().to_string(),
                mode: "default".to_string(),
                agent: "build".to_string(),
                path: PathInfo { cwd: cwd.clone(), root: "/".to_string() },
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
            self.store.save_message(session_id, &Message::Assistant(assistant_msg)).await?;
            if !response.content.is_empty() {
                self.store.save_text_part(session_id, &assistant_message_id, &response.content).await?;
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
            ).await?;

            // Auto-compact before the next iteration if cumulative input
            // is approaching the model's context ceiling.
            if let Some(model_info) = self.provider.default_model() {
                if crate::session::should_compact(total_input_tokens, model_info) {
                    if let Err(e) = crate::session::compact_session(
                        &self.store,
                        session_id,
                        &self.provider,
                        &model_id,
                    ).await {
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
            events.push(ProcessEvent::ToolStart(tool_call.name.clone(), params.clone()));

            let tool = self.tools.iter().find(|t| t.name() == tool_call.name);

            let outcome = match tool {
                Some(tool) => {
                    let ctx = ToolContext {
                        session_id: session_id.clone(),
                        working_dir: working_dir.clone(),
                        permission_rules: crate::permission::Ruleset::default(),
                    };
                    match tool.execute(params.clone(), ctx).await {
                        Ok(tool_result) => ToolPartResult::Completed {
                            output: tool_result.output,
                            attachments: tool_result.attachments.unwrap_or_default(),
                        },
                        Err(e) => ToolPartResult::Error { error: e.to_string() },
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
                        ToolPartResult::Completed { output, attachments } => {
                            ToolPartResult::Completed {
                                output: output.clone(),
                                attachments: attachments.clone(),
                            }
                        }
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
        let tools: Vec<ToolDefinition> = self.tools.iter().map(|t| ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters_schema(),
        }).collect();

        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let system = crate::acp::agent::build_system_prompt(&cwd, &self.tools);

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
