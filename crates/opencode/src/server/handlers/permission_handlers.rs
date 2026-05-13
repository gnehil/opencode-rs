use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct PermissionQuery {
    session_id: Option<String>,
}

pub async fn list_permissions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PermissionQuery>,
) -> Json<serde_json::Value> {
    let pending = state
        .permission_broker
        .pending(query.session_id.as_deref())
        .await;
    Json(json!({
        "pending": pending,
        "session_id": query.session_id
    }))
}

#[derive(Deserialize)]
pub struct PermissionReplyBody {
    action: String,
    pattern: Option<String>,
}

pub async fn reply_permission(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(body): Json<PermissionReplyBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let reply = match body.action.as_str() {
        "allow" => crate::permission::Reply::Once,
        other => crate::permission::Reply::from_str(other).map_err(|_| StatusCode::BAD_REQUEST)?,
    };

    if !state.permission_broker.reply(&request_id, reply).await {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(json!({
        "success": true,
        "request_id": request_id,
        "action": body.action,
        "pattern": body.pattern
    })))
}

pub async fn reject_permission(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state
        .permission_broker
        .reply(&request_id, crate::permission::Reply::Reject)
        .await
    {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(json!({
        "success": true,
        "request_id": request_id,
        "rejected": true
    })))
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
