use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct FileQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
pub struct FindTextQuery {
    pattern: String,
}

#[derive(Deserialize)]
pub struct FindFileQuery {
    query: String,
    dirs: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct FindSymbolQuery {
    #[allow(dead_code)]
    query: String,
}

/// Resolve a user-supplied path relative to the workspace root and ensure the
/// canonicalized result is still inside `root`. Returns 403 for traversal
/// attempts and 404 for paths that don't exist.
///
/// The path is treated as a workspace-relative path even when it looks
/// absolute (e.g. `/etc/passwd` becomes `<root>/etc/passwd`) so a malicious
/// client can't escape just by leading with a `/`.
pub fn resolve_inside_root(root: &Path, supplied: &str) -> Result<PathBuf, StatusCode> {
    let candidate = resolve_workspace_path(root, supplied)?;
    let canonical = candidate
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    if !canonical.starts_with(&root_canonical) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(canonical)
}

fn resolve_workspace_path(root: &Path, supplied: &str) -> Result<PathBuf, StatusCode> {
    let mut candidate = root.to_path_buf();
    let trimmed = supplied.trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(candidate);
    }

    for component in Path::new(trimmed).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => candidate.push(part),
            Component::ParentDir => return Err(StatusCode::FORBIDDEN),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }

    Ok(candidate)
}

fn root_canonical(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

fn relative_path(root: &Path, path: &Path) -> String {
    let root = root_canonical(root);
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    absolute
        .strip_prefix(&root)
        .unwrap_or(&absolute)
        .to_string_lossy()
        .replace('\\', "/")
}

fn workspace_ignore(root: &Path) -> Option<ignore::gitignore::Gitignore> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    for name in [".gitignore", ".ignore"] {
        let file = root.join(name);
        if file.exists() {
            let _ = builder.add(file);
        }
    }
    builder.build().ok()
}

fn is_excluded_name(name: &str) -> bool {
    matches!(name, ".git" | ".DS_Store")
}

fn is_excluded_path(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::Normal(name) if name == ".git" || name == ".DS_Store"))
}

fn ignored_by_workspace(
    matcher: Option<&ignore::gitignore::Gitignore>,
    path: &Path,
    is_dir: bool,
) -> bool {
    matcher
        .map(|matcher| matcher.matched(path, is_dir).is_ignore())
        .unwrap_or(false)
}

pub async fn list_files(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let supplied = query.path.unwrap_or_else(|| ".".to_string());
    let path_buf = resolve_inside_root(&state.workspace_root, &supplied)?;

    if !path_buf.is_dir() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let root = root_canonical(&state.workspace_root);
    let ignore = workspace_ignore(&root);
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&path_buf) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_excluded_name(&name) {
                continue;
            }
            let is_dir = entry.path().is_dir();
            let absolute = entry.path();
            let rel = relative_path(&root, &absolute);
            let ignored = ignore
                .as_ref()
                .map(|matcher| matcher.matched(&absolute, is_dir).is_ignore())
                .unwrap_or(false);
            files.push(json!({
                "name": name,
                "path": rel,
                "absolute": absolute.to_string_lossy(),
                "type": if is_dir { "directory" } else { "file" },
                "ignored": ignored,
            }));
        }
    }
    files.sort_by(|a, b| {
        let a_type = a["type"].as_str().unwrap_or("");
        let b_type = b["type"].as_str().unwrap_or("");
        if a_type != b_type {
            return if a_type == "directory" {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });

    Ok(Json(json!(files)))
}

pub async fn read_file(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let supplied = query.path.ok_or(StatusCode::BAD_REQUEST)?;
    let path_buf = resolve_workspace_path(&state.workspace_root, &supplied)?;

    if path_buf.exists() {
        let canonical = path_buf.canonicalize().map_err(|_| StatusCode::NOT_FOUND)?;
        if !canonical.starts_with(root_canonical(&state.workspace_root)) {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    if is_binary_file(&path_buf) {
        return Ok(Json(json!({
            "type": "binary",
            "content": "",
        })));
    }

    let content = std::fs::read_to_string(&path_buf)
        .map(|content| content.trim().to_string())
        .unwrap_or_default();
    Ok(Json(json!({
        "type": "text",
        "content": content,
    })))
}

pub async fn find_text(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FindTextQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pattern = query.pattern;
    if pattern.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let regex = regex::Regex::new(&pattern).map_err(|_| StatusCode::BAD_REQUEST)?;
    let root = root_canonical(&state.workspace_root);
    let ignore = workspace_ignore(&root);

    let mut matches = Vec::new();
    for entry in WalkDir::new(&root).into_iter().flatten() {
        if is_excluded_path(entry.path()) {
            continue;
        }
        if ignored_by_workspace(ignore.as_ref(), entry.path(), entry.file_type().is_dir()) {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let mut offset = 0usize;
        for (idx, line) in content.lines().enumerate() {
            let submatches: Vec<_> = regex
                .find_iter(line)
                .map(|m| {
                    json!({
                        "match": { "text": m.as_str() },
                        "start": m.start(),
                        "end": m.end(),
                    })
                })
                .collect();
            if !submatches.is_empty() {
                matches.push(json!({
                    "path": { "text": relative_path(&root, entry.path()) },
                    "lines": { "text": line },
                    "line_number": idx + 1,
                    "absolute_offset": offset,
                    "submatches": submatches,
                }));
            }
            offset += line.len() + 1;
        }
    }

    Ok(Json(json!(matches)))
}

pub async fn find_file(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FindFileQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let limit = query.limit.unwrap_or(10);
    if !(1..=200).contains(&limit) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let kind = match query.kind.as_deref() {
        Some("file") => SearchKind::File,
        Some("directory") => SearchKind::Directory,
        Some(_) => return Err(StatusCode::BAD_REQUEST),
        None if query.dirs.as_deref() == Some("false") => SearchKind::File,
        None => SearchKind::All,
    };
    if let Some(dirs) = query.dirs.as_deref() {
        if dirs != "true" && dirs != "false" {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let root = root_canonical(&state.workspace_root);
    let (files, dirs) = scan_workspace_files(&root);
    let items: Vec<String> = match kind {
        SearchKind::File => files,
        SearchKind::Directory => dirs,
        SearchKind::All if query.query.trim().is_empty() => dirs,
        SearchKind::All => files.into_iter().chain(dirs.into_iter()).collect(),
    };
    let results = search_paths(&query.query, items, kind, limit);
    Ok(Json(json!(results)))
}

pub async fn find_symbol(
    Query(_query): Query<FindSymbolQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    Ok(Json(json!([])))
}

pub async fn git_status(
    State(state): State<Arc<AppState>>,
    Query(_query): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let root = root_canonical(&state.workspace_root);
    if !root.join(".git").exists() {
        return Ok(Json(json!([])));
    }

    let mut status = Vec::new();
    if let Ok(output) = std::process::Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotepath=false",
            "diff",
            "--numstat",
            "HEAD",
        ])
        .current_dir(&root)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let mut parts = line.splitn(3, '\t');
            let added = parse_numstat(parts.next());
            let removed = parse_numstat(parts.next());
            let Some(path) = parts.next() else { continue };
            status.push(json!({
                "path": path,
                "added": added,
                "removed": removed,
                "status": "modified",
            }));
        }
    }

    if let Ok(output) = std::process::Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotepath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(&root)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for file in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let added = std::fs::read_to_string(root.join(file))
                .map(|content| content.split('\n').count())
                .unwrap_or(0);
            status.push(json!({
                "path": file,
                "added": added,
                "removed": 0,
                "status": "added",
            }));
        }
    }

    if let Ok(output) = std::process::Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.quotepath=false",
            "diff",
            "--name-only",
            "--diff-filter=D",
            "HEAD",
        ])
        .current_dir(&root)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for file in stdout.lines().filter(|line| !line.trim().is_empty()) {
            status.push(json!({
                "path": file,
                "added": 0,
                "removed": 0,
                "status": "deleted",
            }));
        }
    }

    Ok(Json(json!(status)))
}

