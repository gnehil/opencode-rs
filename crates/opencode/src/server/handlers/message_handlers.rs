use axum::{
    extract::{Path, State, Json},
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
    pub accepted: bool,
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

/// Accept a user prompt and persist it as a User message. The actual LLM
/// turn-taking happens in the ACP path / session processor; this HTTP route
/// is the "ingest" boundary. We persist + emit a `message.create` event and
/// return the new message id so the client can subscribe to /event for the
/// assistant reply.
pub async fn prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, StatusCode> {
    if req.message.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    let session = store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let message_id = MessageID::new();
    let now = chrono::Utc::now().timestamp_millis();

    let user_msg = crate::message::UserMessage {
        id: message_id.clone(),
        session_id: session_id.clone(),
        role: "user".to_string(),
        time: crate::message::UserTime { created: now },
        format: None,
        summary: None,
        agent: session.agent.clone().unwrap_or_else(|| "build".to_string()),
        model: crate::message::ModelRef {
            provider_id: String::new(),
            model_id: session.model.clone().unwrap_or_default(),
            variant: None,
        },
        system: None,
        tools: None,
    };

    store
        .save_message(&session_id, &crate::message::Message::User(user_msg))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    store
        .save_text_part(&session_id, &message_id, &req.message)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    state.event_bus.publish(Event::message_create(
        session_id.to_string(),
        message_id.to_string(),
        MessageRole::User,
    ));

    Ok(Json(PromptResponse {
        session_id: id,
        message_id: message_id.to_string(),
        accepted: true,
    }))
}
