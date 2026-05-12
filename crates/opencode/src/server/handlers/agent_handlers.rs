use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::agent::{get_agent, get_default_agent as agent_get_default};

pub async fn list_agents(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let agents: Vec<serde_json::Value> = ["build", "plan", "general", "explore", "scout", "oracle", "librarian"]
        .iter()
        .filter_map(|name| get_agent(name))
        .map(|a| json!({
            "name": a.name,
            "description": a.description,
            "mode": a.mode.to_string(),
            "permissions": a.permission.iter().map(|p| json!({
                "permission": p.permission,
                "pattern": p.pattern,
                "action": p.action.to_string()
            })).collect::<Vec<_>>()
        }))
        .collect();

    Json(json!({ "agents": agents }))
}

pub async fn get_default_agent(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let agent = agent_get_default();
    Json(json!({
        "name": agent.name,
        "description": agent.description,
        "mode": agent.mode.to_string()
    }))
}