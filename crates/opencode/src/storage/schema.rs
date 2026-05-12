use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;

pub async fn migrate(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query(MIGRATION_SQL).execute(pool).await?;
    Ok(())
}

pub fn migration_sql() -> &'static str {
    MIGRATION_SQL
}

pub async fn init_db<P: AsRef<Path>>(db_path: P) -> anyhow::Result<SqlitePool> {
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;

    let path = db_path.as_ref();

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))?
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await?;

    migrate(&pool).await?;

    Ok(pool)
}

const MIGRATION_SQL: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
PRAGMA synchronous=NORMAL;

CREATE TABLE IF NOT EXISTS project (
    id               TEXT PRIMARY KEY,
    worktree         TEXT NOT NULL,
    vcs              TEXT,
    name             TEXT,
    icon_url         TEXT,
    icon_url_override TEXT,
    icon_color       TEXT,
    time_created     INTEGER NOT NULL,
    time_updated     INTEGER NOT NULL,
    time_initialized INTEGER,
    sandboxes        TEXT NOT NULL,
    commands         TEXT
);

CREATE INDEX IF NOT EXISTS project_worktree_idx ON project(worktree);

CREATE TABLE IF NOT EXISTS session (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL,
    workspace_id      TEXT,
    parent_id         TEXT,
    slug              TEXT NOT NULL,
    directory         TEXT NOT NULL,
    path              TEXT,
    title             TEXT NOT NULL,
    version           TEXT NOT NULL,
    share_url         TEXT,
    summary_additions INTEGER,
    summary_deletions INTEGER,
    summary_files     INTEGER,
    summary_diffs     TEXT,
    revert            TEXT,
    permission        TEXT,
    agent             TEXT,
    model             TEXT,
    time_created      INTEGER NOT NULL,
    time_updated      INTEGER NOT NULL,
    time_compacting   INTEGER,
    time_archived     INTEGER,
    FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS session_project_idx ON session(project_id);
CREATE INDEX IF NOT EXISTS session_workspace_idx ON session(workspace_id);
CREATE INDEX IF NOT EXISTS session_parent_idx ON session(parent_id);

CREATE TABLE IF NOT EXISTS message (
    id           TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    data         TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS message_session_time_created_id_idx 
    ON message(session_id, time_created, id);

CREATE TABLE IF NOT EXISTS part (
    id           TEXT PRIMARY KEY,
    message_id   TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    data         TEXT NOT NULL,
    FOREIGN KEY (message_id) REFERENCES message(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS part_message_id_id_idx ON part(message_id, id);
CREATE INDEX IF NOT EXISTS part_session_idx ON part(session_id);

CREATE TABLE IF NOT EXISTS todo (
    session_id   TEXT NOT NULL,
    content      TEXT NOT NULL,
    status       TEXT NOT NULL,
    priority     TEXT NOT NULL,
    position     INTEGER NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    PRIMARY KEY (session_id, position),
    FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS todo_session_idx ON todo(session_id);

CREATE TABLE IF NOT EXISTS session_message (
    id           TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL,
    type         TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    data         TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS session_message_session_idx ON session_message(session_id);
CREATE INDEX IF NOT EXISTS session_message_session_type_idx 
    ON session_message(session_id, type);
CREATE INDEX IF NOT EXISTS session_message_time_created_idx 
    ON session_message(time_created);

CREATE TABLE IF NOT EXISTS permission (
    project_id   TEXT PRIMARY KEY,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    data         TEXT NOT NULL,
    FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS workspace (
    id         TEXT PRIMARY KEY,
    type       TEXT NOT NULL,
    name       TEXT NOT NULL DEFAULT '',
    branch     TEXT,
    directory  TEXT,
    extra      TEXT,
    project_id TEXT NOT NULL,
    time_used  INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS workspace_project_idx ON workspace(project_id);

CREATE TABLE IF NOT EXISTS session_share (
    session_id   TEXT PRIMARY KEY,
    id           TEXT NOT NULL,
    secret       TEXT NOT NULL,
    url          TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS account (
    id            TEXT PRIMARY KEY,
    email         TEXT NOT NULL,
    url           TEXT NOT NULL,
    access_token  TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    token_expiry  INTEGER,
    time_created  INTEGER NOT NULL,
    time_updated  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS account_state (
    id                INTEGER PRIMARY KEY,
    active_account_id TEXT,
    active_org_id     TEXT,
    FOREIGN KEY (active_account_id) REFERENCES account(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS control_account (
    email         TEXT NOT NULL,
    url           TEXT NOT NULL,
    access_token  TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    token_expiry  INTEGER,
    active        INTEGER NOT NULL DEFAULT 0,
    time_created  INTEGER NOT NULL,
    time_updated  INTEGER NOT NULL,
    PRIMARY KEY (email, url)
);

CREATE TABLE IF NOT EXISTS event_sequence (
    aggregate_id TEXT PRIMARY KEY,
    seq          INTEGER NOT NULL,
    owner_id     TEXT
);

CREATE TABLE IF NOT EXISTS event (
    id           TEXT PRIMARY KEY,
    aggregate_id TEXT NOT NULL,
    seq          INTEGER NOT NULL,
    type         TEXT NOT NULL,
    data         TEXT NOT NULL,
    FOREIGN KEY (aggregate_id) REFERENCES event_sequence(aggregate_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS event_aggregate_idx ON event(aggregate_id);
CREATE INDEX IF NOT EXISTS event_seq_idx ON event(aggregate_id, seq);

CREATE TABLE IF NOT EXISTS data_migration (
    name           TEXT PRIMARY KEY,
    time_completed INTEGER NOT NULL
);
"#;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ProjectRow {
    pub id: String,
    pub worktree: String,
    pub vcs: Option<String>,
    pub name: Option<String>,
    pub icon_url: Option<String>,
    pub icon_url_override: Option<String>,
    pub icon_color: Option<String>,
    pub time_created: i64,
    pub time_updated: i64,
    pub time_initialized: Option<i64>,
    pub sandboxes: String,
    pub commands: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub project_id: String,
    pub workspace_id: Option<String>,
    pub parent_id: Option<String>,
    pub slug: String,
    pub directory: String,
    pub path: Option<String>,
    pub title: String,
    pub version: String,
    pub share_url: Option<String>,
    pub summary_additions: Option<i64>,
    pub summary_deletions: Option<i64>,
    pub summary_files: Option<i64>,
    pub summary_diffs: Option<String>,
    pub revert: Option<String>,
    pub permission: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub time_created: i64,
    pub time_updated: i64,
    pub time_compacting: Option<i64>,
    pub time_archived: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct MessageRow {
    pub id: String,
    pub session_id: String,
    pub time_created: i64,
    pub time_updated: i64,
    pub data: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PartRow {
    pub id: String,
    pub message_id: String,
    pub session_id: String,
    pub time_created: i64,
    pub time_updated: i64,
    pub data: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct TodoRow {
    pub session_id: String,
    pub content: String,
    pub status: String,
    pub priority: String,
    pub position: i64,
    pub time_created: i64,
    pub time_updated: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct SessionMessageRow {
    pub id: String,
    pub session_id: String,
    pub r#type: String,
    pub time_created: i64,
    pub time_updated: i64,
    pub data: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct PermissionRow {
    pub project_id: String,
    pub time_created: i64,
    pub time_updated: i64,
    pub data: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct WorkspaceRow {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub branch: Option<String>,
    pub directory: Option<String>,
    pub extra: Option<String>,
    pub project_id: String,
    pub time_used: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct SessionShareRow {
    pub session_id: String,
    pub id: String,
    pub secret: String,
    pub url: String,
    pub time_created: i64,
    pub time_updated: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct AccountRow {
    pub id: String,
    pub email: String,
    pub url: String,
    pub access_token: String,
    pub refresh_token: String,
    pub token_expiry: Option<i64>,
    pub time_created: i64,
    pub time_updated: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct AccountStateRow {
    pub id: i64,
    pub active_account_id: Option<String>,
    pub active_org_id: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ControlAccountRow {
    pub email: String,
    pub url: String,
    pub access_token: String,
    pub refresh_token: String,
    pub token_expiry: Option<i64>,
    pub active: bool,
    pub time_created: i64,
    pub time_updated: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct EventSequenceRow {
    pub aggregate_id: String,
    pub seq: i64,
    pub owner_id: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct EventRow {
    pub id: String,
    pub aggregate_id: String,
    pub seq: i64,
    pub r#type: String,
    pub data: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct DataMigrationRow {
    pub name: String,
    pub time_completed: i64,
}
