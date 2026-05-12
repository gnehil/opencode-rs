use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

pub async fn health(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime": 0
    }))
}

pub async fn global_config(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "dataDir": std::env::var("OPENCODE_DATA_DIR").unwrap_or_default(),
        "providers": ["anthropic", "openai", "azure", "google", "groq", "xai"]
    }))
}

pub async fn global_dispose(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "success": true }))
}

pub async fn set_auth(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!({
        "success": true,
        "provider": body.get("provider").unwrap_or(&json!("unknown"))
    })))
}

pub async fn remove_auth(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!({ "success": true })))
}

pub async fn log_entry(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(json!({
        "logged": true,
        "level": body.get("level").unwrap_or(&json!("info")),
        "message": body.get("message").unwrap_or(&json!(""))
    }))
}