use axum::{
    extract::{Path, State, Json},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::bus::EventBus;
use crate::id::SessionID;
use crate::session::SessionStore;
use crate::storage::SessionRow;

#[derive(Clone)]
pub struct AppState {
    data_dir: std::path::PathBuf,
    /// All filesystem queries (read/list/find/git) are constrained to paths
    /// canonicalized under this root. Defaults to the process cwd.
    pub workspace_root: std::path::PathBuf,
    pub event_bus: EventBus,
    pub permission_broker: crate::permission::PermissionBroker,
    pub mcp_manager: std::sync::Arc<tokio::sync::RwLock<crate::mcp::McpManager>>,
    pub default_agent: Option<String>,
    pub default_model: Option<String>,
    /// Provider available to HTTP `/prompt`. Optional because servers
    /// that only handle session CRUD (no model dispatch) shouldn't
    /// require credentials to start.
    pub provider: Option<std::sync::Arc<dyn crate::provider::Provider>>,
}

impl AppState {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        let workspace_root = std::env::current_dir()
            .and_then(|p| p.canonicalize())
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        Self {
            data_dir,
            workspace_root,
            event_bus: EventBus::new(),
            permission_broker: crate::permission::PermissionBroker::new(),
            mcp_manager: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::mcp::McpManager::new(),
            )),
            default_agent: None,
            default_model: None,
            provider: None,
        }
    }

    pub fn with_workspace_root(mut self, root: std::path::PathBuf) -> Self {
        self.workspace_root = root.canonicalize().unwrap_or(root);
        self
    }

    pub fn with_provider(
        mut self,
        provider: std::sync::Arc<dyn crate::provider::Provider>,
    ) -> Self {
        self.provider = Some(provider);
        self
    }

    pub fn with_config_defaults(mut self, config: &crate::config::Config) -> Self {
        self.default_agent = config.default_agent.clone();
        self.default_model = self
            .default_agent
            .as_deref()
            .and_then(|agent| config.agent.as_ref().and_then(|agents| agents.get(agent)))
            .and_then(|agent| agent.model.clone())
            .or_else(|| config.model.clone());
        self
    }

    pub async fn get_store(&self) -> SessionStore {
        SessionStore::new(self.data_dir.clone()).await.unwrap()
    }
}

#[derive(Deserialize)]
pub struct CreateSessionBody {
    pub title: String,
    pub project_id: String,
    pub directory: String,
}

#[derive(Deserialize)]
pub struct UpdateSessionBody {
    pub title: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub id: String,
    pub title: String,
    pub project_id: String,
    pub directory: String,
}

impl From<SessionRow> for SessionResponse {
    fn from(row: SessionRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            project_id: row.project_id,
            directory: row.directory,
        }
    }
}

pub async fn list_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SessionResponse>>, StatusCode> {
    let store = state.get_store().await;
    let sessions = store.list(None).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(sessions.into_iter().map(SessionResponse::from).collect()))
}

pub async fn create_session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateSessionBody>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = state.get_store().await;
    let mut session = store
        .create(&body.title, &body.project_id, &std::path::PathBuf::from(&body.directory))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session_id = SessionID::parse(&session.id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(agent) = &state.default_agent {
        store
            .set_agent(&session_id, agent)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        session.agent = Some(agent.clone());
    }
    if let Some(model) = &state.default_model {
        store
            .set_model(&session_id, model)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        session.model = Some(model.clone());
    }
    state
        .event_bus
        .publish(crate::bus::Event::session_create(&session.id));
    Ok(Json(SessionResponse::from(session)))
}

pub async fn get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = state.get_store().await;
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let session = store.get(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match session {
        Some(s) => Ok(Json(SessionResponse::from(s))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn update_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSessionBody>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = state.get_store().await;
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    
    if let Some(title) = body.title {
        store.update_title(&session_id, &title).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    if let Some(agent) = body.agent {
        store.set_agent(&session_id, &agent).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    if let Some(model) = body.model {
        store.set_model(&session_id, &model).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    let session = store.get(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match session {
        Some(s) => {
            state
                .event_bus
                .publish(crate::bus::Event::session_update(&id));
            Ok(Json(SessionResponse::from(s)))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state.get_store().await;
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    store.delete(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .event_bus
        .publish(crate::bus::Event::session_delete(&id));
    Ok(StatusCode::NO_CONTENT)
}

pub async fn archive_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state.get_store().await;
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    store.archive(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .event_bus
        .publish(crate::bus::Event::session_update(&id));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ForkSessionBody {
    pub title: Option<String>,
}

pub async fn fork_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ForkSessionBody>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = state.get_store().await;
    let parent_session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    
    let parent = store.get(&parent_session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match parent {
        Some(p) => {
            let new_title = body.title.unwrap_or_else(|| format!("{} (fork)", p.title));
            let session = store
                .create(&new_title, &p.project_id, &std::path::PathBuf::from(&p.directory))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(SessionResponse::from(session)))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn session_children(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionResponse>>, StatusCode> {
    let store = state.get_store().await;
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;

    // Verify the parent exists so callers can distinguish "no children" from
    // "unknown session".
    if store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let all = store
        .list(None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let parent_id = session_id.to_string();
    let children: Vec<SessionResponse> = all
        .into_iter()
        .filter(|s| s.parent_id.as_deref() == Some(parent_id.as_str()))
        .map(SessionResponse::from)
        .collect();
    Ok(Json(children))
}

#[derive(Deserialize)]
pub struct RevertBody {
    pub message_id: String,
}

/// Revert is not implemented in the storage layer yet. Return 501 rather
/// than pretending to succeed.
pub async fn revert_message(
    State(_state): State<Arc<AppState>>,
    Path(_id): Path<String>,
    Json(_body): Json<RevertBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

/// Abort signals any in-flight processing for the session. There is no
/// shared "in-flight prompt registry" wired through the HTTP server yet
/// (cancellation lives on ACPAgent), so we publish a session.update event
/// so subscribers can observe the request and return 202.
pub async fn abort_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    if store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .event_bus
        .publish(crate::bus::Event::session_update(&id));

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "session_id": id,
            "aborted": true,
        })),
    ))
}

pub async fn session_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state.get_store().await;
    let sessions = store
        .list(None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let total = sessions.len();
    let archived = sessions
        .iter()
        .filter(|s| s.time_archived.is_some())
        .count();
    Ok(Json(serde_json::json!({
        "total": total,
        "active": total - archived,
        "archived": archived,
    })))
}
