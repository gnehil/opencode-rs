use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Json, Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

use super::session_handlers::AppState;
use crate::pty::{PtyConnectToken, PtyCreateInput, PtyID, PtyInfo, PtyStatus, PtyUpdateInput};

const PTY_CONNECT_TICKET_HEADER: &str = "x-opencode-ticket";
const PTY_CONNECT_TICKET_HEADER_VALUE: &str = "1";
const PTY_CONNECT_BUFFER_CHUNK: usize = 64 * 1024;

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

#[derive(Default, Deserialize)]
pub struct PtyConnectQuery {
    cursor: Option<String>,
    ticket: Option<String>,
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

pub async fn pty_connect_token(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<PtyConnectToken>, StatusCode> {
    let header_valid = headers
        .get(PTY_CONNECT_TICKET_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value == PTY_CONNECT_TICKET_HEADER_VALUE)
        .unwrap_or(false);
    if !header_valid {
        return Err(StatusCode::FORBIDDEN);
    }

    let pty_id = PtyID(id);
    if state.pty_service.get(&pty_id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(state.pty_tickets.issue(&pty_id).await))
}

pub async fn pty_connect(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<PtyConnectQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    let pty_id = PtyID(id);
    if state.pty_service.get(&pty_id).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }

    let Some(ticket) = query.ticket.as_deref() else {
        return Err(StatusCode::FORBIDDEN);
    };
    if !state.pty_tickets.consume(&pty_id, ticket).await {
        return Err(StatusCode::FORBIDDEN);
    }

    let cursor = parse_cursor(query.cursor.as_deref());
    Ok(ws
        .on_upgrade(move |socket| handle_pty_socket(state, pty_id, cursor, socket))
        .into_response())
}

async fn handle_pty_socket(
    state: Arc<AppState>,
    pty_id: PtyID,
    cursor: Option<i64>,
    socket: WebSocket,
) {
    let Some(mut output_rx) = state.pty_service.subscribe_output(&pty_id).await else {
        return;
    };
    let Some((snapshot, snapshot_end)) = state.pty_service.connect(&pty_id, cursor).await else {
        return;
    };

    let (mut sender, mut receiver) = socket.split();
    if send_snapshot(&mut sender, &snapshot).await.is_err() {
        return;
    }
    if send_meta(&mut sender, snapshot_end).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            Some(message) = receiver.next() => {
                match message {
                    Ok(Message::Text(text)) => {
                        if state.pty_service.write(&pty_id, text.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Binary(bytes)) => {
                        if let Ok(text) = String::from_utf8(bytes) {
                            if state.pty_service.write(&pty_id, text.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(Message::Ping(bytes)) => {
                        if sender.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(Message::Pong(_)) => {}
                }
            }
            output = output_rx.recv() => {
                match output {
                    Ok(output) => {
                        if output.end <= snapshot_end {
                            continue;
                        }
                        if send_text_chunk(&mut sender, &output.data).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if let Some((_, end)) = state.pty_service.connect(&pty_id, Some(-1)).await {
                            let _ = send_meta(&mut sender, end).await;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn send_snapshot(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    snapshot: &[u8],
) -> Result<(), axum::Error> {
    for chunk in snapshot.chunks(PTY_CONNECT_BUFFER_CHUNK) {
        send_text_chunk(sender, chunk).await?;
    }
    Ok(())
}

async fn send_text_chunk(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    chunk: &[u8],
) -> Result<(), axum::Error> {
    if chunk.is_empty() {
        return Ok(());
    }
    sender
        .send(Message::Text(String::from_utf8_lossy(chunk).into_owned()))
        .await
}

async fn send_meta(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    cursor: usize,
) -> Result<(), axum::Error> {
    let json = serde_json::json!({ "cursor": cursor }).to_string();
    let mut frame = Vec::with_capacity(json.len() + 1);
    frame.push(0);
    frame.extend_from_slice(json.as_bytes());
    sender.send(Message::Binary(frame)).await
}

fn parse_cursor(cursor: Option<&str>) -> Option<i64> {
    cursor.and_then(|value| value.parse::<i64>().ok())
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
