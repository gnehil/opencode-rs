//! Conversation history reconstruction from persisted SessionStore state.
//!
//! Both `session::processor` and `acp::agent` need to send a complete
//! conversation to the provider on every turn: prior user messages, prior
//! assistant messages (with their tool_use calls), and the tool_result
//! payloads. Walking `SessionStore::get_messages_with_parts` and flattening
//! the result lives here so both call sites share semantics.
//!
//! The output shape is `Vec<CompletionMessage>` — the role/content/tool_calls
//! / tool_call_id schema. Providers are responsible for serializing this into
//! their wire format (anthropic tool_use/tool_result content blocks, openai
//! tool_calls + role=tool messages, etc.).

use anyhow::Result;
use base64::Engine;
use serde_json::Value;

use crate::id::SessionID;
use crate::message::{FilePartSource, Message, Part, ToolState};
use crate::provider::CompletionMessage;
use crate::session::SessionStore;

/// Read all persisted messages for `session_id` and produce a normalized
/// conversation for the next provider call.
///
/// For each persisted assistant message we emit:
///   * one assistant `CompletionMessage` with concatenated text + any
///     tool_calls (encoded as JSON values matching OpenAI's tool_calls
///     schema; providers can reinterpret).
///   * one `role: "tool"` message per completed/errored tool call so the
///     model can see the result. Pending/Running tool calls are skipped —
///     we only replay finalized turns.
pub async fn build_completion_messages(
    store: &SessionStore,
    session_id: &SessionID,
) -> Result<Vec<CompletionMessage>> {
    // Honour compaction: if a boundary timestamp is set, drop every
    // message that originated before it. The compaction summary itself
    // was persisted with time_created == boundary so it survives.
    let compaction_boundary: Option<i64> = store
        .get(session_id)
        .await?
        .and_then(|row| row.time_compacting);

    let with_parts = store.get_messages_with_parts(session_id).await?;
    let mut out = Vec::new();

    for wp in with_parts {
        if let Some(boundary) = compaction_boundary {
            let created = match &wp.info {
                Message::User(u) => u.time.created,
                Message::Assistant(a) => a.time.created,
            };
            if created < boundary {
                continue;
            }
        }

        match &wp.info {
            Message::User(_) => {
                let (text, images) = collect_user_content(&wp.parts);
                if !text.is_empty() || !images.is_empty() {
                    out.push(CompletionMessage {
                        role: "user".to_string(),
                        content: text,
                        tool_calls: None,
                        tool_call_id: None,
                        images,
                    });
                }
            }
            Message::Assistant(_) => {
                let text = collect_text(&wp.parts);
                let (tool_calls, tool_results) = collect_tool_parts(&wp.parts);

                // Some assistant turns are pure text, some are pure tool
                // calls, some are both. Emit a single assistant message
                // either way so the model's view of the turn is faithful.
                if !text.is_empty() || !tool_calls.is_empty() {
                    out.push(CompletionMessage {
                        role: "assistant".to_string(),
                        content: text,
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                        images: Vec::new(),
                    });
                }

                // Anthropic merges adjacent tool_results into a single user
                // message at the wire layer; for the canonical
                // CompletionMessage form we keep one entry per result so
                // OpenAI-shaped providers can render them as separate
                // role=tool messages. Anthropic's serializer collapses on
                // its end.
                let mut all_attachments: Vec<String> = Vec::new();
                for (call_id, output, images) in tool_results {
                    out.push(CompletionMessage {
                        role: "tool".to_string(),
                        content: output,
                        tool_calls: None,
                        tool_call_id: Some(call_id),
                        images: Vec::new(),
                    });
                    all_attachments.extend(images);
                }

                // If any tool returned image attachments, surface them
                // in a synthetic user message right after the
                // tool_results. This works for every provider: they
                // all accept images in user messages, and the model
                // sees the image immediately after the "look at this
                // file" tool result that produced it. The text content
                // is a hint so a text-only model isn't left wondering.
                if !all_attachments.is_empty() {
                    out.push(CompletionMessage {
                        role: "user".to_string(),
                        content: "[Tool returned image attachments.]".to_string(),
                        tool_calls: None,
                        tool_call_id: None,
                        images: all_attachments,
                    });
                }
            }
        }
    }
    Ok(out)
}

