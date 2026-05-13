use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct FindQuery {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    find_type: Option<String>,
    pattern: Option<String>,
    path: Option<String>,
}

/// Resolve a user-supplied path relative to the workspace root and ensure the
/// canonicalized result is still inside `root`. Returns 403 for traversal
/// attempts and 404 for paths that don't exist.
///
/// The path is treated as a workspace-relative path even when it looks
/// absolute (e.g. `/etc/passwd` becomes `<root>/etc/passwd`) so a malicious
/// client can't escape just by leading with a `/`.
pub fn resolve_inside_root(root: &Path, supplied: &str) -> Result<PathBuf, StatusCode> {
    let trimmed = supplied.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        root.to_path_buf()
    } else {
        root.join(trimmed)
    };

    let canonical = candidate
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    if !canonical.starts_with(&root_canonical) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(canonical)
}

pub async fn list_files(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let supplied = query.path.unwrap_or_else(|| ".".to_string());
    let path_buf = resolve_inside_root(&state.workspace_root, &supplied)?;

    if !path_buf.is_dir() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut files = Vec::new();
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

    Ok(Json(json!({ "files": files })))
}

pub async fn read_file(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let supplied = query.path.ok_or(StatusCode::BAD_REQUEST)?;
    let path_buf = resolve_inside_root(&state.workspace_root, &supplied)?;

    if !path_buf.is_file() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let content =
        std::fs::read_to_string(&path_buf).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({
        "path": path_buf.to_string_lossy(),
        "content": content,
    })))
}

pub async fn find_text(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pattern = query.pattern.ok_or(StatusCode::BAD_REQUEST)?;
    if pattern.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let supplied = query.path.unwrap_or_else(|| ".".to_string());
    let path_buf = resolve_inside_root(&state.workspace_root, &supplied)?;

    let mut matches = Vec::new();
    // Pattern is passed as a regex arg (no shell expansion), and `path` is
    // already canonicalized inside the workspace root.
    if let Ok(output) = std::process::Command::new("grep")
        .args(["-r", "-n", "--", pattern.as_str()])
        .arg(&path_buf)
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

    Ok(Json(json!({ "matches": matches })))
}

pub async fn git_status(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FindQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let supplied = query.path.unwrap_or_else(|| ".".to_string());
    let path_buf = resolve_inside_root(&state.workspace_root, &supplied)?;

    let mut status = Vec::new();
    if let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&path_buf)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_outside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // Absolute path treated as workspace-relative.
        let result = resolve_inside_root(&root, "/etc/passwd");
        assert_eq!(result.unwrap_err(), StatusCode::NOT_FOUND);

        // Explicit traversal that resolves outside the root.
        std::fs::create_dir(root.join("inner")).unwrap();
        let result = resolve_inside_root(&root, "inner/../../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn accepts_paths_inside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::write(root.join("hello.txt"), "hi").unwrap();

        let resolved = resolve_inside_root(&root, "hello.txt").unwrap();
        assert!(resolved.starts_with(&root));
        assert!(resolved.ends_with("hello.txt"));
    }
}
