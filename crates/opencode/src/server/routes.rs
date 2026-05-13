use axum::{
    routing::{get, post, put, delete, patch},
    Router,
};

use crate::server::handlers::{
    session_handlers, message_handlers, event_handlers, config_handlers, 
    file_handlers, mcp_handlers, global_handlers, agent_handlers, 
    instance_handlers, permission_handlers, tui_handlers, workspace_handlers
};
use crate::server::middleware::cors_layer;
use crate::server::handlers::session_handlers::AppState;

pub fn create_router(data_dir: std::path::PathBuf) -> Router {
    let app_state = std::sync::Arc::new(AppState::new(data_dir));

    create_router_with_state(app_state)
}

pub fn create_router_with_state(app_state: std::sync::Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/global/health", get(global_handlers::health))
        .route("/global/config", get(global_handlers::global_config))
        .route("/global/dispose", post(global_handlers::global_dispose))
        .route("/auth/:provider", put(global_handlers::set_auth))
        .route("/auth/:provider", delete(global_handlers::remove_auth))
        .route("/log", post(global_handlers::log_entry))
        .route("/api/session", get(session_handlers::list_sessions))
        .route("/api/session", post(session_handlers::create_session))
        .route("/api/session/status", get(session_handlers::session_status))
        .route("/api/session/:id", get(session_handlers::get_session))
        .route("/api/session/:id", put(session_handlers::update_session))
        .route("/api/session/:id", delete(session_handlers::delete_session))
        .route("/api/session/:id/archive", post(session_handlers::archive_session))
        .route("/api/session/:id/fork", post(session_handlers::fork_session))
        .route("/api/session/:id/children", get(session_handlers::session_children))
        .route("/api/session/:id/revert", post(session_handlers::revert_message))
        .route("/api/session/:id/abort", post(session_handlers::abort_session))
        .route("/api/session/:id/messages", get(message_handlers::list_messages))
        .route("/api/session/:id/prompt", post(message_handlers::prompt))
        // Aliases matching opencode's official server API shape.
        // `/session/:id/message` is the canonical send-and-wait route;
        // `/session/:id/message/list` (GET) returns the persisted
        // history. Same handlers, different paths.
        .route("/session/:id/message", post(message_handlers::prompt))
        .route("/session/:id/message", get(message_handlers::list_messages))
        .route("/event", get(event_handlers::sse_events))
        .route("/config", get(config_handlers::get_config))
        .route("/config", patch(config_handlers::update_config))
        .route("/config/providers", get(config_handlers::list_providers))
        .route("/provider", get(config_handlers::list_providers))
        .route("/file", get(file_handlers::list_files))
        .route("/file/content", get(file_handlers::read_file))
        .route("/file/status", get(file_handlers::git_status))
        .route("/find", get(file_handlers::find_text))
        .route("/find/file", get(file_handlers::list_files))
        .route("/mcp", get(mcp_handlers::mcp_status).post(mcp_handlers::mcp_add))
        .route("/mcp/:name/connect", post(mcp_handlers::mcp_connect))
        .route("/mcp/:name/disconnect", post(mcp_handlers::mcp_disconnect))
        .route("/mcp/resources", get(mcp_handlers::mcp_list_resources))
        .route("/agent", get(agent_handlers::list_agents))
        .route("/agent/default", get(agent_handlers::get_default_agent))
        .route("/lsp", get(instance_handlers::lsp_status))
        .route("/tool", get(instance_handlers::tool_list))
        .route("/skill", get(instance_handlers::skill_list))
        .route("/path", get(instance_handlers::path_info))
        .route("/permission", get(permission_handlers::list_permissions))
        .route("/permission/:request_id/reply", post(permission_handlers::reply_permission))
        .route("/permission/:request_id/reject", post(permission_handlers::reject_permission))
        .route("/question", get(permission_handlers::list_questions))
        .route("/question/:request_id/reply", post(permission_handlers::reply_question))
        .route("/question/:request_id/reject", post(permission_handlers::reject_question))
        .route("/tui/append-prompt", post(tui_handlers::append_prompt))
        .route("/tui/submit-prompt", post(tui_handlers::submit_prompt))
        .route("/tui/clear-prompt", post(tui_handlers::clear_prompt))
        .route("/tui/execute-command", post(tui_handlers::execute_command))
        .route("/tui/show-toast", post(tui_handlers::show_toast))
        .route("/tui/open-help", post(tui_handlers::open_help))
        .route("/tui/open-sessions", post(tui_handlers::open_sessions))
        .route("/tui/open-models", post(tui_handlers::open_models))
        .route("/tui/select-session", post(tui_handlers::select_session))
        .route("/tui/control/next", get(tui_handlers::tui_next))
        .route("/tui/control/response", post(tui_handlers::tui_response))
        .route("/workspace", get(workspace_handlers::list_workspaces))
        .route("/workspace", post(workspace_handlers::create_workspace))
        .route("/workspace/:id", delete(workspace_handlers::remove_workspace))
        .route("/workspace/status", get(workspace_handlers::workspace_status))
        .route("/sync/start", post(workspace_handlers::sync_start))
        .route("/sync/history", get(workspace_handlers::sync_history))
        .route("/sync/replay", post(workspace_handlers::sync_replay))
        .with_state(app_state)
        .layer(cors_layer())
}
