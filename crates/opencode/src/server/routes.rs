use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};

use crate::server::handlers::session_handlers::AppState;
use crate::server::handlers::{
    agent_handlers, config_handlers, event_handlers, file_handlers, global_handlers,
    instance_handlers, mcp_handlers, message_handlers, permission_handlers, session_handlers,
    tui_handlers, workspace_handlers,
};
use crate::server::middleware::cors_layer;

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
        .route(
            "/api/session/:id/archive",
            post(session_handlers::archive_session),
        )
        .route(
            "/api/session/:id/fork",
            post(session_handlers::fork_session),
        )
        .route(
            "/api/session/:id/children",
            get(session_handlers::session_children),
        )
        .route(
            "/api/session/:id/revert",
            post(session_handlers::revert_message),
        )
        .route(
            "/api/session/:id/unrevert",
            post(session_handlers::unrevert_session),
        )
        .route(
            "/api/session/:id/abort",
            post(session_handlers::abort_session),
        )
        .route(
            "/api/session/:id/messages",
            get(message_handlers::list_messages),
        )
        .route("/api/session/:id/prompt", post(message_handlers::prompt))
        .route(
            "/api/session/:id/prompt_async",
            post(message_handlers::prompt_async),
        )
        .route("/api/session/:id/command", post(message_handlers::command))
        .route("/api/session/:id/shell", post(message_handlers::shell))
        // Aliases matching opencode's official server API shape.
        // `/session/:id/message` is the canonical send-and-wait route;
        // GET on the same path returns the persisted history.
        .route(
            "/session",
            get(session_handlers::list_sessions).post(session_handlers::create_session),
        )
        .route("/session/status", get(session_handlers::session_status))
        .route(
            "/session/:id",
            get(session_handlers::get_session)
                .patch(session_handlers::update_session)
                .delete(session_handlers::delete_session),
        )
        .route("/session/:id/fork", post(session_handlers::fork_session))
        .route(
            "/session/:id/children",
            get(session_handlers::session_children),
        )
        .route(
            "/session/:id/revert",
            post(session_handlers::revert_message),
        )
        .route(
            "/session/:id/unrevert",
            post(session_handlers::unrevert_session),
        )
        .route("/session/:id/abort", post(session_handlers::abort_session))
        .route("/session/:id/message", post(message_handlers::prompt))
        .route("/session/:id/message", get(message_handlers::list_messages))
        .route(
            "/session/:id/prompt_async",
            post(message_handlers::prompt_async),
        )
        .route("/session/:id/command", post(message_handlers::command))
        .route("/session/:id/shell", post(message_handlers::shell))
        .route(
            "/session/:id/message/:message_id",
            get(message_handlers::get_message).delete(message_handlers::delete_message),
        )
        .route(
            "/session/:id/message/:message_id/part/:part_id",
            delete(message_handlers::delete_part).patch(message_handlers::update_part),
        )
        .route("/event", get(event_handlers::sse_events))
        .route("/config", get(config_handlers::get_config))
        .route("/config", patch(config_handlers::update_config))
        .route("/config/providers", get(config_handlers::list_providers))
        .route("/provider", get(config_handlers::list_providers))
        .route("/file", get(file_handlers::list_files))
        .route("/file/content", get(file_handlers::read_file))
        .route("/file/status", get(file_handlers::git_status))
        .route("/find", get(file_handlers::find_text))
        .route("/find/file", get(file_handlers::find_file))
        .route("/find/symbol", get(file_handlers::find_symbol))
        .route(
            "/mcp",
            get(mcp_handlers::mcp_status).post(mcp_handlers::mcp_add),
        )
        .route("/mcp/:name/connect", post(mcp_handlers::mcp_connect))
        .route("/mcp/:name/disconnect", post(mcp_handlers::mcp_disconnect))
        .route("/mcp/resources", get(mcp_handlers::mcp_list_resources))
        .route("/agent", get(agent_handlers::list_agents))
        .route("/agent/default", get(agent_handlers::get_default_agent))
        .route("/command", get(instance_handlers::command_list))
        .route("/lsp", get(instance_handlers::lsp_status))
        .route("/tool", get(instance_handlers::tool_list))
        .route("/skill", get(instance_handlers::skill_list))
        .route("/path", get(instance_handlers::path_info))
        .route("/permission", get(permission_handlers::list_permissions))
        .route(
            "/permission/:request_id/reply",
            post(permission_handlers::reply_permission),
        )
        .route(
            "/permission/:request_id/reject",
            post(permission_handlers::reject_permission),
        )
        .route("/question", get(permission_handlers::list_questions))
        .route(
            "/question/:request_id/reply",
            post(permission_handlers::reply_question),
        )
        .route(
            "/question/:request_id/reject",
            post(permission_handlers::reject_question),
        )
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
        .route(
            "/workspace/:id",
            delete(workspace_handlers::remove_workspace),
        )
        .route(
            "/workspace/status",
            get(workspace_handlers::workspace_status),
        )
        .route("/sync/start", post(workspace_handlers::sync_start))
        .route("/sync/history", get(workspace_handlers::sync_history))
        .route("/sync/replay", post(workspace_handlers::sync_replay))
        .with_state(app_state)
        .layer(cors_layer())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::Service;

    async fn response_json(response: axum::response::Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn send(app: Router, request: Request<Body>) -> axum::response::Response {
        let mut app = app;
        Service::call(&mut app, request).await.unwrap()
    }

    #[tokio::test]
    async fn canonical_file_routes_match_opencode_httpapi_shapes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let root_canonical = root.canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("README.md"), "hello readme\n").unwrap();
        std::fs::write(
            root.join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "secret-token\n").unwrap();

        let state = std::sync::Arc::new(
            AppState::new(root.join("data")).with_workspace_root(root.to_path_buf()),
        );
        let app = create_router_with_state(state);

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/file?path=.")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let files = response_json(response).await;
        let files = files.as_array().expect("/file returns a bare array");
        let src = files.iter().find(|item| item["name"] == "src").unwrap();
        assert_eq!(src["path"], "src");
        assert_eq!(
            src["absolute"],
            root_canonical.join("src").to_string_lossy().as_ref()
        );
        assert_eq!(src["type"], "directory");
        assert_eq!(src["ignored"], false);
        let ignored = files
            .iter()
            .find(|item| item["name"] == "ignored.txt")
            .unwrap();
        assert_eq!(ignored["ignored"], true);

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/file/content?path=README.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let content = response_json(response).await;
        assert_eq!(content["type"], "text");
        assert_eq!(content["content"], "hello readme");
        assert!(content.get("path").is_none());

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/file/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response_json(response).await.is_array());

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/find?pattern=hello")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let matches = response_json(response).await;
        let matches = matches.as_array().expect("/find returns a bare array");
        assert!(matches.iter().any(|item| {
            item["path"]["text"] == "README.md"
                && item["lines"]["text"].as_str().unwrap().contains("hello")
                && item["line_number"] == 1
                && item["submatches"][0]["match"]["text"] == "hello"
        }));

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/find?pattern=secret-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!([]));

        let response = send(
            app,
            Request::builder()
                .method("GET")
                .uri("/find/symbol?query=main")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!([]));
    }

    #[tokio::test]
    async fn find_file_honors_query_type_limit_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src/components")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        std::fs::write(root.join("docs/app.md"), "# app\n").unwrap();
        std::fs::write(root.join("docs/guide.md"), "# guide\n").unwrap();
        std::fs::write(root.join(".gitignore"), "docs/app.md\n").unwrap();

        let state = std::sync::Arc::new(
            AppState::new(root.join("data")).with_workspace_root(root.to_path_buf()),
        );
        let app = create_router_with_state(state);

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/find/file?query=main&type=file&limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!(["src/main.rs"])
        );

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/find/file?query=src&type=directory&limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!(["src/", "src/components/"])
        );

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/find/file?query=&limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!(["docs/", "src/", "src/components/"])
        );

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/find/file?query=app&type=file&limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!([]));

        let response = send(
            app,
            Request::builder()
                .method("GET")
                .uri("/find/file?query=&dirs=false&limit=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let files = response_json(response).await;
        assert_eq!(files.as_array().unwrap().len(), 2);
        assert!(files
            .as_array()
            .unwrap()
            .iter()
            .all(|item| !item.as_str().unwrap().ends_with('/')));
    }

    #[tokio::test]
    async fn canonical_session_routes_accept_local_message_flows() {
        let tmp = tempfile::tempdir().unwrap();
        let state = std::sync::Arc::new(
            AppState::new(tmp.path().join("data")).with_workspace_root(tmp.path().to_path_buf()),
        );
        let app = create_router_with_state(state);

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/command")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let commands = response_json(response).await;
        assert!(commands
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["name"] == "init"));

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/session")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let created = response_json(response).await;
        let session_id = created["id"].as_str().unwrap();

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/session/{session_id}/message"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "noReply": true,
                        "parts": [{ "type": "text", "text": "hello" }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let prompt = response_json(response).await;
        assert_eq!(prompt["completed"], true);
        assert_eq!(prompt["parts"][0]["text"], "hello");

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/session/{session_id}/prompt_async"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "noReply": true,
                        "parts": [{ "type": "text", "text": "async" }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/session/{session_id}/shell"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "command": "printf ok" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let shell = response_json(response).await;
        assert_eq!(shell["info"]["role"], "assistant");
        assert_eq!(shell["parts"][0]["tool"], "bash");
        assert_eq!(shell["parts"][0]["state"]["output"], "ok");

        let response = send(
            app,
            Request::builder()
                .method("POST")
                .uri(format!("/session/{session_id}/unrevert"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
