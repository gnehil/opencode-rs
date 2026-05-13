use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::broadcast;
use tracing::debug;

use super::event::{Event, HandlerId};

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type EventHandler = Box<dyn Fn(&Event) -> BoxFuture + Send + Sync>;

struct HandlerEntry {
    id: HandlerId,
    event_type: String,
    handler: EventHandler,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn generate_id() -> HandlerId {
    NEXT_ID.fetch_add(1, Ordering::Relaxed).to_string()
}

pub struct EventBus {
    handlers: Arc<RwLock<Vec<HandlerEntry>>>,
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel::<Event>(1024);
        EventBus {
            handlers: Arc::new(RwLock::new(Vec::new())),
            tx,
        }
    }

    pub fn subscribe(
        &self,
        event_type: impl Into<String>,
        handler: impl Fn(&Event) -> BoxFuture + Send + Sync + 'static,
    ) -> HandlerId {
        let id = generate_id();
        let event_type = event_type.into();

        self.handlers.write().push(HandlerEntry {
            id: id.clone(),
            event_type: event_type.clone(),
            handler: Box::new(handler),
        });

        id
    }

    pub fn subscribe_once(
        &self,
        event_type: impl Into<String>,
        handler: impl Fn(&Event) -> BoxFuture + Send + Sync + 'static,
    ) -> HandlerId {
        let bus = self.clone();
        let event_type_str = event_type.into();

        self.subscribe(
            event_type_str.clone(),
            Box::new(move |event: &Event| -> BoxFuture {
                let fut = handler(event);
                let et = event_type_str.clone();
                let b = bus.clone();
                Box::pin(async move {
                    fut.await;
                    b.unsubscribe_matching(&et);
                })
            }),
        )
    }

    pub fn unsubscribe(&self, handler_id: &HandlerId) -> bool {
        let mut handlers = self.handlers.write();
        let len_before = handlers.len();
        handlers.retain(|h| h.id != *handler_id);
        handlers.len() < len_before
    }

    pub fn unsubscribe_all(&self) {
        self.handlers.write().clear();
    }

    pub fn unsubscribe_matching(&self, event_type: &str) {
        self.handlers.write().retain(|h| h.event_type != event_type);
    }

    pub fn publish(&self, event: Event) {
        let event_type = event.type_name();

        let handlers = self.handlers.read();
        let matching: Vec<_> = handlers
            .iter()
            .filter(|h| h.event_type == "*" || h.event_type == event_type)
            .collect();

        if matching.is_empty() {
            debug!(event = event_type, "no handlers for event");
        }

        for entry in matching {
            let fut = (entry.handler)(&event);
            tokio::spawn(fut);
        }

        let _ = self.tx.send(event);
    }

    pub fn listener(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn handler_count(&self) -> usize {
        self.handlers.read().len()
    }

    pub fn handler_count_for(&self, event_type: &str) -> usize {
        self.handlers
            .read()
            .iter()
            .filter(|h| h.event_type == "*" || h.event_type == event_type)
            .count()
    }
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        EventBus {
            handlers: Arc::clone(&self.handlers),
            tx: self.tx.clone(),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_subscribe_and_publish() {
        let bus = EventBus::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);

        bus.subscribe(
            "session.create",
            Box::new(move |event: &Event| -> BoxFuture {
                let tx = tx.clone();
                let id = event.id();
                Box::pin(async move {
                    let _ = tx.send(id).await;
                })
            }),
        );

        bus.publish(Event::session_create("s1"));
        let id = rx.recv().await.unwrap();
        assert_eq!(id, "s1");
    }

    #[tokio::test]
    async fn test_unsubscribe() {
        let bus = EventBus::new();

        let _id = bus.subscribe(
            "session.create",
            Box::new(|_: &Event| -> BoxFuture { Box::pin(async {}) }),
        );
        assert_eq!(bus.handler_count(), 1);

        bus.unsubscribe_all();
        assert_eq!(bus.handler_count(), 0);
    }

    #[tokio::test]
    async fn test_wildcard_subscription() {
        let bus = EventBus::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);

        bus.subscribe(
            "*",
            Box::new(move |event: &Event| -> BoxFuture {
                let tx = tx.clone();
                let name = event.type_name().to_string();
                Box::pin(async move {
                    let _ = tx.send(name).await;
                })
            }),
        );

        bus.publish(Event::session_create("s1"));
        bus.publish(Event::mcp_connected("server1"));

        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        let mut received = Vec::new();
        while let Ok(id) = rx.try_recv() {
            received.push(id);
        }
        assert_eq!(received.len(), 2);
    }
}
