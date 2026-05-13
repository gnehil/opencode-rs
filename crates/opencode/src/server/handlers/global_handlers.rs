use axum::{
    extract::{Path, State},
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
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let info = parse_auth_info(body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = crate::auth::AuthStore::new(state.data_dir());
    store
        .load()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    store
        .set(provider.trim(), info)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({
        "success": true,
        "provider": provider
    })))
}

pub async fn remove_auth(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = crate::auth::AuthStore::new(state.data_dir());
    store
        .load()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    store
        .remove(provider.trim())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({ "success": true, "provider": provider })))
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

fn parse_auth_info(body: serde_json::Value) -> anyhow::Result<crate::auth::AuthInfo> {
    if body.get("type").is_some() {
        return Ok(serde_json::from_value(body)?);
    }
    if let Some(key) = body.get("key").and_then(|value| value.as_str()) {
        if let Some(token) = body.get("token").and_then(|value| value.as_str()) {
            return Ok(crate::auth::AuthInfo::wellknown(key, token));
        }
        return Ok(crate::auth::AuthInfo::api(key));
    }
    let access = body
        .get("access")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow::anyhow!("auth body must include type or key"))?;
    let refresh = body
        .get("refresh")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    Ok(crate::auth::AuthInfo::Oauth {
        refresh: refresh.to_string(),
        access: access.to_string(),
        expires: body.get("expires").and_then(|value| value.as_i64()),
        extra: std::collections::HashMap::new(),
    })
}
