use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    Json,
};
use serde::{Deserialize};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct PermissionQuery {
    session_id: Option<String>,
}

pub async fn list_permissions(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<PermissionQuery>,
) -> Json<serde_json::Value> {
    Json(json!({
        "pending": [],
        "session_id": query.session_id
    }))
}

#[derive(Deserialize)]
pub struct PermissionReplyBody {
    action: String,
    pattern: Option<String>,
}

pub async fn reply_permission(
    State(_state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(body): Json<PermissionReplyBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "request_id": request_id,
        "action": body.action,
        "pattern": body.pattern
    }))
}

pub async fn reject_permission(
    State(_state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "request_id": request_id,
        "rejected": true
    }))
}

#[derive(Deserialize)]
pub struct QuestionQuery {
    session_id: Option<String>,
}

pub async fn list_questions(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<QuestionQuery>,
) -> Json<serde_json::Value> {
    Json(json!({
        "questions": [],
        "session_id": query.session_id
    }))
}

#[derive(Deserialize)]
pub struct QuestionReplyBody {
    answer: String,
}

pub async fn reply_question(
    State(_state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(body): Json<QuestionReplyBody>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "request_id": request_id,
        "answer": body.answer
    }))
}

pub async fn reject_question(
    State(_state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "request_id": request_id,
        "rejected": true
    }))
}