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

/// `GET /permission` returns the pending-permission array directly so it
/// matches the TS schema (`Schema.Array(Permission.Request)`) instead of
/// wrapping the array under a `pending` key.
pub async fn list_permissions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PermissionQuery>,
) -> Json<serde_json::Value> {
    let pending = state
        .permission_broker
        .pending(query.session_id.as_deref())
        .await;
    Json(json!(pending))
}

/// Reply payload for `POST /permission/:requestID/reply`. Mirrors TS
/// `ReplyPayload`: `{ reply, message? }`. The legacy `action` field is also
/// accepted for backward compatibility with older Rust clients.
#[derive(Deserialize)]
pub struct PermissionReplyBody {
    #[serde(default)]
    reply: Option<String>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

pub async fn reply_permission(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(body): Json<PermissionReplyBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // `reply` is the TS field name; fall back to legacy `action` so older
    // Rust SDK consumers do not break in one release.
    let raw = body
        .reply
        .as_deref()
        .or(body.action.as_deref())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let reply = match raw {
        "allow" => crate::permission::Reply::Once,
        other => crate::permission::Reply::from_str(other).map_err(|_| StatusCode::BAD_REQUEST)?,
    };

    if !state.permission_broker.reply(&request_id, reply).await {
        return Err(StatusCode::NOT_FOUND);
    }

    // TS responds with a bare boolean; the `_message` consumer is preserved
    // here in case a future workflow wants to log the operator-provided note.
    let _ = body.message;
    Ok(Json(json!(true)))
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

    Ok(Json(json!(true)))
}

/// Payload for the canonical `/session/:sessionID/permissions/:permissionID`
/// route. Matches TS `PermissionResponsePayload`: `{ response: "once" | ... }`.
#[derive(Deserialize)]
pub struct PermissionRespondBody {
    response: String,
}

/// Reply to a permission request via the canonical `/session/.../permissions/...`
/// path. The `sessionID` segment is informational — the broker is keyed by
/// `permissionID`, which is globally unique — so we accept and ignore it for
/// shape parity with the TS server.
pub async fn respond_session_permission(
    State(state): State<Arc<AppState>>,
    Path((_session_id, permission_id)): Path<(String, String)>,
    Json(body): Json<PermissionRespondBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let reply = match body.response.as_str() {
        "allow" => crate::permission::Reply::Once,
        other => crate::permission::Reply::from_str(other).map_err(|_| StatusCode::BAD_REQUEST)?,
    };
    if !state
        .permission_broker
        .reply(&permission_id, reply)
        .await
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(json!(true)))
}

#[derive(Deserialize)]
pub struct QuestionQuery {
    session_id: Option<String>,
}

pub async fn list_questions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<QuestionQuery>,
) -> Json<serde_json::Value> {
    let pending = state
        .question_broker
        .pending(query.session_id.as_deref())
        .await;
    Json(json!(pending))
}

/// Reply payload for `POST /question/:id/reply`. Mirrors the broker's
/// `QuestionReply`: a 2-D array of selected labels, indexed by question.
#[derive(Deserialize)]
pub struct QuestionReplyBody {
    answers: Vec<Vec<String>>,
}

pub async fn reply_question(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
    Json(body): Json<QuestionReplyBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state
        .question_broker
        .reply(&request_id, body.answers)
        .await
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(json!(true)))
}

pub async fn reject_question(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !state.question_broker.reject(&request_id).await {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(json!(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::Service;

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)
    }

    async fn send(app: axum::Router, request: Request<Body>) -> axum::response::Response {
        let mut app = app;
        Service::call(&mut app, request).await.unwrap()
    }

    fn app_state() -> Arc<AppState> {
        let tmp = tempfile::tempdir().unwrap();
        Arc::new(
            AppState::new(tmp.path().join("data"))
                .with_workspace_root(tmp.path().to_path_buf()),
        )
    }

    #[tokio::test]
    async fn list_returns_a_bare_array_matching_ts_schema() {
        let state = app_state();
        let app = crate::server::create_router_with_state(state);

        let response = send(
            app,
            Request::builder()
                .method("GET")
                .uri("/permission")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert!(body.is_array(), "expected array, got {body}");
    }

    #[tokio::test]
    async fn reply_accepts_ts_payload_shape() {
        let state = app_state();
        let app = crate::server::create_router_with_state(state.clone());

        let request = crate::permission::PermissionRequest {
            id: crate::permission::PermissionID::new(),
            session_id: crate::id::SessionID::new(),
            permission: "bash".to_string(),
            patterns: vec!["ls".to_string()],
            metadata: Default::default(),
            always: vec!["ls".to_string()],
            tool: None,
        };
        let request_id = request.id.to_string();
        let waiter = state.permission_broker.register(request).await;

        let response = send(
            app,
            Request::builder()
                .method("POST")
                .uri(format!("/permission/{request_id}/reply"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "reply": "always" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));
        assert!(matches!(
            waiter.await.unwrap(),
            crate::permission::Reply::Always
        ));
    }

    #[tokio::test]
    async fn reply_still_accepts_legacy_action_field() {
        let state = app_state();
        let app = crate::server::create_router_with_state(state.clone());

        let request = crate::permission::PermissionRequest {
            id: crate::permission::PermissionID::new(),
            session_id: crate::id::SessionID::new(),
            permission: "bash".to_string(),
            patterns: vec!["ls".to_string()],
            metadata: Default::default(),
            always: vec!["ls".to_string()],
            tool: None,
        };
        let request_id = request.id.to_string();
        let _waiter = state.permission_broker.register(request).await;

        let response = send(
            app,
            Request::builder()
                .method("POST")
                .uri(format!("/permission/{request_id}/reply"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "action": "once" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));
    }

    #[tokio::test]
    async fn reply_without_reply_or_action_field_is_bad_request() {
        let state = app_state();
        let app = crate::server::create_router_with_state(state);

        let response = send(
            app,
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/permission/{}/reply",
                    crate::permission::PermissionID::new()
                ))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({}).to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
