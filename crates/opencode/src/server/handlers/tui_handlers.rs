use axum::{
    extract::{State, Json},
    http::StatusCode,
};
use serde::{Deserialize};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct TuiPromptBody {
    prompt: String,
    session_id: Option<String>,
}

pub async fn append_prompt(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<TuiPromptBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "prompt": body.prompt,
        "session_id": body.session_id
    }))
}

pub async fn submit_prompt(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<TuiPromptBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "submitted": true,
        "prompt": body.prompt,
        "session_id": body.session_id
    }))
}

pub async fn clear_prompt(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "success": true, "cleared": true }))
}

#[derive(Deserialize)]
pub struct TuiCommandBody {
    command: String,
}

pub async fn execute_command(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<TuiCommandBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "command": body.command,
        "executed": true
    }))
}

#[derive(Deserialize)]
pub struct TuiToastBody {
    message: String,
    duration: Option<u64>,
}

pub async fn show_toast(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<TuiToastBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "message": body.message,
        "duration": body.duration
    }))
}

pub async fn open_help(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "success": true, "help_open": true }))
}

pub async fn open_sessions(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "success": true, "sessions_open": true }))
}

pub async fn open_models(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "success": true, "models_open": true }))
}

#[derive(Deserialize)]
pub struct TuiSessionBody {
    session_id: String,
}

pub async fn select_session(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<TuiSessionBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "session_id": body.session_id,
        "selected": true
    }))
}

pub async fn tui_next(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "request": null }))
}

#[derive(Deserialize)]
pub struct TuiResponseBody {
    response: String,
}

pub async fn tui_response(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<TuiResponseBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "response": body.response
    }))
}