fn parse_numstat(value: Option<&str>) -> usize {
    value.and_then(|value| value.parse().ok()).unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchKind {
    File,
    Directory,
    All,
}

fn scan_workspace_files(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let ignore = workspace_ignore(root);
    for entry in WalkDir::new(root).into_iter().flatten() {
        if is_excluded_path(entry.path()) {
            continue;
        }
        if entry.path() == root {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if is_excluded_name(&name) {
            continue;
        }
        if ignored_by_workspace(ignore.as_ref(), entry.path(), entry.file_type().is_dir()) {
            continue;
        }
        let rel = relative_path(root, entry.path());
        if entry.file_type().is_dir() {
            dirs.push(format!("{rel}/"));
        } else if entry.file_type().is_file() {
            files.push(rel);
        }
    }
    files.sort();
    dirs.sort();
    (files, dirs)
}

fn search_paths(query: &str, items: Vec<String>, kind: SearchKind, limit: usize) -> Vec<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        let mut items = items;
        if kind == SearchKind::Directory || kind == SearchKind::All {
            sort_hidden_last(&mut items, false);
        }
        items.truncate(limit);
        return items;
    }

    let prefer_hidden = trimmed.starts_with('.') || trimmed.contains("/.");
    let mut scored: Vec<_> = items
        .into_iter()
        .filter_map(|item| fuzzy_score(trimmed, &item).map(|score| (score, item)))
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| a_score.cmp(b_score).then_with(|| a.cmp(b)));
    let mut output: Vec<_> = scored.into_iter().map(|(_, item)| item).collect();
    if kind == SearchKind::Directory {
        sort_hidden_last(&mut output, prefer_hidden);
    }
    output.truncate(limit);
    output
}

fn fuzzy_score(query: &str, target: &str) -> Option<(usize, usize, usize)> {
    let query = query.to_lowercase();
    let target_lower = target.to_lowercase();
    if let Some(pos) = target_lower.find(&query) {
        return Some((0, pos, target.len()));
    }

    let mut last = 0usize;
    let mut spread = 0usize;
    for ch in query.chars() {
        let relative = target_lower[last..].find(ch)?;
        last += relative + ch.len_utf8();
        spread += relative;
    }
    Some((1, spread, target.len()))
}

fn hidden(path: &str) -> bool {
    path.split('/')
        .filter(|part| !part.is_empty())
        .any(|part| part.starts_with('.') && part.len() > 1)
}

fn sort_hidden_last(items: &mut [String], prefer_hidden: bool) {
    if prefer_hidden {
        return;
    }
    items.sort_by(|a, b| hidden(a).cmp(&hidden(b)).then_with(|| a.cmp(b)));
}

fn is_binary_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "exe"
            | "dll"
            | "pdb"
            | "bin"
            | "so"
            | "dylib"
            | "o"
            | "a"
            | "lib"
            | "wav"
            | "mp3"
            | "ogg"
            | "flac"
            | "aac"
            | "mp4"
            | "avi"
            | "mov"
            | "wmv"
            | "webm"
            | "mkv"
            | "zip"
            | "tar"
            | "gz"
            | "bz2"
            | "7z"
            | "rar"
            | "xz"
            | "pdf"
            | "doc"
            | "docx"
            | "ppt"
            | "pptx"
            | "xls"
            | "xlsx"
            | "dmg"
            | "iso"
            | "sqlite"
            | "db"
    )
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
