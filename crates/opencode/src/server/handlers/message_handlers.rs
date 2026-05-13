use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::bus::{Event, MessageRole};
use crate::id::{MessageID, PartID, SessionID};
use crate::message::part::{
    AgentPart, AgentPartSource, FilePart, FilePartSource, SubtaskModel, SubtaskPart, TextPart,
    TextPartTime,
};
use crate::message::{Message, ModelRef, Part, WithParts};

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    #[serde(
        default,
        rename = "messageID",
        alias = "message_id",
        alias = "messageId"
    )]
    pub message_id: Option<String>,
    pub message: Option<String>,
    pub parts: Option<Vec<serde_json::Value>>,
    pub agent: Option<String>,
    pub model: Option<serde_json::Value>,
    pub no_reply: Option<bool>,
}

#[derive(Serialize)]
pub struct PromptResponse {
    pub session_id: String,
    pub message_id: String,
    /// Assistant's reply text. Empty string when the assistant chose
    /// only to invoke tools (the agent loop ran tools internally and
    /// max_iterations was reached without a final text turn).
    pub content: String,
    /// True when the loop produced a final assistant text reply.
    /// False when the prompt was rejected, the provider erred, or
    /// the loop ran to max_iterations without an answer.
    pub completed: bool,
}

struct PromptTurn {
    text: String,
    parts: Vec<UserPartDraft>,
    message_id: Option<String>,
    agent: Option<String>,
    model_selection: Option<String>,
    no_reply: bool,
}

struct PromptOutput {
    legacy: PromptResponse,
    with_parts: Option<WithParts>,
}

#[derive(Clone)]
enum UserPartDraft {
    Text {
        id: Option<PartID>,
        text: String,
        synthetic: Option<bool>,
        ignored: Option<bool>,
        time: Option<TextPartTime>,
        metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
    },
    File {
        id: Option<PartID>,
        mime: String,
        filename: Option<String>,
        url: String,
        source: Option<FilePartSource>,
    },
    Agent {
        id: Option<PartID>,
        name: String,
        source: Option<AgentPartSource>,
    },
    Subtask {
        id: Option<PartID>,
        prompt: String,
        description: String,
        agent: String,
        model: Option<SubtaskModel>,
        command: Option<String>,
    },
}

impl UserPartDraft {
    fn into_part(self, session_id: &SessionID, message_id: &MessageID) -> Part {
        match self {
            UserPartDraft::Text {
                id,
                text,
                synthetic,
                ignored,
                time,
                metadata,
            } => Part::Text(TextPart {
                id: id.unwrap_or_else(PartID::new),
                session_id: session_id.clone(),
                message_id: message_id.clone(),
                text,
                synthetic,
                ignored,
                time,
                metadata,
            }),
            UserPartDraft::File {
                id,
                mime,
                filename,
                url,
                source,
            } => Part::File(FilePart {
                id: id.unwrap_or_else(PartID::new),
                session_id: session_id.clone(),
                message_id: message_id.clone(),
                mime,
                filename,
                url,
                source,
            }),
            UserPartDraft::Agent { id, name, source } => Part::Agent(AgentPart {
                id: id.unwrap_or_else(PartID::new),
                session_id: session_id.clone(),
                message_id: message_id.clone(),
                name,
                source,
            }),
            UserPartDraft::Subtask {
                id,
                prompt,
                description,
                agent,
                model,
                command,
            } => Part::Subtask(SubtaskPart {
                id: id.unwrap_or_else(PartID::new),
                session_id: session_id.clone(),
                message_id: message_id.clone(),
                prompt,
                description,
                agent,
                model,
                command,
            }),
        }
    }

    fn has_content(&self) -> bool {
        match self {
            UserPartDraft::Text { text, .. } => !text.trim().is_empty(),
            UserPartDraft::File { .. }
            | UserPartDraft::Agent { .. }
            | UserPartDraft::Subtask { .. } => true,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRequest {
    #[serde(
        default,
        rename = "messageID",
        alias = "message_id",
        alias = "messageId"
    )]
    pub message_id: Option<String>,
    pub command: String,
    #[serde(default)]
    pub arguments: Option<String>,
    pub agent: Option<String>,
    pub model: Option<serde_json::Value>,
    #[serde(default)]
    pub parts: Option<Vec<serde_json::Value>>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellRequest {
    #[serde(
        default,
        rename = "messageID",
        alias = "message_id",
        alias = "messageId"
    )]
    pub message_id: Option<String>,
    pub command: String,
    pub agent: Option<String>,
    pub model: Option<serde_json::Value>,
}

