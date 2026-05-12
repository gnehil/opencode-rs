use axum::{
    extract::{State, Query},
    http::StatusCode,
    Json,
};
use serde::{Deserialize};
use serde_json::json;
use std::sync::Arc;
use std::path::PathBuf;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct FindQuery {
    #[serde(rename = "type")]
    find_type: Option<String>,
    pattern: Option<String>,
    path: Option<String>,
}

pub async fn list_files(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let path = query.path.unwrap_or_else(|| ".".to_string());
    let path_buf = PathBuf::from(&path);

    let mut files = Vec::new();
    if path_buf.exists() && path_buf.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&path_buf) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.path().is_dir();
                files.push(json!({
                    "name": name,
                    "path": entry.path().to_string_lossy(),
                    "isDirectory": is_dir
                }));
            }
        }
    }

    Ok(Json(json!({ "files": files })))
}

pub async fn read_file(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let path = query.path.unwrap_or_else(|| ".".to_string());
    let path_buf = PathBuf::from(&path);

    if !path_buf.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    if path_buf.is_file() {
        if let Ok(content) = std::fs::read_to_string(&path_buf) {
            return Ok(Json(json!({
                "path": path,
                "content": content
            })));
        }
    }

    Err(StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn find_text(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pattern = query.pattern.unwrap_or_default();
    let path = query.path.unwrap_or_else(|| ".".to_string());

    let mut matches = Vec::new();

    let path_buf = PathBuf::from(&path);
    if path_buf.exists() {
        if let Ok(output) = std::process::Command::new("grep")
            .args(["-r", "-n", &pattern, &path])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some((file_path, rest)) = line.split_once(':') {
                    if let Some((line_num, content)) = rest.split_once(':') {
                        matches.push(json!({
                            "file": file_path,
                            "line": line_num.parse::<u64>().unwrap_or(0),
                            "content": content
                        }));
                    }
                }
            }
        }
    }

    Ok(Json(json!({ "matches": matches })))
}

pub async fn git_status(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let path = query.path.unwrap_or_else(|| ".".to_string());

    let mut status = Vec::new();
    if let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&path)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.len() >= 3 {
                let state = line[..2].trim();
                let file = line[3..].to_string();
                status.push(json!({
                    "state": state,
                    "file": file
                }));
            }
        }
    }

    Ok(Json(json!({ "status": status })))
}