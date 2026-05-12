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
use serde_json::Value;

use crate::id::SessionID;
use crate::message::{Message, Part, ToolState};
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
    let with_parts = store.get_messages_with_parts(session_id).await?;
    let mut out = Vec::new();

    for wp in with_parts {
        match &wp.info {
            Message::User(_) => {
                let text = collect_text(&wp.parts);
                if !text.is_empty() {
                    out.push(CompletionMessage {
                        role: "user".to_string(),
                        content: text,
                        tool_calls: None,
                        tool_call_id: None,
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
                        tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                        tool_call_id: None,
                    });
                }

                // Anthropic merges adjacent tool_results into a single user
                // message at the wire layer; for the canonical
                // CompletionMessage form we keep one entry per result so
                // OpenAI-shaped providers can render them as separate
                // role=tool messages. Anthropic's serializer collapses on
                // its end.
                for (call_id, output) in tool_results {
                    out.push(CompletionMessage {
                        role: "tool".to_string(),
                        content: output,
                        tool_calls: None,
                        tool_call_id: Some(call_id),
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
            if !t.text.is_empty() {
                chunks.push(t.text.clone());
            }
        }
    }
    chunks.join("\n")
}

/// Walk the assistant's parts and return:
///   * `Vec<Value>` shaped like OpenAI's tool_calls array
///   * `Vec<(call_id, output_string)>` for completed/errored calls
fn collect_tool_parts(parts: &[Part]) -> (Vec<Value>, Vec<(String, String)>) {
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
                tool_results.push((t.call_id.clone(), output));
            }
        }
    }

    (tool_calls, tool_results)
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
                cache: crate::message::CacheUsage { read: 0.0, write: 0.0 },
            },
            structured: None,
            variant: None,
            finish: None,
        };
        store
            .save_message(&session_id, &crate::message::Message::Assistant(assistant_msg))
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
                ToolPartResult::Completed { output: "a\nb".to_string() },
            )
            .await
            .unwrap();

        // Rebuild.
        let history = build_completion_messages(&store, &session_id).await.unwrap();

        // Expect: user, assistant (with tool_calls), tool (the result).
        assert_eq!(history.len(), 3, "{:#?}", history);

        assert_eq!(history[0].role, "user");
        assert_eq!(history[0].content, "list files");

        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[1].content, "running ls");
        let calls = history[1].tool_calls.as_ref().expect("assistant should carry tool_calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "toolu_42");
        assert_eq!(calls[0]["function"]["name"], "bash");

        assert_eq!(history[2].role, "tool");
        assert_eq!(history[2].tool_call_id.as_deref(), Some("toolu_42"));
        assert_eq!(history[2].content, "a\nb");
    }
}
