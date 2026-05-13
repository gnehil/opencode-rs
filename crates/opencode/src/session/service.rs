use anyhow::Result;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use std::sync::atomic::{AtomicI64, Ordering};

use crate::id::{MessageID, PartID, SessionID};
use crate::message::{Message, Part, WithParts};
use crate::storage::{init_db, MessageRow, PartRow, SessionRow};

/// Argument for `save_tool_part` — the outcome side of a tool call we want
/// to persist as a `ToolPart` of the assistant message.
#[derive(Debug, Clone)]
pub enum ToolPartResult {
    Completed {
        output: String,
        /// File attachments (typically images) the tool produced. They
        /// get persisted into ToolStateCompleted.attachments so the
        /// next history rebuild can surface them as vision input.
        attachments: Vec<crate::message::part::FilePart>,
    },
    Error {
        error: String,
    },
}

/// Process-wide monotonic clock for persistence timestamps.
///
/// Two `save_message` calls in the same millisecond would otherwise share
/// a `time_created` value, and SQLite's stable sort can pick either
/// order on retrieval. The downstream conversation-history rebuild needs
/// the ordering to match insertion order, so we hand out a unique,
/// strictly increasing integer per call by taking the system clock as a
/// floor and incrementing past any prior value.
static LAST_TS_MS: AtomicI64 = AtomicI64::new(0);

