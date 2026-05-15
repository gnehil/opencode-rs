//! Interactive `question` tool support.
//!
//! Mirrors TypeScript `question/index.ts`: an in-process broker keyed by
//! `request_id` holds a pending question until a client replies via the HTTP
//! API. The tool side calls [`QuestionBroker::ask`] and blocks on the
//! returned receiver until either an `answers` array arrives or the request
//! is rejected.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex};
use ulid::Ulid;

use crate::id::SessionID;

/// Question option mirrors the TS `Option` schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A single question prompt sent to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionInfo {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<bool>,
}

/// A pending question request. Shape matches the TS `Request` schema (modulo
/// the `tool` field which is informational only — we do not persist it).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionRequest {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub questions: Vec<QuestionInfo>,
}

/// Reply payload for `POST /question/:id/reply`. Each user question can
/// have multiple selected labels, so the answers form a nested array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionReply {
    pub answers: Vec<Vec<String>>,
}

/// Outcome of a question broker `ask` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionOutcome {
    Replied(Vec<Vec<String>>),
    Rejected,
}

struct Pending {
    request: QuestionRequest,
    tx: oneshot::Sender<QuestionOutcome>,
}

/// Holds in-flight question requests and lets the tool block on a reply.
#[derive(Clone, Default)]
pub struct QuestionBroker {
    inner: Arc<Mutex<HashMap<String, Pending>>>,
}

impl QuestionBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a fresh question request and return a one-shot receiver the
    /// caller awaits for the user's reply.
    pub async fn ask(
        &self,
        session_id: &SessionID,
        questions: Vec<QuestionInfo>,
    ) -> (String, oneshot::Receiver<QuestionOutcome>) {
        let id = Ulid::new().to_string();
        let (tx, rx) = oneshot::channel();
        let request = QuestionRequest {
            id: id.clone(),
            session_id: session_id.to_string(),
            questions,
        };
        self.inner
            .lock()
            .await
            .insert(id.clone(), Pending { request, tx });
        (id, rx)
    }

    /// List pending questions, optionally filtered by session.
    pub async fn pending(&self, session_id: Option<&str>) -> Vec<QuestionRequest> {
        self.inner
            .lock()
            .await
            .values()
            .filter(|pending| {
                session_id
                    .map(|id| pending.request.session_id == id)
                    .unwrap_or(true)
            })
            .map(|pending| pending.request.clone())
            .collect()
    }

    /// Resolve a request with user answers. Returns true when the request id
    /// matched a pending entry.
    pub async fn reply(&self, request_id: &str, answers: Vec<Vec<String>>) -> bool {
        if let Some(pending) = self.inner.lock().await.remove(request_id) {
            let _ = pending.tx.send(QuestionOutcome::Replied(answers));
            true
        } else {
            false
        }
    }

    /// Reject a question request — the caller's `ask` future resolves with
    /// `QuestionOutcome::Rejected`.
    pub async fn reject(&self, request_id: &str) -> bool {
        if let Some(pending) = self.inner.lock().await.remove(request_id) {
            let _ = pending.tx.send(QuestionOutcome::Rejected);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(label: &str) -> QuestionOption {
        QuestionOption {
            label: label.to_string(),
            description: None,
        }
    }

    #[tokio::test]
    async fn ask_and_reply_resolves_the_waiter() {
        let broker = QuestionBroker::new();
        let session_id = SessionID::new();
        let (id, rx) = broker
            .ask(
                &session_id,
                vec![QuestionInfo {
                    question: "Continue?".to_string(),
                    header: None,
                    options: vec![opt("yes"), opt("no")],
                    multiple: None,
                    custom: None,
                }],
            )
            .await;
        assert_eq!(broker.pending(None).await.len(), 1);
        assert!(broker.reply(&id, vec![vec!["yes".to_string()]]).await);
        assert_eq!(
            rx.await.unwrap(),
            QuestionOutcome::Replied(vec![vec!["yes".to_string()]])
        );
        assert!(broker.pending(None).await.is_empty());
    }

    #[tokio::test]
    async fn rejecting_a_request_unblocks_the_waiter() {
        let broker = QuestionBroker::new();
        let session_id = SessionID::new();
        let (id, rx) = broker
            .ask(
                &session_id,
                vec![QuestionInfo {
                    question: "?".to_string(),
                    header: None,
                    options: vec![],
                    multiple: None,
                    custom: None,
                }],
            )
            .await;
        assert!(broker.reject(&id).await);
        assert_eq!(rx.await.unwrap(), QuestionOutcome::Rejected);
    }

    #[tokio::test]
    async fn pending_filters_by_session() {
        let broker = QuestionBroker::new();
        let s1 = SessionID::new();
        let s2 = SessionID::new();
        broker.ask(&s1, vec![]).await;
        broker.ask(&s2, vec![]).await;
        assert_eq!(broker.pending(Some(&s1.to_string())).await.len(), 1);
        assert_eq!(broker.pending(None).await.len(), 2);
    }
}
