use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};

use crate::server::handlers::session_handlers::AppState;
use crate::server::handlers::{
    agent_handlers, config_handlers, event_handlers, file_handlers, global_handlers,
    instance_handlers, mcp_handlers, message_handlers, permission_handlers, pty_handlers,
    session_handlers, tui_handlers, workspace_handlers,
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
        .route("/api/session/:id/todo", get(session_handlers::session_todo))
        .route("/api/session/:id/diff", get(session_handlers::session_diff))
        .route(
            "/api/session/:id/init",
            post(session_handlers::init_session),
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
        .route("/session/:id/todo", get(session_handlers::session_todo))
        .route("/session/:id/diff", get(session_handlers::session_diff))
        .route("/session/:id/init", post(session_handlers::init_session))
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
        .route("/pty/shells", get(pty_handlers::pty_shells))
        .route(
            "/pty",
            get(pty_handlers::pty_list).post(pty_handlers::pty_create),
        )
        .route(
            "/pty/:id",
            get(pty_handlers::pty_get)
                .put(pty_handlers::pty_update)
                .delete(pty_handlers::pty_remove),
        )
        .route(
            "/pty/:id/connect-token",
            post(pty_handlers::pty_connect_token),
        )
        .route("/pty/:id/connect", get(pty_handlers::pty_connect))
        .route(
            "/mcp",
            get(mcp_handlers::mcp_status).post(mcp_handlers::mcp_add),
        )
        .route(
            "/mcp/:name/auth",
            post(mcp_handlers::mcp_auth_start).delete(mcp_handlers::mcp_auth_remove),
        )
        .route(
            "/mcp/:name/auth/callback",
            post(mcp_handlers::mcp_auth_callback),
        )
        .route(
            "/mcp/:name/auth/authenticate",
            post(mcp_handlers::mcp_auth_authenticate),
        )
        .route("/mcp/:name/connect", post(mcp_handlers::mcp_connect))
        .route("/mcp/:name/disconnect", post(mcp_handlers::mcp_disconnect))
        .route("/mcp/resources", get(mcp_handlers::mcp_list_resources))
        .route("/agent", get(agent_handlers::list_agents))
        .route("/agent/default", get(agent_handlers::get_default_agent))
        .route(
            "/instance/dispose",
            post(instance_handlers::instance_dispose),
        )
        .route("/vcs", get(instance_handlers::vcs_info))
        .route("/vcs/status", get(instance_handlers::vcs_status))
        .route("/vcs/diff", get(instance_handlers::vcs_diff))
        .route("/vcs/diff/raw", get(instance_handlers::vcs_diff_raw))
        .route("/vcs/apply", post(instance_handlers::vcs_apply))
        .route("/command", get(instance_handlers::command_list))
        .route("/lsp", get(instance_handlers::lsp_status))
        .route("/tool", get(instance_handlers::tool_list))
        .route("/skill", get(instance_handlers::skill_list))
        .route("/formatter", get(instance_handlers::formatter_status))
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
        .route("/tui/open-themes", post(tui_handlers::open_themes))
        .route("/tui/open-models", post(tui_handlers::open_models))
        .route("/tui/select-session", post(tui_handlers::select_session))
        .route("/tui/publish", post(tui_handlers::publish))
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
        std::fs::write(root.join("image.png"), [137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
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
                .uri("/file/content?path=image.png")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let content = response_json(response).await;
        assert_eq!(content["type"], "text");
        assert_eq!(content["content"], "iVBORw0KGgo=");
        assert_eq!(content["encoding"], "base64");
        assert_eq!(content["mimeType"], "image/png");

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
        let app = create_router_with_state(state.clone());

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
    async fn file_content_includes_git_diff_and_patch_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("tracked.txt"), "before\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("tracked.txt"), "before\nafter\n").unwrap();

        let state = std::sync::Arc::new(
            AppState::new(root.join("data")).with_workspace_root(root.to_path_buf()),
        );
        let app = create_router_with_state(state.clone());

        let response = send(
            app,
            Request::builder()
                .method("GET")
                .uri("/file/content?path=tracked.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let content = response_json(response).await;
        assert_eq!(content["type"], "text");
        assert_eq!(content["content"], "before\nafter");
        assert!(content["diff"].as_str().unwrap().contains("+after"));
        assert_eq!(content["patch"]["oldFileName"], "tracked.txt");
        assert_eq!(content["patch"]["newFileName"], "tracked.txt");
        assert_eq!(content["patch"]["hunks"][0]["oldStart"], 1);
        assert_eq!(content["patch"]["hunks"][0]["oldLines"], 1);
        assert_eq!(content["patch"]["hunks"][0]["newStart"], 1);
        assert_eq!(content["patch"]["hunks"][0]["newLines"], 2);
        assert!(content["patch"]["hunks"][0]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line == "+after"));
    }

    #[tokio::test]
    async fn instance_vcs_and_formatter_routes_match_opencode_httpapi_shapes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let root_canonical = root.canonicalize().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("tracked.txt"), "before\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["branch", "-M", "main"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("tracked.txt"), "before\nafter\n").unwrap();
        std::fs::write(root.join("untracked.txt"), "new\n").unwrap();

        let state = std::sync::Arc::new(
            AppState::new(root.join("data")).with_workspace_root(root.to_path_buf()),
        );
        let app = create_router_with_state(state);

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/path")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let paths = response_json(response).await;
        assert!(paths["home"].as_str().is_some());
        assert!(paths["state"].as_str().is_some());
        assert!(paths["config"].as_str().is_some());
        assert_eq!(paths["worktree"], root_canonical.to_string_lossy().as_ref());
        assert_eq!(
            paths["directory"],
            root_canonical.to_string_lossy().as_ref()
        );

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/instance/dispose")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/vcs")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let vcs = response_json(response).await;
        assert_eq!(vcs["branch"], "main");
        assert_eq!(vcs["default_branch"], "main");

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/vcs/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let status = response_json(response).await;
        assert!(status.as_array().unwrap().iter().any(|item| {
            item["file"] == "tracked.txt"
                && item["status"] == "modified"
                && item["additions"] == 1
                && item["deletions"] == 0
        }));
        assert!(status.as_array().unwrap().iter().any(|item| {
            item["file"] == "untracked.txt"
                && item["status"] == "added"
                && item["additions"] == 1
                && item["deletions"] == 0
        }));

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/vcs/diff?mode=git")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let diff = response_json(response).await;
        assert!(diff.as_array().unwrap().iter().any(|item| {
            item["file"] == "tracked.txt"
                && item["status"] == "modified"
                && item["patch"].as_str().unwrap().contains("+after")
        }));
        assert!(diff.as_array().unwrap().iter().any(|item| {
            item["file"] == "untracked.txt"
                && item["status"] == "added"
                && item["patch"].as_str().unwrap().contains("+new")
        }));

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/vcs/diff/raw")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let raw = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(raw.contains("+after"));
        assert!(raw.contains("+new"));

        let patch = "\
diff --git a/applied.txt b/applied.txt
new file mode 100644
--- /dev/null
+++ b/applied.txt
@@ -0,0 +1 @@
+applied
";
        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/vcs/apply")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "patch": patch }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!({ "applied": true })
        );
        assert_eq!(
            std::fs::read_to_string(root.join("applied.txt")).unwrap(),
            "applied\n"
        );

        let response = send(
            app,
            Request::builder()
                .method("GET")
                .uri("/formatter")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!([]));
    }

    #[tokio::test]
    async fn pty_routes_expose_local_session_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let state = std::sync::Arc::new(
            AppState::new(root.join("data")).with_workspace_root(root.to_path_buf()),
        );
        let app = create_router_with_state(state.clone());

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/pty")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!([]));

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/pty/shells")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response_json(response).await.is_array());

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/pty")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "command": "/bin/sleep",
                        "args": ["1"],
                        "cwd": root,
                        "title": "Test terminal"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let created = response_json(response).await;
        let pty_id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["title"], "Test terminal");
        assert_eq!(created["command"], "/bin/sleep");
        assert_eq!(created["args"], serde_json::json!(["1"]));
        assert_eq!(created["cwd"], root.to_string_lossy().as_ref());
        assert_eq!(created["status"], "running");
        assert!(created["pid"].as_u64().unwrap() > 0);
        assert!(created.get("exit_code").is_none());

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri(format!("/pty/{pty_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["id"], pty_id);

        let response = send(
            app.clone(),
            Request::builder()
                .method("PUT")
                .uri(format!("/pty/{pty_id}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "title": "Updated terminal", "size": { "rows": 30, "cols": 100 } })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["title"], "Updated terminal");

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/pty/{pty_id}/connect-token"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri(format!("/pty/{pty_id}/connect-token"))
                .header("x-opencode-ticket", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let token = response_json(response).await;
        assert!(token["ticket"].as_str().unwrap().len() > 16);
        assert_eq!(token["expires_in"], 60);
        assert!(
            state
                .pty_tickets
                .consume(
                    &crate::pty::PtyID(pty_id.clone()),
                    token["ticket"].as_str().unwrap()
                )
                .await
        );
        assert!(
            !state
                .pty_tickets
                .consume(
                    &crate::pty::PtyID(pty_id.clone()),
                    token["ticket"].as_str().unwrap()
                )
                .await
        );

        let response = send(
            app.clone(),
            Request::builder()
                .method("DELETE")
                .uri(format!("/pty/{pty_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));

        let response = send(
            app,
            Request::builder()
                .method("GET")
                .uri(format!("/pty/{pty_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn tui_routes_publish_events_and_drive_control_queue() {
        let tmp = tempfile::tempdir().unwrap();
        let state = std::sync::Arc::new(
            AppState::new(tmp.path().join("data")).with_workspace_root(tmp.path().to_path_buf()),
        );
        let app = create_router_with_state(state.clone());
        let mut events = state.event_bus.listener();

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/tui/append-prompt")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "text": "hello" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));
        match events.recv().await.unwrap() {
            crate::bus::event::Event::TuiPromptAppend(event) => {
                assert_eq!(event.text, "hello");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/tui/execute-command")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "command": "messages_page_down" }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));
        match events.recv().await.unwrap() {
            crate::bus::event::Event::TuiCommandExecute(event) => {
                assert_eq!(event.command.as_deref(), Some("session.page.down"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/tui/open-themes")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));
        match events.recv().await.unwrap() {
            crate::bus::event::Event::TuiCommandExecute(event) => {
                assert_eq!(event.command.as_deref(), Some("session.list"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/tui/publish")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "type": "tui.toast.show",
                        "properties": { "message": "saved", "variant": "success" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));
        match events.recv().await.unwrap() {
            crate::bus::event::Event::TuiToastShow(event) => {
                assert_eq!(event.message, "saved");
                assert_eq!(event.variant, "success");
                assert_eq!(event.duration, 5000);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        state
            .tui_control
            .submit_request(crate::tui::control::TuiRequest {
                path: "/session".to_string(),
                body: serde_json::json!({ "limit": 1 }),
            })
            .await
            .unwrap();
        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri("/tui/control/next")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!({ "path": "/session", "body": { "limit": 1 } })
        );

        let response = send(
            app,
            Request::builder()
                .method("POST")
                .uri("/tui/control/response")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "accepted": true }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!(true));
        assert_eq!(
            state.tui_control.next_response().await.unwrap(),
            serde_json::json!({ "accepted": true })
        );
    }

    #[tokio::test]
    async fn mcp_auth_routes_match_remote_oauth_flow_shapes() {
        let oauth_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let oauth_addr = oauth_listener.local_addr().unwrap();
        let oauth_base = format!("http://{}", oauth_addr);
        let oauth_app = Router::new()
            .route(
                "/.well-known/oauth-authorization-server",
                get({
                    let oauth_base = oauth_base.clone();
                    move || {
                        let oauth_base = oauth_base.clone();
                        async move {
                            axum::Json(serde_json::json!({
                                "authorization_endpoint": format!("{oauth_base}/authorize"),
                                "token_endpoint": format!("{oauth_base}/token")
                            }))
                        }
                    }
                }),
            )
            .route(
                "/token",
                post(|| async {
                    axum::Json(serde_json::json!({
                        "access_token": "access-123",
                        "token_type": "Bearer",
                        "expires_in": 3600
                    }))
                }),
            );
        let oauth_server = tokio::spawn(async move {
            axum::serve(oauth_listener, oauth_app).await.unwrap();
        });

        let tmp = tempfile::tempdir().unwrap();
        let mut mcp = std::collections::HashMap::new();
        mcp.insert(
            "remote".to_string(),
            crate::config::McpConfigEntry::Full(crate::config::McpServerConfig {
                kind: Some("remote".to_string()),
                url: Some(oauth_base.clone()),
                oauth: Some(crate::config::McpOAuthConfig::Options(
                    crate::config::McpOAuthOptions {
                        client_id: Some("client-123".to_string()),
                        client_secret: None,
                        scope: Some("tools.read".to_string()),
                        redirect_uri: None,
                    },
                )),
                command: None,
                args: None,
                env: None,
                transport: None,
                enabled: None,
                timeout: None,
                headers: None,
            }),
        );
        mcp.insert(
            "no-oauth".to_string(),
            crate::config::McpConfigEntry::Full(crate::config::McpServerConfig {
                kind: Some("remote".to_string()),
                url: Some(oauth_base.clone()),
                oauth: Some(crate::config::McpOAuthConfig::Enabled(false)),
                command: None,
                args: None,
                env: None,
                transport: None,
                enabled: None,
                timeout: None,
                headers: None,
            }),
        );
        let config = crate::config::Config {
            mcp: Some(mcp),
            ..Default::default()
        };
        let state = std::sync::Arc::new(
            AppState::new(tmp.path().join("data"))
                .with_workspace_root(tmp.path().to_path_buf())
                .with_config_defaults(&config),
        );
        let app = create_router_with_state(state.clone());

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/mcp/no-oauth/auth")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/mcp/remote/auth")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let started = response_json(response).await;
        let authorization_url = started["authorizationUrl"].as_str().unwrap();
        let oauth_state = started["oauthState"].as_str().unwrap();
        assert!(authorization_url.starts_with(&format!("{oauth_base}/authorize?")));
        assert!(authorization_url.contains("client_id=client-123"));
        assert!(authorization_url.contains("code_challenge_method=S256"));
        assert!(authorization_url.contains("scope=tools.read"));
        assert!(authorization_url.contains(&format!("state={oauth_state}")));

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/mcp/remote/auth/authenticate")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!({ "status": "needs_auth" })
        );

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/mcp/remote/auth/callback")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "code": "abc" }).to_string()))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            state
                .mcp_auth_store
                .get("remote")
                .await
                .and_then(|entry| entry.tokens)
                .map(|tokens| tokens.access_token),
            Some("access-123".to_string())
        );

        let response = send(
            app,
            Request::builder()
                .method("DELETE")
                .uri("/mcp/remote/auth")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!({ "success": true })
        );
        assert!(state.mcp_auth_store.get("remote").await.is_none());

        oauth_server.abort();
    }

    #[tokio::test]
    async fn canonical_session_local_routes_expose_todo_diff_and_init() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("tracked.txt"), "before\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(root)
            .output()
            .unwrap();
        std::fs::write(root.join("tracked.txt"), "before\nafter\n").unwrap();

        let state = std::sync::Arc::new(
            AppState::new(root.join("data")).with_workspace_root(root.to_path_buf()),
        );
        let app = create_router_with_state(state.clone());

        let response = send(
            app.clone(),
            Request::builder()
                .method("POST")
                .uri("/session")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "directory": root }).to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let created = response_json(response).await;
        let session_id = created["id"].as_str().unwrap();

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri(format!("/session/{session_id}/todo"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await, serde_json::json!([]));

        let parsed_session_id = crate::id::SessionID::parse(session_id).unwrap();
        state
            .get_store()
            .await
            .replace_todos(
                &parsed_session_id,
                &[crate::tool::TodoItem {
                    content: "finish local routes".to_string(),
                    status: "in_progress".to_string(),
                    priority: "high".to_string(),
                }],
            )
            .await
            .unwrap();
        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri(format!("/session/{session_id}/todo"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await,
            serde_json::json!([{
                "content": "finish local routes",
                "status": "in_progress",
                "priority": "high"
            }])
        );

        let response = send(
            app.clone(),
            Request::builder()
                .method("GET")
                .uri(format!("/session/{session_id}/diff"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let diff = response_json(response).await;
        let diff = diff.as_array().expect("/session/:id/diff returns an array");
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0]["file"], "tracked.txt");
        assert_eq!(diff[0]["additions"], 1);
        assert_eq!(diff[0]["deletions"], 0);
        assert_eq!(diff[0]["status"], "modified");
        assert!(diff[0]["patch"].as_str().unwrap().contains("@@"));

        let response = send(
            app,
            Request::builder()
                .method("POST")
                .uri(format!("/session/{session_id}/init"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "providerID": "anthropic",
                        "modelID": "claude-sonnet-4-5",
                        "messageID": "msg_test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
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
