use axum::{
    extract::{State, Path, Json, Query},
    http::StatusCode,
};
use serde::{Deserialize};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct WorkspaceQuery {
    project_id: Option<String>,
}

pub async fn list_workspaces(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<WorkspaceQuery>,
) -> Json<serde_json::Value> {
    Json(json!({
        "workspaces": [],
        "project_id": query.project_id
    }))
}

#[derive(Deserialize)]
pub struct CreateWorkspaceBody {
    name: String,
    project_id: String,
    branch: Option<String>,
}

pub async fn create_workspace(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<CreateWorkspaceBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "name": body.name,
        "project_id": body.project_id,
        "branch": body.branch
    }))
}

pub async fn remove_workspace(
    State(_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "workspace_id": id,
        "removed": true
    }))
}

pub async fn workspace_status(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "connected": false,
        "syncing": false
    }))
}

#[derive(Deserialize)]
pub struct SyncStartBody {
    session_id: Option<String>,
}

pub async fn sync_start(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<SyncStartBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "session_id": body.session_id,
        "sync_started": true
    }))
}

pub async fn sync_history(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({ "events": [] }))
}

#[derive(Deserialize)]
pub struct SyncReplayBody {
    session_id: String,
}

pub async fn sync_replay(
    State(_state): State<Arc<AppState>>,
    Json(body): Json<SyncReplayBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "session_id": body.session_id,
        "replayed": true
    }))
}