fn collect_text(parts: &[Part]) -> String {
    let mut chunks = Vec::new();
    for part in parts {
        if let Part::Text(t) = part {
            if t.ignored != Some(true) && !t.text.is_empty() {
                chunks.push(t.text.clone());
            }
        }
    }
    chunks.join("\n")
}

fn collect_user_content(parts: &[Part]) -> (String, Vec<String>) {
    let mut chunks = Vec::new();
    let mut images = Vec::new();
    let has_text = parts.iter().any(|part| matches!(part, Part::Text(_)));

    for part in parts {
        match part {
            Part::Text(t) => {
                if t.ignored != Some(true) && !t.text.is_empty() {
                    chunks.push(t.text.clone());
                }
            }
            Part::File(file) => {
                if file.mime.starts_with("image/") {
                    chunks.push(format!(
                        "[Attached {}: {}]",
                        file.mime,
                        file.filename.as_deref().unwrap_or("file")
                    ));
                    images.push(file.url.clone());
                } else if let Some(text) = file_text_content(file) {
                    chunks.push(text);
                } else {
                    chunks.push(format!(
                        "[Attached {}: {}]",
                        file.mime,
                        file.filename.as_deref().unwrap_or("file")
                    ));
                }
            }
            Part::Agent(agent) => {
                chunks.push(format!(
                    "Use the above message and context to generate a prompt and call the task tool with subagent: {}",
                    agent.name
                ));
            }
            Part::Subtask(task) => {
                chunks.push(format!(
                    "The following subtask was requested.\nDescription: {}\nAgent: {}\nPrompt: {}",
                    task.description, task.agent, task.prompt
                ));
            }
            Part::Compaction(_) if !has_text => chunks.push("What did we do so far?".to_string()),
            Part::Compaction(_) => {}
            _ => {}
        }
    }

    (chunks.join("\n"), images)
}

fn file_text_content(file: &crate::message::part::FilePart) -> Option<String> {
    if let Some(source_text) = file_source_text(&file.source) {
        return Some(source_text);
    }
    if file.mime != "text/plain" {
        return None;
    }
    decode_text_data_url(&file.url)
}

fn file_source_text(source: &Option<FilePartSource>) -> Option<String> {
    match source {
        Some(FilePartSource::File { text, .. })
        | Some(FilePartSource::Symbol { text, .. })
        | Some(FilePartSource::Resource { text, .. }) => Some(text.value.clone()),
        None => None,
    }
}

fn decode_text_data_url(url: &str) -> Option<String> {
    let after_data = url.strip_prefix("data:")?;
    let (meta, payload) = after_data.split_once(',')?;
    let lower = meta.to_ascii_lowercase();
    if !lower.starts_with("text/plain") {
        return None;
    }
    if lower.split(';').any(|part| part == "base64") {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .ok()?;
        return String::from_utf8(bytes).ok();
    }
    urlencoding::decode(payload)
        .ok()
        .map(|text| text.into_owned())
}

/// Walk the assistant's parts and return:
///   * `Vec<Value>` shaped like OpenAI's tool_calls array
///   * `Vec<(call_id, output_string, image_urls)>` for completed/errored
///     calls. `image_urls` are the data/https URLs of any attachments
///     the tool emitted (only Completed state can have them).
fn collect_tool_parts(parts: &[Part]) -> (Vec<Value>, Vec<(String, String, Vec<String>)>) {
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();

    for part in parts {
        if let Part::Tool(t) = part {
            let input = tool_state_input(&t.state);
            tool_calls.push(serde_json::json!({
                "id": t.call_id,
                "type": "function",
                "function": {
                    "name": t.tool,
                    "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string()),
                }
            }));
            if let Some(output) = tool_state_output(&t.state) {
                let images = tool_state_attachments(&t.state);
                tool_results.push((t.call_id.clone(), output, images));
            }
        }
    }

    (tool_calls, tool_results)
}

