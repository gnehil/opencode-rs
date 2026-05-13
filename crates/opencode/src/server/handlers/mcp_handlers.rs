use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct AddMcpRequest {
    pub name: String,
    pub config: crate::config::McpServerConfig,
}

pub async fn mcp_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let manager = state.mcp_manager.read().await;
    Ok(Json(json!(manager.status())))
}

pub async fn mcp_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AddMcpRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut manager = state.mcp_manager.write().await;
    manager
        .start_server(&body.name, &body.config)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(json!(manager.status())))
}

pub async fn mcp_connect(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>, StatusCode> {
    let mut manager = state.mcp_manager.write().await;
    manager
        .connect_server(&name)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(true))
}

pub async fn mcp_disconnect(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>, StatusCode> {
    let mut manager = state.mcp_manager.write().await;
    manager
        .stop_server(&name)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(true))
}

pub async fn mcp_list_resources(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let manager = state.mcp_manager.read().await;
    let resources = manager.list_all_resources().await;
    Ok(Json(json!(resources)))
}
