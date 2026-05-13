use axum::{extract::State, Json};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::agent::{get_default_agent as agent_get_default, list_agents as agent_list, AgentInfo};

pub async fn list_agents(State(state): State<Arc<AppState>>) -> Json<Vec<AgentInfo>> {
    Json(agent_list(state.config.as_ref()))
}

pub async fn get_default_agent(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let agent = agent_get_default();
    Json(json!({
        "name": agent.name,
        "description": agent.description,
        "mode": agent.mode.to_string()
    }))
}