/// Return the image-bearing attachment URLs from a Completed tool state.
/// Errors/Pending/Running never carry attachments.
fn tool_state_attachments(state: &ToolState) -> Vec<String> {
    match state {
        ToolState::Completed(c) => c
            .attachments
            .as_ref()
            .map(|a| {
                a.iter()
                    .filter(|f| f.mime.starts_with("image/"))
                    .map(|f| f.url.clone())
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Return the input map for any ToolState variant (they all carry input).
pub fn tool_state_input(state: &ToolState) -> Value {
    let map = match state {
        ToolState::Pending(p) => &p.input,
        ToolState::Running(r) => &r.input,
        ToolState::Completed(c) => &c.input,
        ToolState::Error(e) => &e.input,
    };
    serde_json::to_value(map).unwrap_or(Value::Null)
}

/// Return the textual result for finished tool calls. Pending/Running are
/// considered "not yet observable" by the model and excluded.
pub fn tool_state_output(state: &ToolState) -> Option<String> {
    match state {
        ToolState::Completed(c) => Some(c.output.clone()),
        ToolState::Error(e) => Some(format!("Error: {}", e.error)),
        ToolState::Pending(_) | ToolState::Running(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SessionID;
    use crate::session::service::ToolPartResult;
    use crate::session::SessionStore;

    async fn store_with_tmp_data_dir() -> (SessionStore, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::new(tmp.path().to_path_buf()).await.unwrap();
        (store, tmp)
    }

    #[tokio::test]
    async fn full_tool_roundtrip_yields_assistant_then_tool_messages() {
        let (store, _tmp) = store_with_tmp_data_dir().await;

        // Set up a session.
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();

        // Persist a user turn.
        let user_id = crate::id::MessageID::new();
        let now = chrono::Utc::now().timestamp_millis();
        let user_msg = crate::message::UserMessage {
            id: user_id.clone(),
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime { created: now },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: crate::message::ModelRef {
                provider_id: "anthropic".to_string(),
                model_id: "m".to_string(),
                variant: None,
            },
            system: None,
            tools: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::User(user_msg))
            .await
            .unwrap();
        store
            .save_text_part(&session_id, &user_id, "list files")
            .await
            .unwrap();

        // Persist an assistant turn with a completed tool call.
        let assistant_id = crate::id::MessageID::new();
        let assistant_msg = crate::message::AssistantMessage {
            id: assistant_id.clone(),
            session_id: session_id.clone(),
            role: "assistant".to_string(),
            time: crate::message::AssistantTime {
                created: now,
                completed: Some(now),
            },
            error: None,
            parent_id: user_id.to_string(),
            model_id: "m".to_string(),
            provider_id: "anthropic".to_string(),
            mode: "default".to_string(),
            agent: "build".to_string(),
            path: crate::message::PathInfo {
                cwd: "/tmp".to_string(),
                root: "/".to_string(),
            },
            summary: None,
            cost: 0.0,
            tokens: crate::message::TokenUsage {
                input: 0.0,
                output: 0.0,
                reasoning: 0.0,
                total: None,
                cache: crate::message::CacheUsage {
                    read: 0.0,
                    write: 0.0,
                },
            },
            structured: None,
            variant: None,
            finish: None,
        };
        store
            .save_message(
                &session_id,
                &crate::message::Message::Assistant(assistant_msg),
            )
            .await
            .unwrap();
        store
            .save_text_part(&session_id, &assistant_id, "running ls")
            .await
            .unwrap();
        store
            .save_tool_part(
                &session_id,
                &assistant_id,
                "bash",
                "toolu_42",
                &serde_json::json!({"command": "ls"}),
                ToolPartResult::Completed {
                    output: "a\nb".to_string(),
                    attachments: vec![],
                },
            )
            .await
            .unwrap();

        // Rebuild.
        let history = build_completion_messages(&store, &session_id)
            .await
            .unwrap();

        // Expect: user, assistant (with tool_calls), tool (the result).
        assert_eq!(history.len(), 3, "{:#?}", history);

        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "list files");

        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "running ls");
        let calls = history[1]
            .tool_calls
            .as_ref()
            .expect("assistant should carry tool_calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "toolu_42");
        assert_eq!(calls[0]["function"]["name"], "bash");

        assert_eq!(history[2].role, "tool");
        assert_eq!(history[2].tool_call_id.as_deref(), Some("toolu_42"));
        assert_eq!(history[2].content, "a\nb");
    }

    #[tokio::test]
    async fn compaction_boundary_drops_older_messages() {
        let (store, _tmp) = store_with_tmp_data_dir().await;
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();

        // Save two user messages at different timestamps.
        let m1_id = crate::id::MessageID::new();
        let m1 = crate::message::UserMessage {
            id: m1_id.clone(),
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime { created: 1_000 },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: crate::message::ModelRef {
                provider_id: "anthropic".to_string(),
                model_id: "m".to_string(),
                variant: None,
            },
            system: None,
            tools: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::User(m1))
            .await
            .unwrap();
        store
            .save_text_part(&session_id, &m1_id, "OLD MESSAGE")
            .await
            .unwrap();

        let m2_id = crate::id::MessageID::new();
        let m2 = crate::message::UserMessage {
            id: m2_id.clone(),
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime { created: 5_000 },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: crate::message::ModelRef {
                provider_id: "anthropic".to_string(),
                model_id: "m".to_string(),
                variant: None,
            },
            system: None,
            tools: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::User(m2))
            .await
            .unwrap();
        store
            .save_text_part(&session_id, &m2_id, "NEW MESSAGE")
            .await
            .unwrap();

        // No boundary yet — both are visible.
        let before = build_completion_messages(&store, &session_id)
            .await
            .unwrap();
        assert_eq!(before.len(), 2, "{:?}", before);

        // Set the boundary between the two messages.
        store.set_time_compacting(&session_id, 4_000).await.unwrap();

        let after = build_completion_messages(&store, &session_id)
            .await
            .unwrap();
        assert_eq!(after.len(), 1, "{:?}", after);
        assert_eq!(after[0].content, "NEW MESSAGE");
    }

    #[tokio::test]
    async fn tool_image_attachment_becomes_user_message_with_images() {
        let (store, _tmp) = store_with_tmp_data_dir().await;
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();

        // user → assistant(tool_call) → tool_completed_with_image
        let user_id = crate::id::MessageID::new();
        let user = crate::message::UserMessage {
            id: user_id.clone(),
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime {
                created: chrono::Utc::now().timestamp_millis(),
            },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: crate::message::ModelRef {
                provider_id: "anthropic".to_string(),
                model_id: "m".to_string(),
                variant: None,
            },
            system: None,
            tools: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::User(user))
            .await
            .unwrap();
        store
            .save_text_part(&session_id, &user_id, "show me the logo")
            .await
            .unwrap();

        let asst_id = crate::id::MessageID::new();
        let asst = crate::message::AssistantMessage {
            id: asst_id.clone(),
            session_id: session_id.clone(),
            role: "assistant".to_string(),
            time: crate::message::AssistantTime {
                created: chrono::Utc::now().timestamp_millis(),
                completed: None,
            },
            error: None,
            parent_id: user_id.to_string(),
            model_id: "m".to_string(),
            provider_id: "anthropic".to_string(),
            mode: "default".to_string(),
            agent: "build".to_string(),
            path: crate::message::PathInfo {
                cwd: "/tmp".to_string(),
                root: "/".to_string(),
            },
            summary: None,
            cost: 0.0,
            tokens: crate::message::TokenUsage {
                input: 0.0,
                output: 0.0,
                reasoning: 0.0,
                total: None,
                cache: crate::message::CacheUsage {
                    read: 0.0,
                    write: 0.0,
                },
            },
            structured: None,
            variant: None,
            finish: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::Assistant(asst))
            .await
            .unwrap();

        // Persist a tool part with one image attachment.
        let img = crate::message::part::FilePart {
            id: crate::id::PartID::new(),
            session_id: session_id.clone(),
            message_id: asst_id.clone(),
            mime: "image/png".to_string(),
            filename: Some("logo.png".to_string()),
            url: "data:image/png;base64,XYZ".to_string(),
            source: None,
        };
        store
            .save_tool_part(
                &session_id,
                &asst_id,
                "read",
                "call_7",
                &serde_json::json!({"filePath": "/tmp/logo.png"}),
                ToolPartResult::Completed {
                    output: "Read image".to_string(),
                    attachments: vec![img],
                },
            )
            .await
            .unwrap();

        let history = build_completion_messages(&store, &session_id)
            .await
            .unwrap();
        // user, assistant(tool_call), tool, user(synthetic with image).
        assert_eq!(history.len(), 4, "{:#?}", history);
        assert_eq!(history[2].role, "tool");
        assert!(history[2].images.is_empty());
        assert_eq!(history[3].role, "user");
        assert_eq!(history[3].images.len(), 1);
        assert_eq!(history[3].images[0], "data:image/png;base64,XYZ");
    }

    #[tokio::test]
    async fn user_file_part_becomes_user_message_with_images() {
        let (store, _tmp) = store_with_tmp_data_dir().await;
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let user_id = crate::id::MessageID::new();
        let user = crate::message::UserMessage {
            id: user_id,
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime {
                created: chrono::Utc::now().timestamp_millis(),
            },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: crate::message::ModelRef {
                provider_id: "anthropic".to_string(),
                model_id: "m".to_string(),
                variant: None,
            },
            system: None,
            tools: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::User(user))
            .await
            .unwrap();
        store
            .save_part(&crate::message::Part::Text(
                crate::message::part::TextPart {
                    id: crate::id::PartID::new(),
                    session_id: session_id.clone(),
                    message_id: user_id,
                    text: "describe this image".to_string(),
                    synthetic: None,
                    ignored: None,
                    time: None,
                    metadata: None,
                },
            ))
            .await
            .unwrap();
        store
            .save_part(&crate::message::Part::File(
                crate::message::part::FilePart {
                    id: crate::id::PartID::new(),
                    session_id: session_id.clone(),
                    message_id: user_id,
                    mime: "image/png".to_string(),
                    filename: Some("diagram.png".to_string()),
                    url: "data:image/png;base64,ABC".to_string(),
                    source: None,
                },
            ))
            .await
            .unwrap();

        let history = build_completion_messages(&store, &session_id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1, "{:#?}", history);
        assert_eq!(history[0].role, "user");
        assert!(history[0].content.contains("describe this image"));
        assert_eq!(history[0].images, vec!["data:image/png;base64,ABC"]);
    }

    #[tokio::test]
    async fn user_agent_and_subtask_parts_are_visible_to_model() {
        let (store, _tmp) = store_with_tmp_data_dir().await;
        let session = store
            .create("t", "p", &std::path::PathBuf::from("/tmp"))
            .await
            .unwrap();
        let session_id = SessionID::parse(&session.id).unwrap();
        let user_id = crate::id::MessageID::new();
        let user = crate::message::UserMessage {
            id: user_id,
            session_id: session_id.clone(),
            role: "user".to_string(),
            time: crate::message::UserTime {
                created: chrono::Utc::now().timestamp_millis(),
            },
            format: None,
            summary: None,
            agent: "build".to_string(),
            model: crate::message::ModelRef {
                provider_id: "anthropic".to_string(),
                model_id: "m".to_string(),
                variant: None,
            },
            system: None,
            tools: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::User(user))
            .await
            .unwrap();
        store
            .save_part(&crate::message::Part::Agent(
                crate::message::part::AgentPart {
                    id: crate::id::PartID::new(),
                    session_id: session_id.clone(),
                    message_id: user_id,
                    name: "reviewer".to_string(),
                    source: None,
                },
            ))
            .await
            .unwrap();
        store
            .save_part(&crate::message::Part::Subtask(
                crate::message::part::SubtaskPart {
                    id: crate::id::PartID::new(),
                    session_id: session_id.clone(),
                    message_id: user_id,
                    prompt: "inspect auth flow".to_string(),
                    description: "Review auth".to_string(),
                    agent: "reviewer".to_string(),
                    model: None,
                    command: Some("review".to_string()),
                },
            ))
            .await
            .unwrap();

        let history = build_completion_messages(&store, &session_id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1, "{:#?}", history);
        assert!(history[0].content.contains("reviewer"));
        assert!(history[0].content.contains("inspect auth flow"));
    }
}
