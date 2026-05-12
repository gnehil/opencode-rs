use axum::{
    extract::{Path, State, Json},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::id::SessionID;
use crate::session::SessionStore;
use crate::storage::SessionRow;

#[derive(Clone)]
pub struct AppState {
    data_dir: std::path::PathBuf,
}

impl AppState {
    pub fn new(data_dir: std::path::PathBuf) -> Self {
        Self { data_dir }
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
    let session = store
        .create(&body.title, &body.project_id, &std::path::PathBuf::from(&body.directory))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        Some(s) => Ok(Json(SessionResponse::from(s))),
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
    Ok(StatusCode::NO_CONTENT)
}

pub async fn archive_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state.get_store().await;
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    store.archive(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
    Ok(Json(vec![]))
}

#[derive(Deserialize)]
pub struct RevertBody {
    pub message_id: String,
}

pub async fn revert_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<RevertBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({
        "success": true,
        "session_id": id,
        "reverted_message_id": body.message_id
    })))
}

pub async fn abort_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({
        "success": true,
        "session_id": id,
        "aborted": true
    })))
}

pub async fn session_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(serde_json::json!({
        "active": 0,
        "processing": 0,
        "waiting": 0
    })))
}

#[derive(Deserialize)]
pub struct UpdateSessionRequest {
    title: Option<String>,
}

#[derive(Serialize)]
pub struct SessionResponse {
    id: String,
    title: String,
    directory: String,
    created: i64,
}

#[derive(Clone)]
pub struct AppState {
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
    
    pub async fn get_store(&self) -> anyhow::Result<Arc<SessionStore>> {
        let store = SessionStore::new(self.data_dir.clone()).await?;
        Ok(Arc::new(store))
    }
}

pub async fn list_sessions(
    State(app_state): State<Arc<AppState>>,
) -> Result<Json<Vec<SessionResponse>>, StatusCode> {
    let store = app_state.get_store().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let sessions = store.list(None).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let response: Vec<SessionResponse> = sessions.iter().map(|s| SessionResponse {
        id: s.id.clone(),
        title: s.title.clone(),
        directory: s.directory.clone(),
        created: s.time_created,
    }).collect();
    
    Ok(Json(response))
}

pub async fn create_session(
    State(app_state): State<Arc<AppState>>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = app_state.get_store().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let cwd = std::env::current_dir().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session = store.create(&req.title, "default", &cwd).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(Json(SessionResponse {
        id: session.id,
        title: session.title,
        directory: session.directory,
        created: session.time_created,
    }))
}

pub async fn get_session(
    State(app_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = app_state.get_store().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session_id = crate::id::SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let session = store.get(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    match session {
        Some(s) => Ok(Json(SessionResponse {
            id: s.id,
            title: s.title,
            directory: s.directory,
            created: s.time_created,
        })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn update_session(
    State(app_state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<SessionResponse>, StatusCode> {
    let store = app_state.get_store().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session_id = crate::id::SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    
    if let Some(title) = req.title {
        store.update_title(&session_id, &title).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    
    let session = store.get(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    match session {
        Some(s) => Ok(Json(SessionResponse {
            id: s.id,
            title: s.title,
            directory: s.directory,
            created: s.time_created,
        })),
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn delete_session(
    State(app_state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = app_state.get_store().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let session_id = crate::id::SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    store.delete(&session_id).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    Ok(StatusCode::NO_CONTENT)
}