use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{oneshot, Mutex};

use crate::permission::{PermissionRequest, Reply};

struct PendingPermission {
    request: PermissionRequest,
    tx: oneshot::Sender<Reply>,
}

#[derive(Clone, Default)]
pub struct PermissionBroker {
    inner: Arc<Mutex<HashMap<String, PendingPermission>>>,
}

impl PermissionBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, request: PermissionRequest) -> oneshot::Receiver<Reply> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .lock()
            .await
            .insert(request.id.to_string(), PendingPermission { request, tx });
        rx
    }

    pub async fn pending(&self, session_id: Option<&str>) -> Vec<PermissionRequest> {
        self.inner
            .lock()
            .await
            .values()
            .filter(|pending| {
                session_id
                    .map(|id| pending.request.session_id.to_string() == id)
                    .unwrap_or(true)
            })
            .map(|pending| pending.request.clone())
            .collect()
    }

    pub async fn reply(&self, request_id: &str, reply: Reply) -> bool {
        match self.inner.lock().await.remove(request_id) {
            Some(pending) => {
                let _ = pending.tx.send(reply);
                true
            }
            None => false,
        }
    }

    pub async fn remove(&self, request_id: &str) -> bool {
        self.inner.lock().await.remove(request_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SessionID;
    use crate::permission::PermissionID;

    fn request(session_id: SessionID) -> PermissionRequest {
        PermissionRequest {
            id: PermissionID::new(),
            session_id,
            permission: "bash".to_string(),
            patterns: vec!["git status".to_string()],
            metadata: HashMap::new(),
            always: vec![],
            tool: None,
        }
    }

    #[tokio::test]
    async fn pending_and_reply_resolves_waiter() {
        let broker = PermissionBroker::new();
        let session_id = SessionID::new();
        let request = request(session_id.clone());
        let request_id = request.id.to_string();

        let rx = broker.register(request).await;
        assert_eq!(broker.pending(Some(&session_id.to_string())).await.len(), 1);

        assert!(broker.reply(&request_id, Reply::Once).await);
        assert_eq!(rx.await.unwrap(), Reply::Once);
        assert!(broker
            .pending(Some(&session_id.to_string()))
            .await
            .is_empty());
    }
}
