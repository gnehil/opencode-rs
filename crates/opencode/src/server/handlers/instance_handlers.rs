use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;

pub async fn lsp_status(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "languages": ["rust", "typescript", "python"],
        "status": "available"
    }))
}

pub async fn tool_list(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let tools = [
        "bash", "read", "write", "edit", "glob", "grep", "task",
        "webfetch", "websearch", "lsp_diagnostics", "lsp_goto_definition",
        "lsp_find_references", "lsp_rename", "lsp_symbols"
    ];

    Json(json!({
        "tools": tools.iter().map(|t| json!({
            "name": t,
            "available": true
        })).collect::<Vec<_>>()
    }))
}

pub async fn skill_list(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "skills": [
            {"name": "playwright", "description": "Browser automation via Playwright MCP"},
            {"name": "frontend-ui-ux", "description": "Designer-turned-developer UI/UX"},
            {"name": "git-master", "description": "Git operations"},
            {"name": "review-work", "description": "Post-implementation review orchestrator"}
        ]
    }))
}

pub async fn path_info(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "home": std::env::var("HOME").unwrap_or_default(),
        "cwd": std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "dataDir": std::env::var("OPENCODE_DATA_DIR").unwrap_or_default(),
        "configDir": std::env::var("OPENCODE_CONFIG_DIR").unwrap_or_default()
    }))
}