use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::bus::{Event, MessageRole};
use crate::id::{MessageID, SessionID};

#[derive(Deserialize)]
pub struct PromptRequest {
    pub message: String,
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
) -> Result<Json<PromptResponse>, StatusCode> {
    if req.message.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let provider = state
        .provider
        .clone()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let store = std::sync::Arc::new(state.get_store().await);

    // Verify the session exists before kicking off the LLM.
    let session = store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let mcp_tools = {
        let manager = state.mcp_manager.read().await;
        manager.runtime_tools().await
    };

    let agent_name = session
        .agent
        .clone()
        .or_else(|| state.default_agent.clone())
        .unwrap_or_else(|| crate::agent::DEFAULT_AGENT_NAME.to_string());
    let model_selection = session
        .model
        .clone()
        .or_else(|| state.default_model.clone());

    let mut processor = crate::session::PromptProcessor::new(store.clone(), provider)
        .with_bus(state.event_bus.clone())
        .with_permission_broker(state.permission_broker.clone())
        .with_tools(crate::tool::registry_with(mcp_tools))
        .with_agent(agent_name);
    if let Some(model) = &model_selection {
        processor = processor.with_model_selection(model);
    }

    let events = processor
        .process_stream(&session_id, &req.message)
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

    // Best-effort: pull the most recent user message id from the
    // store as a correlation handle. (Processor doesn't surface it
    // directly.)
    let message_id = match store.get_messages(&session_id).await.ok().and_then(|msgs| {
        msgs.into_iter().rev().find_map(|m| match m {
            crate::message::Message::User(u) => Some(u.id.to_string()),
            _ => None,
        })
    }) {
        Some(id) => id,
        None => MessageID::new().to_string(),
    };

    state.event_bus.publish(Event::message_create(
        session_id.to_string(),
        message_id.clone(),
        MessageRole::Assistant,
    ));

    Ok(Json(PromptResponse {
        session_id: id,
        message_id,
        content,
        completed,
    }))
}
