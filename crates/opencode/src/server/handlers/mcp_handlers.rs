use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

pub async fn mcp_status(State(_state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!({
        "servers": [],
        "connected": false
    })))
}

pub async fn mcp_list_resources(State(_state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!({
        "resources": []
    })))
}