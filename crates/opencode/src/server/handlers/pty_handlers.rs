use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use serde::Serialize;
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::pty::{PtyCreateInput, PtyID, PtyInfo, PtyStatus, PtyUpdateInput};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyInfoResponse {
    id: String,
    title: String,
    command: String,
    args: Vec<String>,
    cwd: String,
    status: &'static str,
    pid: u32,
}

impl From<PtyInfo> for PtyInfoResponse {
    fn from(info: PtyInfo) -> Self {
        Self {
            id: info.id.0,
            title: info.title,
            command: info.command,
            args: info.args,
            cwd: info.cwd,
            status: match info.status {
                PtyStatus::Running => "running",
                PtyStatus::Exited => "exited",
            },
            pid: info.pid,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellItem {
    path: String,
    name: String,
    acceptable: bool,
}

pub async fn pty_shells() -> Json<Vec<ShellItem>> {
    Json(list_shells())
}

pub async fn pty_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PtyInfoResponse>>, StatusCode> {
    let sessions = state.pty_service.list().await;
    Ok(Json(
        sessions.into_iter().map(PtyInfoResponse::from).collect(),
    ))
}

pub async fn pty_create(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PtyCreateInput>,
) -> Result<Json<PtyInfoResponse>, StatusCode> {
    let info = state
        .pty_service
        .create(body)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(PtyInfoResponse::from(info)))
}

pub async fn pty_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<PtyInfoResponse>, StatusCode> {
    let info = state
        .pty_service
        .get(&PtyID(id))
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(PtyInfoResponse::from(info)))
}

pub async fn pty_update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PtyUpdateInput>,
) -> Result<Json<PtyInfoResponse>, StatusCode> {
    let info = state
        .pty_service
        .update(&PtyID(id), body)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(PtyInfoResponse::from(info)))
}

pub async fn pty_remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<bool>, StatusCode> {
    let pty_id = PtyID(id);
    if state.pty_service.get(&pty_id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    state
        .pty_service
        .remove(&pty_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(true))
}

fn list_shells() -> Vec<ShellItem> {
    let shells = if cfg!(windows) {
        vec![
            std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string()),
            "powershell".to_string(),
            "pwsh".to_string(),
        ]
    } else {
        std::fs::read_to_string("/etc/shells")
            .ok()
            .map(|content| {
                content
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "/bin/zsh".to_string(),
                    "/bin/bash".to_string(),
                    "/bin/sh".to_string(),
                ]
            })
    };

    shells
        .into_iter()
        .filter(|path| shell_exists(path))
        .map(|path| {
            let name = shell_name(&path);
            ShellItem {
                path,
                acceptable: shell_acceptable(&name),
                name,
            }
        })
        .collect()
}

fn shell_exists(path: &str) -> bool {
    if std::path::Path::new(path).is_absolute() {
        return std::path::Path::new(path).is_file();
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(path))
                .find(|candidate| candidate.is_file())
        })
        .is_some()
}

fn shell_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase()
}

fn shell_acceptable(name: &str) -> bool {
    !matches!(name, "fish" | "nu")
}
