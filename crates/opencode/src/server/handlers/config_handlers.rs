use axum::{extract::State, http::StatusCode, Json};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

pub async fn get_config(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!({
        "provider": "anthropic",
        "model": "claude-3-5-sonnet-20241022",
        "agent": "build",
        "experimental": {},
        "permissions": []
    })))
}

pub async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let old_value = state
        .config
        .as_ref()
        .and_then(|config| serde_json::to_value(config).ok())
        .unwrap_or(serde_json::Value::Null);
    if let Err(error) = state
        .plugin_manager
        .trigger_config_change(crate::plugin::ConfigChangeInput {
            config_type: "project".to_string(),
            old_value,
            new_value: body.clone(),
        })
        .await
    {
        tracing::warn!("plugin config hook failed: {}", error);
    }
    Ok(Json(json!({
        "success": true,
        "config": body
    })))
}

pub async fn list_providers(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!({
        "providers": [
            {"id": "anthropic", "name": "Anthropic", "models": ["claude-3-5-sonnet-20241022", "claude-3-5-haiku-20241022"]},
            {"id": "openai", "name": "OpenAI", "models": ["gpt-4o", "gpt-4o-mini"]},
            {"id": "azure", "name": "Azure OpenAI", "models": []},
            {"id": "google", "name": "Google AI", "models": ["gemini-2.0-flash"]},
            {"id": "groq", "name": "Groq", "models": ["llama-3.3-70b-versatile"]},
            {"id": "xai", "name": "xAI", "models": ["grok-2"]}
        ]
    })))
}