pub async fn list_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    let with_parts = store
        .get_messages_with_parts(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let payload: Vec<serde_json::Value> = with_parts
        .into_iter()
        .map(|wp| {
            serde_json::json!({
                "info": wp.info,
                "parts": wp.parts,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "messages": payload })))
}

pub async fn get_message(
    State(state): State<Arc<AppState>>,
    Path((id, message_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let message_id = MessageID::parse(&message_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    let message = store
        .get_message_with_parts(&session_id, &message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({
        "info": message.info,
        "parts": message.parts,
    })))
}

pub async fn delete_message(
    State(state): State<Arc<AppState>>,
    Path((id, message_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let message_id = MessageID::parse(&message_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    let deleted = store
        .delete_message(&session_id, &message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(serde_json::json!(true)))
}

pub async fn delete_part(
    State(state): State<Arc<AppState>>,
    Path((id, message_id, part_id)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let message_id = MessageID::parse(&message_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let part_id = PartID::parse(&part_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    let deleted = store
        .delete_part(&session_id, &message_id, &part_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(serde_json::json!(true)))
}

pub async fn update_part(
    State(state): State<Arc<AppState>>,
    Path((id, message_id, part_id)): Path<(String, String, String)>,
    Json(part): Json<Part>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let message_id = MessageID::parse(&message_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let part_id = PartID::parse(&part_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    if part_ids(&part)
        != (
            session_id.to_string(),
            message_id.to_string(),
            part_id.to_string(),
        )
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = state.get_store().await;
    let part = store
        .update_part(&session_id, &message_id, &part_id, &part)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(
        serde_json::to_value(part).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ))
}

fn part_ids(part: &Part) -> (String, String, String) {
    match part {
        Part::Text(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Subtask(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Reasoning(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::File(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Tool(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::StepStart(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::StepFinish(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Snapshot(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Patch(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Agent(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Retry(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
        Part::Compaction(part) => (
            part.session_id.to_string(),
            part.message_id.to_string(),
            part.id.to_string(),
        ),
    }
}

/// Drive a full agent turn for `session_id`:
///   1. Look up the session and its configured agent.
///   2. Build a `PromptProcessor` wired to the AppState's provider and
///      event bus (so streaming events surface on `/event`).
///   3. `process_stream(session_id, prompt)` — persists the user
///      message, runs the multi-turn tool loop, persists the
///      assistant + tool parts. Bounded by the processor's internal
///      max_iterations (10).
///   4. Return the final assistant text + completion flag.
///
/// 503 if AppState has no provider configured (server was started
/// without API credentials).
pub async fn prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PromptRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let turn = prompt_turn_from_request(req)?;
    if !turn.has_content() {
        return Err(StatusCode::BAD_REQUEST);
    }
    run_prompt_turn(state, id, turn)
        .await
        .map(prompt_output_json)
}

pub async fn prompt_async(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PromptRequest>,
) -> Result<StatusCode, StatusCode> {
    let turn = prompt_turn_from_request(req)?;
    if !turn.has_content() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !turn.no_reply && state.provider.is_none() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    if store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    tokio::spawn(async move {
        if let Err(status) = run_prompt_turn(state, id, turn).await {
            tracing::warn!("HTTP prompt_async failed with status {}", status);
        }
    });

    Ok(StatusCode::NO_CONTENT)
}

pub async fn command(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CommandRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let command_name = req.command.trim().trim_start_matches('/');
    if command_name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let commands = crate::command::load_commands(&state.workspace_root, state.config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let command = commands
        .into_iter()
        .find(|command| command.name == command_name)
        .ok_or(StatusCode::BAD_REQUEST)?;

    let arguments = req.arguments.as_deref().unwrap_or_default();
    let text = crate::command::render_template(&command.template, arguments);
    let mut parts = vec![UserPartDraft::Text {
        id: None,
        text: text.clone(),
        synthetic: None,
        ignored: None,
        time: None,
        metadata: None,
    }];
    parts.extend(parse_user_part_drafts(&req.parts)?);

    let turn = PromptTurn {
        text,
        parts,
        message_id: req.message_id,
        agent: command.agent.or(req.agent),
        model_selection: command
            .model
            .or_else(|| model_selection_from_value(req.model.as_ref())),
        no_reply: false,
    };
    run_prompt_turn(state, id, turn)
        .await
        .map(prompt_output_json)
}

pub async fn shell(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ShellRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if req.command.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = std::sync::Arc::new(state.get_store().await);
    let session = store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let agent_name = req
        .agent
        .clone()
        .or_else(|| session.agent.clone())
        .or_else(|| state.default_agent.clone())
        .unwrap_or_else(|| crate::agent::DEFAULT_AGENT_NAME.to_string());
    let model_selection = model_selection_from_value(req.model.as_ref())
        .or_else(|| session.model.clone())
        .or_else(|| state.default_model.clone());
    let model = model_ref_from_selection(model_selection.as_deref(), "shell", "local-shell");
    let user_message_id = optional_message_id(req.message_id.as_deref())?;
    let now = chrono::Utc::now().timestamp_millis();
    let user_message = crate::message::UserMessage {
        id: user_message_id.clone(),
        session_id: session_id.clone(),
        role: "user".to_string(),
        time: crate::message::UserTime { created: now },
        format: None,
        summary: None,
        agent: agent_name.clone(),
        model: model.clone(),
        system: None,
        tools: None,
    };
    store
        .save_message(&session_id, &Message::User(user_message))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    store
        .save_text_part(
            &session_id,
            &user_message_id,
            "The following tool was executed by the user",
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.event_bus.publish(Event::message_create(
        session_id.to_string(),
        user_message_id.to_string(),
        MessageRole::User,
    ));

    let assistant_message_id = MessageID::new();
    let cwd = std::path::PathBuf::from(&session.directory);
    let path = crate::message::PathInfo {
        cwd: cwd.to_string_lossy().to_string(),
        root: state.workspace_root.to_string_lossy().to_string(),
    };
    let started = chrono::Utc::now().timestamp_millis();
    let assistant_message = crate::message::AssistantMessage {
        id: assistant_message_id.clone(),
        session_id: session_id.clone(),
        role: "assistant".to_string(),
        time: crate::message::AssistantTime {
            created: started,
            completed: Some(started),
        },
        error: None,
        parent_id: user_message_id.to_string(),
        model_id: model.model_id.clone(),
        provider_id: model.provider_id.clone(),
        mode: agent_name.clone(),
        agent: agent_name,
        path,
        summary: None,
        cost: 0.0,
        tokens: crate::message::TokenUsage {
            input: 0.0,
            output: 0.0,
            reasoning: 0.0,
            total: None,
            cache: crate::message::CacheUsage {
                read: 0.0,
                write: 0.0,
            },
        },
        structured: None,
        variant: None,
        finish: None,
    };
    store
        .save_message(&session_id, &Message::Assistant(assistant_message))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let input = serde_json::json!({ "command": req.command });
    state.event_bus.publish(Event::tool_start(
        session_id.to_string(),
        "bash",
        input.clone(),
    ));
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let output = tokio::process::Command::new(shell)
        .arg("-lc")
        .arg(&req.command)
        .current_dir(&cwd)
        .output()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        let code = output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        text.push_str(&format!("\n\n<metadata>\nExit code: {code}\n</metadata>"));
    }

    let result = if output.status.success() {
        crate::session::service::ToolPartResult::Completed {
            output: text.clone(),
            attachments: Vec::new(),
        }
    } else {
        crate::session::service::ToolPartResult::Error {
            error: text.clone(),
        }
    };
    store
        .save_tool_part(
            &session_id,
            &assistant_message_id,
            "bash",
            &uuid::Uuid::new_v4().to_string(),
            &input,
            result,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if output.status.success() {
        state.event_bus.publish(Event::tool_complete(
            session_id.to_string(),
            "bash",
            serde_json::json!({ "result": text }),
        ));
    } else {
        state
            .event_bus
            .publish(Event::tool_error(session_id.to_string(), "bash", text));
    }
    state.event_bus.publish(Event::message_create(
        session_id.to_string(),
        assistant_message_id.to_string(),
        MessageRole::Assistant,
    ));

    let with_parts = store
        .get_message_with_parts(&session_id, &assistant_message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        serde_json::to_value(with_parts).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ))
}

async fn run_prompt_turn(
    state: Arc<AppState>,
    id: String,
    turn: PromptTurn,
) -> Result<PromptOutput, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let provider = state
        .provider
        .clone()
        .filter(|_| !turn.no_reply)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE);

    let store = std::sync::Arc::new(state.get_store().await);

    // Verify the session exists before kicking off the LLM.
    let session = store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if turn.no_reply {
        let message_id = optional_message_id(turn.message_id.as_deref())?;
        let model = model_ref_from_selection(
            turn.model_selection
                .as_deref()
                .or(session.model.as_deref())
                .or(state.default_model.as_deref()),
            "local",
            "no-reply",
        );
        let agent_name = turn
            .agent
            .clone()
            .or_else(|| session.agent.clone())
            .or_else(|| state.default_agent.clone())
            .unwrap_or_else(|| crate::agent::DEFAULT_AGENT_NAME.to_string());
        let now = chrono::Utc::now().timestamp_millis();
        let msg = crate::message::UserMessage {
            id: message_id.clone(),
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime { created: now },
            format: None,
            summary: None,
            agent: agent_name,
            model,
            system: None,
            tools: None,
        };
        store
            .save_message(&session_id, &Message::User(msg))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let user_parts = turn.parts_for_message(&session_id, &message_id);
        if user_parts.is_empty() {
            store
                .save_text_part(&session_id, &message_id, &turn.text)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        } else {
            for part in user_parts {
                store
                    .save_part(&part)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            }
        }
        state.event_bus.publish(Event::message_create(
            session_id.to_string(),
            message_id.to_string(),
            MessageRole::User,
        ));
        let with_parts = store
            .get_message_with_parts(&session_id, &message_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(PromptOutput {
            legacy: PromptResponse {
                session_id: id,
                message_id: message_id.to_string(),
                content: String::new(),
                completed: true,
            },
            with_parts,
        });
    }

    let provider = provider?;
    let mcp_tools = {
        let manager = state.mcp_manager.read().await;
        manager.runtime_tools().await
    };

    let agent_name = turn
        .agent
        .clone()
        .or_else(|| session.agent.clone())
        .or_else(|| state.default_agent.clone())
        .unwrap_or_else(|| crate::agent::DEFAULT_AGENT_NAME.to_string());
    let model_selection = turn
        .model_selection
        .clone()
        .or_else(|| session.model.clone())
        .or_else(|| state.default_model.clone());

    let mut processor = crate::session::PromptProcessor::new(store.clone(), provider)
        .with_bus(state.event_bus.clone())
        .with_permission_broker(state.permission_broker.clone())
        .with_tools(crate::tool::registry_with(mcp_tools))
        .with_agent(agent_name);
    if let Some(model) = &model_selection {
        processor = processor.with_model_selection(model);
    }

    let user_message_id = optional_message_id(turn.message_id.as_deref())?;
    let user_parts = turn.parts_for_message(&session_id, &user_message_id);
    let events = processor
        .process_stream_with_parts(&session_id, &turn.text, user_message_id, user_parts)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Recover the user message id (the first persisted message for
    // this prompt) and the final assistant text. The processor
    // persists everything via the store; for the client we need a
    // synthetic summary.
    //
    // We could query the store to find the most recent assistant
    // turn's text, but that's redundant — the processor emitted
    // ProcessEvent::Done with the accumulated text. Walk the events.
    let mut content = String::new();
    let mut completed = false;
    let mut errored = None;
    for ev in &events {
        match ev {
            crate::session::processor::ProcessEvent::Done(text) => {
                content = text.clone();
                completed = true;
            }
            crate::session::processor::ProcessEvent::Error(msg) => {
                errored = Some(msg.clone());
            }
            _ => {}
        }
    }

    if let Some(msg) = errored {
        if !completed {
            // Loop failed before reaching a final assistant turn.
            tracing::warn!("HTTP prompt errored: {}", msg);
        }
    }

    // Best-effort: pull the most recent assistant message as the canonical
    // opencode response payload. The legacy fields below preserve the older
    // Rust HTTP summary shape for existing callers.
    let messages = store
        .get_messages_with_parts(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let response_message = messages
        .iter()
        .rev()
        .find(|message| matches!(message.info, Message::Assistant(_)))
        .cloned();
    let message_id = response_message
        .as_ref()
        .map(|message| message_id_of(&message.info))
        .or_else(|| {
            messages
                .iter()
                .rev()
                .find_map(|message| match &message.info {
                    Message::User(user) => Some(user.id.to_string()),
                    _ => None,
                })
        });
    let message_id = match message_id {
        Some(id) => id,
        None => MessageID::new().to_string(),
    };

    state.event_bus.publish(Event::message_create(
        session_id.to_string(),
        message_id.clone(),
        MessageRole::Assistant,
    ));

    Ok(PromptOutput {
        legacy: PromptResponse {
            session_id: id,
            message_id,
            content,
            completed,
        },
        with_parts: response_message,
    })
}

impl PromptTurn {
    fn has_content(&self) -> bool {
        self.parts.iter().any(UserPartDraft::has_content) || !self.text.trim().is_empty()
    }

    fn parts_for_message(&self, session_id: &SessionID, message_id: &MessageID) -> Vec<Part> {
        self.parts
            .clone()
            .into_iter()
            .map(|part| part.into_part(session_id, message_id))
            .collect()
    }
}

fn prompt_turn_from_request(req: PromptRequest) -> Result<PromptTurn, StatusCode> {
    let text = prompt_request_text(&req);
    let parts = prompt_request_parts(&req)?;
    Ok(PromptTurn {
        text,
        parts,
        message_id: req.message_id,
        agent: req.agent,
        model_selection: model_selection_from_value(req.model.as_ref()),
        no_reply: req.no_reply.unwrap_or(false),
    })
}

fn prompt_output_json(output: PromptOutput) -> Json<serde_json::Value> {
    let mut value = output
        .with_parts
        .and_then(|with_parts| serde_json::to_value(with_parts).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let serde_json::Value::Object(object) = &mut value {
        object.insert(
            "session_id".to_string(),
            serde_json::Value::String(output.legacy.session_id),
        );
        object.insert(
            "message_id".to_string(),
            serde_json::Value::String(output.legacy.message_id),
        );
        object.insert(
            "content".to_string(),
            serde_json::Value::String(output.legacy.content),
        );
        object.insert(
            "completed".to_string(),
            serde_json::Value::Bool(output.legacy.completed),
        );
    }
    Json(value)
}

fn prompt_request_text(req: &PromptRequest) -> String {
    let mut chunks = Vec::new();
    if let Some(message) = req.message.as_deref().filter(|message| !message.is_empty()) {
        chunks.push(message.to_string());
    }
    if let Some(parts) = &req.parts {
        for part in parts {
            if part.get("type").and_then(|value| value.as_str()) == Some("text") {
                if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                    if !text.is_empty() {
                        chunks.push(text.to_string());
                    }
                }
            }
        }
    }
    chunks.join("\n")
}

fn prompt_request_parts(req: &PromptRequest) -> Result<Vec<UserPartDraft>, StatusCode> {
    let mut parts = Vec::new();
    if let Some(message) = req.message.as_deref().filter(|message| !message.is_empty()) {
        parts.push(UserPartDraft::Text {
            id: None,
            text: message.to_string(),
            synthetic: None,
            ignored: None,
            time: None,
            metadata: None,
        });
    }
    parts.extend(parse_user_part_drafts(&req.parts)?);
    Ok(parts)
}

fn parse_user_part_drafts(
    parts: &Option<Vec<serde_json::Value>>,
) -> Result<Vec<UserPartDraft>, StatusCode> {
    let mut parsed = Vec::new();
    let Some(parts) = parts else {
        return Ok(parsed);
    };
    for part in parts {
        let part_type = required_string(part, "type")?;
        let id = optional_part_id(part)?;
        match part_type {
            "text" => parsed.push(UserPartDraft::Text {
                id,
                text: required_string(part, "text")?.to_string(),
                synthetic: optional_field(part, "synthetic")?,
                ignored: optional_field(part, "ignored")?,
                time: optional_field(part, "time")?,
                metadata: optional_field(part, "metadata")?,
            }),
            "file" => parsed.push(UserPartDraft::File {
                id,
                mime: required_string_either(part, "mime", "mediaType")?.to_string(),
                filename: optional_string(part, "filename").map(ToString::to_string),
                url: required_string(part, "url")?.to_string(),
                source: optional_field(part, "source")?,
            }),
            "agent" => parsed.push(UserPartDraft::Agent {
                id,
                name: required_string(part, "name")?.to_string(),
                source: optional_field(part, "source")?,
            }),
            "subtask" => parsed.push(UserPartDraft::Subtask {
                id,
                prompt: required_string(part, "prompt")?.to_string(),
                description: required_string(part, "description")?.to_string(),
                agent: required_string(part, "agent")?.to_string(),
                model: optional_field(part, "model")?,
                command: optional_string(part, "command").map(ToString::to_string),
            }),
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    }
    Ok(parsed)
}

fn optional_part_id(part: &serde_json::Value) -> Result<Option<PartID>, StatusCode> {
    let Some(raw) = optional_string(part, "id") else {
        return Ok(None);
    };
    PartID::parse(raw)
        .map(Some)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn required_string<'a>(part: &'a serde_json::Value, field: &str) -> Result<&'a str, StatusCode> {
    optional_string(part, field).ok_or(StatusCode::BAD_REQUEST)
}

fn required_string_either<'a>(
    part: &'a serde_json::Value,
    first: &str,
    second: &str,
) -> Result<&'a str, StatusCode> {
    optional_string(part, first)
        .or_else(|| optional_string(part, second))
        .ok_or(StatusCode::BAD_REQUEST)
}

fn optional_string<'a>(part: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    part.get(field).and_then(|value| value.as_str())
}

fn optional_field<T: DeserializeOwned>(
    part: &serde_json::Value,
    field: &str,
) -> Result<Option<T>, StatusCode> {
    let Some(value) = part.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value.clone())
        .map(Some)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn optional_message_id(value: Option<&str>) -> Result<MessageID, StatusCode> {
    match value {
        Some(value) => MessageID::parse(value).map_err(|_| StatusCode::BAD_REQUEST),
        None => Ok(MessageID::new()),
    }
}

fn message_id_of(message: &Message) -> String {
    match message {
        Message::User(message) => message.id.to_string(),
        Message::Assistant(message) => message.id.to_string(),
    }
}

fn model_selection_from_value(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
            Some(value.trim().to_string())
        }
        Some(serde_json::Value::Object(value)) => {
            let provider = value
                .get("providerID")
                .or_else(|| value.get("provider_id"))
                .and_then(|value| value.as_str())?;
            let model = value
                .get("modelID")
                .or_else(|| value.get("model_id"))
                .and_then(|value| value.as_str())?;
            Some(format!("{provider}/{model}"))
        }
        _ => None,
    }
}

fn model_ref_from_selection(
    selection: Option<&str>,
    fallback_provider: &str,
    fallback_model: &str,
) -> ModelRef {
    if let Some(selection) = selection.map(str::trim).filter(|value| !value.is_empty()) {
        if let Some((provider, model)) = selection.split_once('/') {
            return ModelRef {
                provider_id: provider.to_string(),
                model_id: model.to_string(),
                variant: None,
            };
        }
        return ModelRef {
            provider_id: fallback_provider.to_string(),
            model_id: selection.to_string(),
            variant: None,
        };
    }
    ModelRef {
        provider_id: fallback_provider.to_string(),
        model_id: fallback_model.to_string(),
        variant: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_request_text_accepts_legacy_message_and_ts_parts() {
        let req = PromptRequest {
            message: Some("legacy".to_string()),
            parts: Some(vec![
                serde_json::json!({ "type": "text", "text": "first" }),
                serde_json::json!({ "type": "file", "url": "file:///tmp/a.txt" }),
                serde_json::json!({ "type": "text", "text": "second" }),
            ]),
            ..Default::default()
        };

        assert_eq!(prompt_request_text(&req), "legacy\nfirst\nsecond");
    }

    #[test]
    fn prompt_request_parts_accepts_file_agent_and_subtask_inputs() {
        let req = PromptRequest {
            message: Some("legacy".to_string()),
            parts: Some(vec![
                serde_json::json!({
                    "type": "file",
                    "mime": "image/png",
                    "filename": "diagram.png",
                    "url": "data:image/png;base64,ABC"
                }),
                serde_json::json!({ "type": "agent", "name": "reviewer" }),
                serde_json::json!({
                    "type": "subtask",
                    "prompt": "inspect auth",
                    "description": "Review auth",
                    "agent": "reviewer",
                    "command": "review"
                }),
            ]),
            ..Default::default()
        };

        let parts = prompt_request_parts(&req).unwrap();
        assert_eq!(parts.len(), 4);

        let session_id = SessionID::new();
        let message_id = MessageID::new();
        let stored = parts
            .into_iter()
            .map(|part| part.into_part(&session_id, &message_id))
            .collect::<Vec<_>>();
        assert!(matches!(stored[0], Part::Text(_)));
        assert!(matches!(stored[1], Part::File(_)));
        assert!(matches!(stored[2], Part::Agent(_)));
        assert!(matches!(stored[3], Part::Subtask(_)));
    }
}
