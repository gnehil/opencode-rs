use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct ToolListQuery {
    provider: Option<String>,
    model: Option<String>,
}

pub async fn tool_ids(State(state): State<Arc<AppState>>) -> Json<Vec<String>> {
    let mut ids = effective_tools(&state)
        .into_iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Json(ids)
}

pub async fn tool_list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ToolListQuery>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    if query
        .provider
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
        || query.model.as_deref().unwrap_or_default().trim().is_empty()
    {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut tools = effective_tools(&state)
        .into_iter()
        .map(|tool| {
            serde_json::json!({
                "id": tool.name(),
                "description": tool.description(),
                "parameters": tool.parameters_schema(),
            })
        })
        .collect::<Vec<_>>();
    tools.sort_by(|a, b| {
        a.get("id")
            .and_then(|value| value.as_str())
            .cmp(&b.get("id").and_then(|value| value.as_str()))
    });
    Ok(Json(tools))
}

fn effective_tools(state: &AppState) -> Vec<Arc<dyn crate::tool::Tool>> {
    crate::tool::registry_for_options(crate::tool::RegistryOptions::from_config_and_env(
        state.config.as_ref(),
    ))
}
