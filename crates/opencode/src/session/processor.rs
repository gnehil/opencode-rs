use std::sync::Arc;
use std::collections::HashMap;

use crate::id::{SessionID, MessageID, PartID};
use crate::message::{Message, UserMessage, AssistantMessage, UserTime, ModelRef};
use crate::message::{AssistantTime, TokenUsage, CacheUsage, PathInfo};
use crate::message::part::TextPart;
use crate::provider::{Provider, CompletionRequest, ToolDefinition};
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
        let mut iteration = 0;
        let mut current_prompt = prompt.to_string();
        let mut accumulated_content = String::new();

        while iteration < self.max_iterations {
            iteration += 1;
            
            let user_message_id = MessageID::new();
            let now = chrono::Utc::now().timestamp_millis();
            let model_id = self.provider.default_model()
                .and_then(|m| m.id.clone())
                .map(|m| m.to_string())
                .unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string());

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

            self.save_message(session_id, &Message::User(user_msg)).await?;
            self.save_text_part(session_id, &user_message_id, &current_prompt, now).await?;

            self.bus.publish(Event::message_create(
                session_id.to_string(),
                user_message_id.to_string(),
                MessageRole::User,
            ));

            let request = self.build_request(&model_id)?;
            
            let response = match self.provider.complete(request).await {
                Ok(r) => r,
                Err(e) => {
                    events.push(ProcessEvent::Error(e.to_string()));
                    break;
                }
            };

            accumulated_content.push_str(&response.content);
            events.push(ProcessEvent::TextDelta(response.content.clone()));

            self.bus.publish(Event::message_stream(
                session_id.to_string(),
                MessageID::new().to_string(),
                &response.content,
            ));

            let assistant_message_id = MessageID::new();
            let assistant_msg = AssistantMessage {
                id: assistant_message_id.clone(),
                session_id: session_id.clone(),
                role: "assistant".to_string(),
                time: AssistantTime {
                    created: now,
                    completed: Some(now),
                },
                error: None,
                parent_id: user_message_id.to_string(),
                model_id: model_id,
                provider_id: self.provider.name().to_string(),
                mode: "default".to_string(),
                agent: "build".to_string(),
                path: PathInfo {
                    cwd: std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
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

            self.save_message(session_id, &Message::Assistant(assistant_msg)).await?;
            self.save_text_part(session_id, &assistant_message_id, &response.content, now).await?;

            self.bus.publish(Event::message_create(
                session_id.to_string(),
                assistant_message_id.to_string(),
                MessageRole::Assistant,
            ));

            if response.tool_calls.is_empty() {
                events.push(ProcessEvent::Done(accumulated_content));
                break;
            }

            let tool_results = self.execute_tool_calls_with_events(session_id, &response.tool_calls, &mut events).await?;
            
            current_prompt = self.format_tool_results(&tool_results);
        }

        Ok(events)
    }

    async fn execute_tool_calls_with_events(
        &self, 
        session_id: &SessionID, 
        tool_calls: &[crate::provider::ToolCall],
        events: &mut Vec<ProcessEvent>
    ) -> anyhow::Result<HashMap<String, serde_json::Value>> {
        let working_dir = std::env::current_dir()?;
        let mut results = HashMap::new();
        
        for tool_call in tool_calls {
            let tool = self.tools.iter().find(|t| t.name() == tool_call.name);
            
            if let Some(tool) = tool {
                let params: serde_json::Value = match serde_json::from_str(&tool_call.arguments) {
                    Ok(p) => p,
                    Err(_) => serde_json::json!({}),
                };

                self.bus.publish(Event::tool_start(
                    session_id.to_string(),
                    tool_call.name.clone(),
                    params.clone(),
                ));
                events.push(ProcessEvent::ToolStart(tool_call.name.clone(), params.clone()));

                let ctx = ToolContext {
                    session_id: session_id.clone(),
                    working_dir: working_dir.clone(),
                    permission_rules: crate::permission::Ruleset::default(),
                };

                let result = tool.execute(params.clone(), ctx).await;
                
                match result {
                    Ok(tool_result) => {
                        let output = serde_json::to_string(&tool_result)?;
                        let output_value = serde_json::json!({ "result": output });
                        results.insert(tool_call.id.clone(), output_value.clone());
                        
                        self.bus.publish(Event::tool_complete(
                            session_id.to_string(),
                            tool_call.name.clone(),
                            output_value.clone(),
                        ));
                        events.push(ProcessEvent::ToolComplete(tool_call.name.clone(), output_value));
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        results.insert(tool_call.id.clone(), serde_json::json!({ "error": error_msg }));
                        
                        self.bus.publish(Event::tool_error(
                            session_id.to_string(),
                            tool_call.name.clone(),
                            error_msg.clone(),
                        ));
                    }
                }
            } else {
                let error_msg = format!("Unknown tool: {}", tool_call.name);
                results.insert(tool_call.id.clone(), serde_json::json!({ "error": error_msg }));
                
                self.bus.publish(Event::tool_error(
                    session_id.to_string(),
                    tool_call.name.clone(),
                    error_msg,
                ));
            }
        }

        Ok(results)
    }

    fn format_tool_results(&self, results: &HashMap<String, serde_json::Value>) -> String {
        let mut formatted = String::new();
        for (id, result) in results {
            formatted.push_str(&format!("Tool result {}:\n{}\n\n", id, serde_json::to_string(result).unwrap_or_default()));
        }
        formatted
    }

    async fn save_message(&self, session_id: &SessionID, message: &Message) -> anyhow::Result<()> {
        let data = serde_json::to_string(message)?;
        let now = chrono::Utc::now().timestamp_millis();
        let message_id_str = match message {
            Message::User(u) => u.id.to_string(),
            Message::Assistant(a) => a.id.to_string(),
        };

        sqlx::query(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)"
        )
        .bind(&message_id_str)
        .bind(session_id.to_string())
        .bind(now)
        .bind(now)
        .bind(&data)
        .execute(self.store.pool.as_ref())
        .await?;

        Ok(())
    }

    async fn save_text_part(&self, session_id: &SessionID, message_id: &MessageID, text: &str, time: i64) -> anyhow::Result<()> {
        let part_id = PartID::new();
        let part_data = serde_json::to_string(&TextPart {
            id: part_id.clone(),
            session_id: session_id.clone(),
            message_id: message_id.clone(),
            text: text.to_string(),
            synthetic: None,
            ignored: None,
            time: None,
            metadata: None,
        })?;

        sqlx::query(
            "INSERT INTO part (id, session_id, message_id, time_created, data) VALUES (?1, ?2, ?3, ?4, ?5)"
        )
        .bind(part_id.to_string())
        .bind(session_id.to_string())
        .bind(message_id.to_string())
        .bind(time)
        .bind(&part_data)
        .execute(self.store.pool.as_ref())
        .await?;

        Ok(())
    }

    fn build_request(&self, model_id: &str) -> anyhow::Result<CompletionRequest> {
        let tools: Vec<ToolDefinition> = self.tools.iter().map(|t| ToolDefinition {
            name: t.name().to_string(),
            description: t.description().to_string(),
            parameters: t.parameters_schema(),
        }).collect();

        let user_msg = Message::User(UserMessage {
            id: MessageID::new(),
            session_id: SessionID::new(),
            role: "user".to_string(),
            time: UserTime { created: chrono::Utc::now().timestamp_millis() },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: ModelRef {
                provider_id: self.provider.name().to_string(),
                model_id: model_id.to_string(),
                variant: None,
            },
            system: None,
            tools: None,
        });

        Ok(CompletionRequest {
            model: crate::provider::ModelID::new(model_id),
            messages: vec![user_msg],
            system: None,
            tools,
            max_tokens: Some(4096),
            temperature: None,
            top_p: None,
            stop_sequences: None,
        })
    }
}