fn next_monotonic_ms() -> i64 {
    let wall = chrono::Utc::now().timestamp_millis();
    loop {
        let prev = LAST_TS_MS.load(Ordering::SeqCst);
        let next = if wall > prev { wall } else { prev + 1 };
        match LAST_TS_MS.compare_exchange(prev, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return next,
            Err(_) => continue,
        }
    }
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

    pub async fn create(
        &self,
        title: &str,
        project_id: &str,
        directory: &PathBuf,
    ) -> Result<SessionRow> {
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
        let row = sqlx::query_as::<_, SessionRow>("SELECT * FROM session WHERE id = ?1")
            .bind(session_id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await?;

        Ok(row)
    }

    pub async fn get_todos(&self, session_id: &SessionID) -> Result<Vec<crate::tool::TodoItem>> {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT content, status, priority
             FROM todo
             WHERE session_id = ?1
             ORDER BY position ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;

        Ok(rows
            .into_iter()
            .map(|(content, status, priority)| crate::tool::TodoItem {
                content,
                status,
                priority,
            })
            .collect())
    }

    pub async fn replace_todos(
        &self,
        session_id: &SessionID,
        todos: &[crate::tool::TodoItem],
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM todo WHERE session_id = ?1")
            .bind(session_id.to_string())
            .execute(&mut *tx)
            .await?;

        for (position, todo) in todos.iter().enumerate() {
            sqlx::query(
                "INSERT INTO todo
                 (session_id, content, status, priority, position, time_created, time_updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .bind(session_id.to_string())
            .bind(&todo.content)
            .bind(&todo.status)
            .bind(&todo.priority)
            .bind(position as i64)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
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
                "SELECT * FROM session WHERE time_archived IS NULL ORDER BY time_updated DESC",
            )
            .fetch_all(self.pool.as_ref())
            .await?
        };

        Ok(rows)
    }

    pub async fn update_title(&self, session_id: &SessionID, title: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query("UPDATE session SET title = ?1, time_updated = ?2 WHERE id = ?3")
            .bind(title)
            .bind(now)
            .bind(session_id.to_string())
            .execute(self.pool.as_ref())
            .await?;

        Ok(())
    }

    pub async fn archive(&self, session_id: &SessionID) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query("UPDATE session SET time_archived = ?1, time_updated = ?2 WHERE id = ?3")
            .bind(now)
            .bind(now)
            .bind(session_id.to_string())
            .execute(self.pool.as_ref())
            .await?;

        Ok(())
    }

    pub async fn delete(&self, session_id: &SessionID) -> Result<()> {
        sqlx::query("DELETE FROM session WHERE id = ?1")
            .bind(session_id.to_string())
            .execute(self.pool.as_ref())
            .await?;

        Ok(())
    }

    pub async fn set_agent(&self, session_id: &SessionID, agent: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query("UPDATE session SET agent = ?1, time_updated = ?2 WHERE id = ?3")
            .bind(agent)
            .bind(now)
            .bind(session_id.to_string())
            .execute(self.pool.as_ref())
            .await?;

        Ok(())
    }

    pub async fn set_time_compacting(&self, session_id: &SessionID, ts: i64) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query("UPDATE session SET time_compacting = ?1, time_updated = ?2 WHERE id = ?3")
            .bind(ts)
            .bind(now)
            .bind(session_id.to_string())
            .execute(self.pool.as_ref())
            .await?;
        Ok(())
    }

    pub async fn set_model(&self, session_id: &SessionID, model: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();

        sqlx::query("UPDATE session SET model = ?1, time_updated = ?2 WHERE id = ?3")
            .bind(model)
            .bind(now)
            .bind(session_id.to_string())
            .execute(self.pool.as_ref())
            .await?;

        Ok(())
    }

    pub async fn get_messages(&self, session_id: &SessionID) -> Result<Vec<Message>> {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM message WHERE session_id = ?1 ORDER BY time_created ASC, id ASC",
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

    pub async fn get_parts_by_session(
        &self,
        session_id: &SessionID,
    ) -> Result<HashMap<String, Vec<Part>>> {
        let rows = sqlx::query_as::<_, PartRow>(
            "SELECT * FROM part WHERE session_id = ?1 ORDER BY time_created ASC, id ASC",
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
        let now = next_monotonic_ms();
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
        let now = next_monotonic_ms();

        let input_map: std::collections::HashMap<String, serde_json::Value> = match input {
            serde_json::Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => std::collections::HashMap::new(),
        };

        let state = match result {
            ToolPartResult::Completed {
                output,
                attachments,
            } => ToolState::Completed(crate::message::tool_state::ToolStateCompleted {
                input: input_map,
                output,
                title: tool_name.to_string(),
                metadata: std::collections::HashMap::new(),
                time: crate::message::tool_state::ToolStateEndedTime {
                    start: now,
                    end: now,
                    compacted: None,
                },
                attachments: if attachments.is_empty() {
                    None
                } else {
                    Some(attachments)
                },
            }),
            ToolPartResult::Error { error } => {
                ToolState::Error(crate::message::tool_state::ToolStateError {
                    input: input_map,
                    error,
                    metadata: None,
                    time: crate::message::tool_state::ToolStateEndedTime {
                        start: now,
                        end: now,
                        compacted: None,
                    },
                })
            }
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
        let now = next_monotonic_ms();
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

    pub async fn save_part(&self, part: &Part) -> Result<()> {
        let (session_id, message_id, part_id) = part_storage_ids(part);
        let now = next_monotonic_ms();
        let data = serde_json::to_string(part)?;

        sqlx::query(
            "INSERT OR REPLACE INTO part (id, message_id, session_id, time_created, time_updated, data) \
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

    pub async fn get_message_with_parts(
        &self,
        session_id: &SessionID,
        message_id: &MessageID,
    ) -> Result<Option<WithParts>> {
        let Some(row) = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM message WHERE session_id = ?1 AND id = ?2",
        )
        .bind(session_id.to_string())
        .bind(message_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await?
        else {
            return Ok(None);
        };

        let info = serde_json::from_str::<Message>(&row.data)?;
        let part_rows = sqlx::query_as::<_, PartRow>(
            "SELECT * FROM part WHERE session_id = ?1 AND message_id = ?2 ORDER BY time_created ASC, id ASC",
        )
        .bind(session_id.to_string())
        .bind(message_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await?;
        let parts = part_rows
            .into_iter()
            .filter_map(|row| serde_json::from_str::<Part>(&row.data).ok())
            .collect();

        Ok(Some(WithParts { info, parts }))
    }

    pub async fn delete_message(
        &self,
        session_id: &SessionID,
        message_id: &MessageID,
    ) -> Result<bool> {
        sqlx::query("DELETE FROM part WHERE session_id = ?1 AND message_id = ?2")
            .bind(session_id.to_string())
            .bind(message_id.to_string())
            .execute(self.pool.as_ref())
            .await?;
        let result = sqlx::query("DELETE FROM message WHERE session_id = ?1 AND id = ?2")
            .bind(session_id.to_string())
            .bind(message_id.to_string())
            .execute(self.pool.as_ref())
            .await?;
        let deleted = result.rows_affected() > 0;
        if deleted {
            self.touch_session(session_id).await?;
        }
        Ok(deleted)
    }

    pub async fn delete_part(
        &self,
        session_id: &SessionID,
        message_id: &MessageID,
        part_id: &PartID,
    ) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM part WHERE session_id = ?1 AND message_id = ?2 AND id = ?3")
                .bind(session_id.to_string())
                .bind(message_id.to_string())
                .bind(part_id.to_string())
                .execute(self.pool.as_ref())
                .await?;
        let deleted = result.rows_affected() > 0;
        if deleted {
            self.touch_session(session_id).await?;
        }
        Ok(deleted)
    }

    pub async fn update_part(
        &self,
        session_id: &SessionID,
        message_id: &MessageID,
        part_id: &PartID,
        part: &Part,
    ) -> Result<Option<Part>> {
        let existing = sqlx::query_as::<_, PartRow>(
            "SELECT * FROM part WHERE session_id = ?1 AND message_id = ?2 AND id = ?3",
        )
        .bind(session_id.to_string())
        .bind(message_id.to_string())
        .bind(part_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await?;
        if existing.is_none() {
            return Ok(None);
        }

        let now = chrono::Utc::now().timestamp_millis();
        let data = serde_json::to_string(part)?;
        sqlx::query(
            "UPDATE part SET data = ?1, time_updated = ?2
             WHERE session_id = ?3 AND message_id = ?4 AND id = ?5",
        )
        .bind(data)
        .bind(now)
        .bind(session_id.to_string())
        .bind(message_id.to_string())
        .bind(part_id.to_string())
        .execute(self.pool.as_ref())
        .await?;
        self.touch_session(session_id).await?;

        Ok(Some(part.clone()))
    }

    pub async fn clear_revert(&self, session_id: &SessionID) -> Result<Option<SessionRow>> {
        if self.get(session_id).await?.is_none() {
            return Ok(None);
        }

        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "UPDATE session
             SET revert = NULL,
                 time_updated = ?1
             WHERE id = ?2",
        )
        .bind(now)
        .bind(session_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        self.get(session_id).await
    }

    async fn touch_session(&self, session_id: &SessionID) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query("UPDATE session SET time_updated = ?1 WHERE id = ?2")
            .bind(now)
            .bind(session_id.to_string())
            .execute(self.pool.as_ref())
            .await?;
        Ok(())
    }

    pub async fn revert_to_message(
        &self,
        session_id: &SessionID,
        message_id: &MessageID,
        part_id: Option<&PartID>,
    ) -> Result<Option<SessionRow>> {
        let Some(target) = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM message WHERE session_id = ?1 AND id = ?2",
        )
        .bind(session_id.to_string())
        .bind(message_id.to_string())
        .fetch_optional(self.pool.as_ref())
        .await?
        else {
            return Ok(None);
        };

        if let Some(part_id) = part_id {
            let part = sqlx::query_as::<_, PartRow>(
                "SELECT * FROM part WHERE session_id = ?1 AND message_id = ?2 AND id = ?3",
            )
            .bind(session_id.to_string())
            .bind(message_id.to_string())
            .bind(part_id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await?;
            if part.is_none() {
                return Ok(None);
            }
        }

        if let Some(part_id) = part_id {
            let target_part = sqlx::query_as::<_, PartRow>(
                "SELECT * FROM part WHERE session_id = ?1 AND message_id = ?2 AND id = ?3",
            )
            .bind(session_id.to_string())
            .bind(message_id.to_string())
            .bind(part_id.to_string())
            .fetch_one(self.pool.as_ref())
            .await?;

            sqlx::query(
                "DELETE FROM part
                 WHERE session_id = ?1
                   AND message_id = ?2
                   AND (time_created > ?3 OR (time_created = ?3 AND id >= ?4))",
            )
            .bind(session_id.to_string())
            .bind(message_id.to_string())
            .bind(target_part.time_created)
            .bind(part_id.to_string())
            .execute(self.pool.as_ref())
            .await?;
        }

        sqlx::query(
            "DELETE FROM message
             WHERE session_id = ?1
               AND (time_created > ?2 OR (time_created = ?2 AND id > ?3))",
        )
        .bind(session_id.to_string())
        .bind(target.time_created)
        .bind(message_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        let now = chrono::Utc::now().timestamp_millis();
        let revert = serde_json::to_string(&crate::session::SessionRevert {
            message_id: message_id.to_string(),
            part_id: part_id.map(|id| id.to_string()),
            snapshot: None,
            diff: None,
        })?;

        sqlx::query(
            "UPDATE session
             SET revert = ?1,
                 summary_additions = NULL,
                 summary_deletions = NULL,
                 summary_files = NULL,
                 summary_diffs = NULL,
                 time_updated = ?2
             WHERE id = ?3",
        )
        .bind(revert)
        .bind(now)
        .bind(session_id.to_string())
        .execute(self.pool.as_ref())
        .await?;

        self.get(session_id).await
    }
}

fn part_storage_ids(part: &Part) -> (&SessionID, &MessageID, PartID) {
    match part {
        Part::Text(part) => (&part.session_id, &part.message_id, part.id),
        Part::Subtask(part) => (&part.session_id, &part.message_id, part.id),
        Part::Reasoning(part) => (&part.session_id, &part.message_id, part.id),
        Part::File(part) => (&part.session_id, &part.message_id, part.id),
        Part::Tool(part) => (&part.session_id, &part.message_id, part.id),
        Part::StepStart(part) => (&part.session_id, &part.message_id, part.id),
        Part::StepFinish(part) => (&part.session_id, &part.message_id, part.id),
        Part::Snapshot(part) => (&part.session_id, &part.message_id, part.id),
        Part::Patch(part) => (&part.session_id, &part.message_id, part.id),
        Part::Agent(part) => (&part.session_id, &part.message_id, part.id),
        Part::Retry(part) => (&part.session_id, &part.message_id, part.id),
        Part::Compaction(part) => (&part.session_id, &part.message_id, part.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::MessageID;
    use crate::message::{Message, ModelRef, Part, UserMessage, UserTime};

    fn user_message(session_id: &SessionID) -> (MessageID, Message) {
        let message_id = MessageID::new();
        (
            message_id,
            Message::User(UserMessage {
                id: message_id,
                session_id: session_id.clone(),
                time: UserTime { created: 0 },
                agent: "build".to_string(),
                model: ModelRef {
                    provider_id: "test".to_string(),
                    model_id: "test-model".to_string(),
                    variant: None,
                },
                ..Default::default()
            }),
        )
    }

    fn message_id(message: &Message) -> String {
        match message {
            Message::User(user) => user.id.to_string(),
            Message::Assistant(assistant) => assistant.id.to_string(),
        }
    }

    fn part_id(part: &Part) -> PartID {
        match part {
            Part::Text(part) => part.id,
            Part::Subtask(part) => part.id,
            Part::Reasoning(part) => part.id,
            Part::File(part) => part.id,
            Part::Tool(part) => part.id,
            Part::StepStart(part) => part.id,
            Part::StepFinish(part) => part.id,
            Part::Snapshot(part) => part.id,
            Part::Patch(part) => part.id,
            Part::Agent(part) => part.id,
            Part::Retry(part) => part.id,
            Part::Compaction(part) => part.id,
        }
    }

    #[tokio::test]
    async fn message_and_part_mutation_apis_update_storage() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(tmp.path().to_path_buf()).await.unwrap();
        let session = store
            .create("test", "default", &PathBuf::from("/tmp/project"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let (msg_id, message) = user_message(&session_id);
        store.save_message(&session_id, &message).await.unwrap();
        store
            .save_text_part(&session_id, &msg_id, "original")
            .await
            .unwrap();

        let with_parts = store
            .get_message_with_parts(&session_id, &msg_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message_id(&with_parts.info), msg_id.to_string());
        assert_eq!(with_parts.parts.len(), 1);

        let mut updated_part = with_parts.parts[0].clone();
        let part_id = part_id(&updated_part);
        match &mut updated_part {
            Part::Text(text) => text.text = "updated".to_string(),
            other => panic!("unexpected part type: {other:?}"),
        }
        let stored_part = store
            .update_part(&session_id, &msg_id, &part_id, &updated_part)
            .await
            .unwrap()
            .unwrap();
        match stored_part {
            Part::Text(text) => assert_eq!(text.text, "updated"),
            other => panic!("unexpected part type: {other:?}"),
        }

        assert!(store
            .delete_part(&session_id, &msg_id, &part_id)
            .await
            .unwrap());
        let after_part_delete = store
            .get_message_with_parts(&session_id, &msg_id)
            .await
            .unwrap()
            .unwrap();
        assert!(after_part_delete.parts.is_empty());

        assert!(store.delete_message(&session_id, &msg_id).await.unwrap());
        assert!(store
            .get_message_with_parts(&session_id, &msg_id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn revert_to_message_deletes_later_messages_and_parts_and_updates_session() {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(tmp.path().to_path_buf()).await.unwrap();
        let session = store
            .create("revert", "project", &tmp.path().to_path_buf())
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();

        let (first_id, first) = user_message(&session_id);
        let (target_id, target) = user_message(&session_id);
        let (third_id, third) = user_message(&session_id);

        store.save_message(&session_id, &first).await.unwrap();
        store
            .save_text_part(&session_id, &first_id, "first")
            .await
            .unwrap();
        store.save_message(&session_id, &target).await.unwrap();
        store
            .save_text_part(&session_id, &target_id, "target")
            .await
            .unwrap();
        store.save_message(&session_id, &third).await.unwrap();
        store
            .save_text_part(&session_id, &third_id, "third")
            .await
            .unwrap();

        let before = store.get(&session_id).await.unwrap().unwrap();
        let updated = store
            .revert_to_message(&session_id, &target_id, None)
            .await
            .unwrap()
            .unwrap();

        let messages = store.get_messages(&session_id).await.unwrap();
        let remaining_ids = messages.iter().map(message_id).collect::<Vec<_>>();
        assert_eq!(
            remaining_ids,
            vec![message_id(&first), target_id.to_string()]
        );

        let parts = store.get_parts_by_session(&session_id).await.unwrap();
        assert!(parts.contains_key(&target_id.to_string()));
        assert!(!parts.contains_key(&third_id.to_string()));
        assert!(updated.time_updated >= before.time_updated);
        let revert: serde_json::Value =
            serde_json::from_str(updated.revert.as_deref().unwrap()).unwrap();
        assert_eq!(
            revert,
            serde_json::json!({
                "messageID": target_id.to_string(),
                "partID": null,
                "snapshot": null,
                "diff": null
            })
        );

        let cleared = store
            .clear_revert(&session_id)
            .await
            .unwrap()
            .expect("session should exist");
        assert!(cleared.revert.is_none());
    }
}
