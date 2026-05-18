use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::storage::ProjectRow;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUpdateBody {
    name: Option<String>,
    icon: Option<ProjectIconInput>,
    commands: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIconInput {
    url: Option<String>,
    #[serde(rename = "override")]
    override_url: Option<String>,
    color: Option<String>,
}

struct DiscoveredProject {
    id: String,
    worktree: String,
    vcs: Option<String>,
}

pub async fn project_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let store = state.get_store().await;
    let current = discover_project(&state.workspace_root);
    upsert_current_project(&store, &current)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let rows = sqlx::query_as::<_, ProjectRow>("SELECT * FROM project ORDER BY time_updated DESC")
        .fetch_all(store.pool.as_ref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(rows.into_iter().map(project_json).collect()))
}

pub async fn project_current(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state.get_store().await;
    let current = discover_project(&state.workspace_root);
    let row = upsert_current_project(&store, &current)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(project_json(row)))
}

pub async fn project_init_git(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_git_worktree(&state.workspace_root) {
        let output = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&state.workspace_root)
            .output()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        if !output.status.success() {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    project_current(State(state)).await
}

pub async fn project_update(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectUpdateBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let store = state.get_store().await;
    if project_id == discover_project(&state.workspace_root).id {
        let current = discover_project(&state.workspace_root);
        upsert_current_project(&store, &current)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    let existing = sqlx::query_as::<_, ProjectRow>("SELECT * FROM project WHERE id = ?1")
        .bind(&project_id)
        .fetch_optional(store.pool.as_ref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let icon = body.icon.unwrap_or(ProjectIconInput {
        url: existing.icon_url,
        override_url: existing.icon_url_override,
        color: existing.icon_color,
    });
    let commands = body
        .commands
        .map(|commands| commands.to_string())
        .or(existing.commands);
    let now = chrono::Utc::now().timestamp_millis();

    let row = sqlx::query_as::<_, ProjectRow>(
        "UPDATE project
         SET name = ?1, icon_url = ?2, icon_url_override = ?3, icon_color = ?4,
             commands = ?5, time_updated = ?6
         WHERE id = ?7
         RETURNING *",
    )
    .bind(body.name.or(existing.name))
    .bind(icon.url)
    .bind(icon.override_url)
    .bind(icon.color)
    .bind(commands)
    .bind(now)
    .bind(project_id)
    .fetch_optional(store.pool.as_ref())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(project_json(row)))
}

async fn upsert_current_project(
    store: &crate::session::SessionStore,
    current: &DiscoveredProject,
) -> anyhow::Result<ProjectRow> {
    let now = chrono::Utc::now().timestamp_millis();
    let name = std::path::Path::new(&current.worktree)
        .file_name()
        .map(|name| name.to_string_lossy().to_string());

    sqlx::query(
        "INSERT OR IGNORE INTO project
         (id, worktree, vcs, name, time_created, time_updated, sandboxes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(&current.id)
    .bind(&current.worktree)
    .bind(&current.vcs)
    .bind(name)
    .bind(now)
    .bind(now)
    .bind("[]")
    .execute(store.pool.as_ref())
    .await?;

    sqlx::query("UPDATE project SET worktree = ?1, vcs = ?2, time_updated = ?3 WHERE id = ?4")
        .bind(&current.worktree)
        .bind(&current.vcs)
        .bind(now)
        .bind(&current.id)
        .execute(store.pool.as_ref())
        .await?;

    Ok(
        sqlx::query_as::<_, ProjectRow>("SELECT * FROM project WHERE id = ?1")
            .bind(&current.id)
            .fetch_one(store.pool.as_ref())
            .await?,
    )
}

fn discover_project(root: &std::path::Path) -> DiscoveredProject {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let is_git = is_git_worktree(&root);
    let worktree = if is_git {
        git_success_text(&root, &["rev-parse", "--show-toplevel"])
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| root.to_string_lossy().to_string())
    } else {
        root.to_string_lossy().to_string()
    };
    let id = if is_git {
        git_success_text(&root, &["rev-list", "--max-parents=0", "HEAD"])
            .and_then(|text| {
                text.lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| "global".to_string())
    } else {
        "global".to_string()
    };
    DiscoveredProject {
        id,
        worktree,
        vcs: is_git.then(|| "git".to_string()),
    }
}

fn project_json(row: ProjectRow) -> serde_json::Value {
    let sandboxes = serde_json::from_str::<Vec<String>>(&row.sandboxes).unwrap_or_default();
    let commands = row
        .commands
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok());
    let icon =
        if row.icon_url.is_some() || row.icon_url_override.is_some() || row.icon_color.is_some() {
            Some(serde_json::json!({
                "url": row.icon_url,
                "override": row.icon_url_override,
                "color": row.icon_color,
            }))
        } else {
            None
        };
    let mut object = serde_json::json!({
        "id": row.id,
        "worktree": row.worktree,
        "vcs": row.vcs,
        "name": row.name,
        "time": {
            "created": row.time_created,
            "updated": row.time_updated,
            "initialized": row.time_initialized,
        },
        "sandboxes": sandboxes,
    });
    if let serde_json::Value::Object(map) = &mut object {
        if let Some(icon) = icon {
            map.insert("icon".to_string(), icon);
        }
        if let Some(commands) = commands {
            map.insert("commands".to_string(), commands);
        }
    }
    object
}

fn is_git_worktree(root: &std::path::Path) -> bool {
    git_output(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_success_text(root: &std::path::Path, args: &[&str]) -> Option<String> {
    git_output(root, args)
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_output(root: &std::path::Path, args: &[&str]) -> Option<std::process::Output> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()
}
