//! Conversation compaction.
//!
//! When the session's running token usage approaches the model's context
//! ceiling we trigger a compaction: ask the provider to summarize the
//! current conversation, persist the summary as a synthetic User turn,
//! and bump `session.time_compacting` so `build_completion_messages`
//! drops everything older than the summary.
//!
//! Compaction is intentionally non-destructive: the old messages stay in
//! the database (so /api/session/:id/messages and revert can still see
//! them). They just stop appearing in the next provider call.

use anyhow::Result;
use std::sync::Arc;

use crate::id::SessionID;
use crate::provider::{CompletionMessage, CompletionRequest, ModelID, Provider};
use crate::session::SessionStore;

/// Run a compaction pass on `session_id`. Returns Ok(()) on success.
///
/// On failure (provider error, persistence error) we propagate the error
/// up — the caller is responsible for deciding whether to retry or
/// continue past it. We do **not** advance time_compacting unless the
/// summary was persisted, so a partial failure leaves the session in its
/// pre-compaction state.
pub async fn compact_session(
    store: &Arc<SessionStore>,
    session_id: &SessionID,
    provider: &Arc<dyn Provider>,
    model_id: &str,
) -> Result<()> {
    // 1. Rebuild the full conversation (already excludes pre-prior-compaction
    //    turns) so the summarizer sees only the live window.
    let history = crate::session::build_completion_messages(store, session_id).await?;
    if history.is_empty() {
        return Ok(());
    }

    // 2. Append a final instruction. Using a `user` role for the summary
    //    request keeps the provider call structurally simple (it's the
    //    same shape as every other call) and works across providers
    //    that don't accept a system message change mid-conversation.
    let mut messages = history;
    messages.push(CompletionMessage {
        role: "user".to_string(),
        content: SUMMARY_INSTRUCTION.to_string(),
        tool_calls: None,
        tool_call_id: None,
    });

    let request = CompletionRequest {
        model: ModelID::new(model_id),
        messages,
        system: Some(SUMMARY_SYSTEM.to_string()),
        tools: vec![],
        max_tokens: Some(2048),
        temperature: Some(0.0),
        top_p: None,
        stop_sequences: None,
    };

    let response = provider
        .complete(request)
        .await
        .map_err(|e| anyhow::anyhow!("compaction summarizer failed: {}", e))?;

    let summary = response.content.trim().to_string();
    if summary.is_empty() {
        anyhow::bail!("compaction summarizer returned empty summary");
    }

    // 3. Persist the summary as a synthetic User message. Boundary marker
    //    is the message's time_created — anything strictly older will be
    //    filtered out by build_completion_messages on subsequent turns.
    let boundary_ts = chrono::Utc::now().timestamp_millis();
    let summary_message_id = crate::id::MessageID::new();
    let summary_msg = crate::message::UserMessage {
        id: summary_message_id.clone(),
        session_id: session_id.clone(),
        role: "user".to_string(),
        time: crate::message::UserTime { created: boundary_ts },
        format: None,
        summary: None,
        agent: "build".to_string(),
        model: crate::message::ModelRef {
            provider_id: String::new(),
            model_id: model_id.to_string(),
            variant: None,
        },
        system: None,
        tools: None,
    };
    store
        .save_message(session_id, &crate::message::Message::User(summary_msg))
        .await?;
    store
        .save_text_part(
            session_id,
            &summary_message_id,
            &format!("{}{}", COMPACTION_PREFIX, summary),
        )
        .await?;

    // 4. Move the boundary forward. From now on history rebuilds will
    //    include only messages with time_created >= boundary_ts (i.e. the
    //    summary itself and anything that comes after).
    store
        .set_time_compacting(session_id, boundary_ts)
        .await?;

    Ok(())
}

/// Prefix on the summary text part so a human inspecting the database
/// can tell at a glance that the message is a compaction artifact rather
/// than a real user turn.
pub const COMPACTION_PREFIX: &str = "[Conversation summary — prior turns compacted]\n\n";

const SUMMARY_SYSTEM: &str =
    "You are a conversation summarizer. You will be shown the full transcript of an \
     ongoing coding-agent session. Produce a concise but complete summary that captures: \
     (a) the user's overall goal; (b) decisions and constraints established; (c) files \
     and tools touched, with their final state; (d) any open questions or pending work. \
     Output plain prose — no markdown headers, no list bullets, no preamble.";

const SUMMARY_INSTRUCTION: &str =
    "Now produce the summary as instructed in the system prompt. Output ONLY the summary text.";
