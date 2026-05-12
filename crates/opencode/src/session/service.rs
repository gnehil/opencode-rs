use anyhow::Result;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::Arc;
use std::collections::HashMap;

use crate::id::SessionID;
use crate::storage::{SessionRow, MessageRow, PartRow, init_db};
use crate::message::{Message, WithParts, Part};

/// Argument for `save_tool_part` — the outcome side of a tool call we want
/// to persist as a `ToolPart` of the assistant message.
pub enum ToolPartResult {
    Completed { output: String },
    Error { error: String },
}

pub struct SessionStore {
    pub pool: Arc<SqlitePool>,
}

impl SessionStore {
    pub async fn new(data_dir: PathBuf) -> Result<Self> {
        let db_path = data_dir.join("opencode.db");
        let pool = init_db(&db_path).await?;
        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    pub async fn create(&self, title: &str, project_id: &str, directory: &PathBuf) -> Result<SessionRow> {
        let session_id = SessionID::new();
        let now = chrono::Utc::now().timestamp_millis();
        let session_id_str = session_id.to_string();
        let slug = session_id_str.chars().take(8).collect::<String>();
        let version = "v1";
        self.ensure_project(project_id, directory, now).await?;

        sqlx::query(
            "INSERT INTO session (id, project_id, slug, directory, title, version, time_created, time_updated) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
        )
        .bind(&session_id_str)
        .bind(project_id)
        .bind(&slug)
        .bind(directory.to_string_lossy().as_ref())
        .bind(title)
        .bind(version)
        .bind(now)
        .bind(now)
        .execute(self.pool.as_ref())
        .await?;

        Ok(SessionRow {
            id: session_id_str,
            project_id: project_id.to_string(),
            workspace_id: None,
            parent_id: None,
            slug,
            directory: directory.to_string_lossy().to_string(),
            path: None,
            title: title.to_string(),
            version: version.to_string(),
            share_url: None,
            summary_additions: None,
            summary_deletions: None,
            summary_files: None,
            summary_diffs: None,
            revert: None,
            permission: None,
            agent: None,
            model: None,
            time_created: now,
            time_updated: now,
            time_compacting: None,
            time_archived: None,
        })
    }

    async fn ensure_project(&self, project_id: &str, directory: &PathBuf, now: i64) -> Result<()> {
        let name = directory
            .file_name()
            .map(|name| name.to_string_lossy().to_string());

        sqlx::query(
            "INSERT OR IGNORE INTO project (id, worktree, name, time_created, time_updated, sandboxes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
        )
        .bind(project_id)
        .bind(directory.to_string_lossy().as_ref())
        .bind(name)
        .bind(now)
        .bind(now)
        .bind("[]")
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    pub async fn get(&self, session_id: &SessionID) -> Result<Option<SessionRow>> {
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT * FROM session WHERE id = ?1"
        )
        .bind(session_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await?;

        Ok(row)
    }

    pub async fn list(&self, project_id: Option<&str>) -> Result<Vec<SessionRow>> {
        let rows = if let Some(pid) = project_id {
            sqlx::query_as::<_, SessionRow>(
                "SELECT * FROM session WHERE project_id = ?1 AND time_archived IS NULL ORDER BY time_updated DESC"
            )
            .bind(pid)
            .fetch_all(self.pool.as_ref())
            .await?
        } else {
            sqlx::query_as::<_, SessionRow>(
                "SELECT * FROM session WHERE time_archived IS NULL ORDER BY time_updated DESC"
            )
            .fetch_all(self.pool.as_ref())
            .await?
        };

        Ok(rows)
    }

    pub async fn update_title(&self, session_id: &SessionID, title: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            "UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3"
        )
        .bind(title)
        .bind(now)
        .bind(session_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    pub async fn archive(&self, session_id: &SessionID) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            "UPDATE session SET time_archived = ?1, time_updated = ?2 WHERE id = ?3"
        )
        .bind(now)
        .bind(now)
        .bind(session_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    pub async fn delete(&self, session_id: &SessionID) -> Result<()> {
        sqlx::query(
            "DELETE FROM session WHERE id = ?1"
        )
        .bind(session_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    pub async fn set_agent(&self, session_id: &SessionID, agent: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            "UPDATE session SET agent = ?1, time_updated = ?2 WHERE id = ?3"
        )
        .bind(agent)
        .bind(now)
        .bind(session_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    pub async fn set_model(&self, session_id: &SessionID, model: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query(
            "UPDATE session SET model = ?1, time_updated = ?2 WHERE id = ?3"
        )
        .bind(model)
        .bind(now)
        .bind(session_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    pub async fn get_messages(&self, session_id: &SessionID) -> Result<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM message WHERE session_id = ?1 ORDER BY time_created ASC"
        )
        .bind(session_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;

        let messages: Vec<Message> = rows
            .iter()
            .filter_map(|row| serde_json::from_str(&row.data).ok())
            .collect();

        Ok(messages)
    }

    pub async fn get_parts_by_session(&self, session_id: &SessionID) -> Result<HashMap<String, Vec<Part>>> {
        let rows = sqlx::query_as::<_, PartRow>(
            "SELECT * FROM part WHERE session_id = ?1 ORDER BY time_created ASC"
        )
        .bind(session_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;

        let mut parts_by_message: HashMap<String, Vec<Part>> = HashMap::new();
        for row in rows {
            if let Ok(part) = serde_json::from_str::<Part>(&row.data) {
                parts_by_message
                    .entry(row.message_id)
                    .or_insert_with(Vec::new)
                    .push(part);
            }
        }

        Ok(parts_by_message)
    }

    pub async fn save_message(&self, session_id: &SessionID, message: &Message) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let id = match message {
            Message::User(u) => u.id.to_string(),
            Message::Assistant(a) => a.id.to_string(),
        };
        let data = serde_json::to_string(message)?;

        sqlx::query(
            "INSERT OR REPLACE INTO message (id, session_id, time_created, time_updated, data) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&id)
        .bind(session_id.to_string())
        .bind(now)
        .bind(now)
        .bind(data)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn save_tool_part(
        &self,
        session_id: &SessionID,
        message_id: &crate::id::MessageID,
        tool_name: &str,
        call_id: &str,
        input: &serde_json::Value,
        result: ToolPartResult,
    ) -> Result<()> {
        use crate::message::ToolState;
        let part_id = crate::id::PartID::new();
        let now = chrono::Utc::now().timestamp_millis();

        let input_map: std::collections::HashMap<String, serde_json::Value> =
            match input {
                serde_json::Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                _ => std::collections::HashMap::new(),
            };

        let state = match result {
            ToolPartResult::Completed { output } => ToolState::Completed(crate::message::tool_state::ToolStateCompleted {
                input: input_map,
                output,
                title: tool_name.to_string(),
                metadata: std::collections::HashMap::new(),
                time: crate::message::tool_state::ToolStateEndedTime {
                    start: now,
                    end: now,
                    compacted: None,
                },
                attachments: None,
            }),
            ToolPartResult::Error { error } => ToolState::Error(crate::message::tool_state::ToolStateError {
                input: input_map,
                error,
                metadata: None,
                time: crate::message::tool_state::ToolStateEndedTime {
                    start: now,
                    end: now,
                    compacted: None,
                },
            }),
        };

        let tool_part = crate::message::part::ToolPart {
            id: part_id.clone(),
            session_id: session_id.clone(),
            message_id: message_id.clone(),
            call_id: call_id.to_string(),
            tool: tool_name.to_string(),
            state,
            metadata: None,
        };
        let data = serde_json::to_string(&crate::message::part::Part::Tool(tool_part))?;

        sqlx::query(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(part_id.to_string())
        .bind(message_id.to_string())
        .bind(session_id.to_string())
        .bind(now)
        .bind(now)
        .bind(data)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn save_text_part(
        &self,
        session_id: &SessionID,
        message_id: &crate::id::MessageID,
        text: &str,
    ) -> Result<()> {
        let part_id = crate::id::PartID::new();
        let now = chrono::Utc::now().timestamp_millis();
        let part = serde_json::json!({
            "id": part_id.to_string(),
            "messageID": message_id.to_string(),
            "sessionID": session_id.to_string(),
            "type": "text",
            "text": text,
        });

        sqlx::query(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(part_id.to_string())
        .bind(message_id.to_string())
        .bind(session_id.to_string())
        .bind(now)
        .bind(now)
        .bind(part.to_string())
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    pub async fn get_messages_with_parts(&self, session_id: &SessionID) -> Result<Vec<WithParts>> {
        let messages = self.get_messages(session_id).await?;
        let parts_by_message = self.get_parts_by_session(session_id).await?;

        let with_parts: Vec<WithParts> = messages
            .iter()
            .map(|msg| {
                let msg_id = match msg {
                    Message::User(u) => u.id.to_string(),
                    Message::Assistant(a) => a.id.to_string(),
                };
                WithParts {
                    info: msg.clone(),
                    parts: parts_by_message.get(&msg_id).cloned().unwrap_or_default(),
                }
            })
            .collect();

        Ok(with_parts)
    }
}
