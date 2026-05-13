use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

use super::session_handlers::AppState;
use crate::bus::event::Event as BusEvent;
use crate::bus::EventBus;

#[derive(Deserialize)]
pub struct EventQuery {
    session_id: Option<String>,
}

pub async fn sse_events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EventQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let bus: EventBus = state.event_bus.clone();
    let filter_sid = query.session_id;

    let stream = async_stream::stream! {
        let mut rx = bus.listener();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
        heartbeat.tick().await;

        yield Ok(sse_message_event(connected_payload()));

        loop {
            tokio::select! {
                received = rx.recv() => match received {
                    Ok(event) => {
                    let session_id = event.session_id();
                    if let Some(filter) = &filter_sid {
                        if &session_id != filter {
                            continue;
                        }
                    }

                    yield Ok(sse_message_event(bus_event_payload(&event)));
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                },
                _ = heartbeat.tick() => {
                    yield Ok(sse_message_event(heartbeat_payload()));
                }
            }
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn sse_message_event(payload: serde_json::Value) -> Event {
    Event::default().event("message").data(payload.to_string())
}

fn connected_payload() -> serde_json::Value {
    event_payload(
        uuid::Uuid::new_v4().to_string(),
        "server.connected",
        serde_json::json!({}),
    )
}

fn heartbeat_payload() -> serde_json::Value {
    event_payload(
        uuid::Uuid::new_v4().to_string(),
        "server.heartbeat",
        serde_json::json!({}),
    )
}

fn event_payload(
    id: impl Into<String>,
    event_type: impl Into<String>,
    properties: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "id": id.into(),
        "type": event_type.into(),
        "properties": properties,
    })
}

fn bus_event_payload(event: &BusEvent) -> serde_json::Value {
    event_payload(event.id(), event.type_name(), event_properties(event))
}

fn event_properties(event: &BusEvent) -> serde_json::Value {
    match event {
        BusEvent::SessionCreate(e) | BusEvent::SessionUpdate(e) | BusEvent::SessionDelete(e) => {
            let mut props = serde_json::Map::new();
            props.insert("sessionID".to_string(), serde_json::json!(e.session_id));
            insert_optional(&mut props, "metadata", &e.metadata);
            serde_json::Value::Object(props)
        }
        BusEvent::MessageCreate(e) => {
            let mut props = serde_json::Map::new();
            props.insert("sessionID".to_string(), serde_json::json!(e.session_id));
            props.insert("messageID".to_string(), serde_json::json!(e.message_id));
            props.insert("role".to_string(), serde_json::json!(e.role));
            insert_optional(&mut props, "content", &e.content);
            serde_json::Value::Object(props)
        }
        BusEvent::MessageStream(e) => serde_json::json!({
            "sessionID": e.session_id,
            "messageID": e.message_id,
            "delta": e.delta,
            "isDone": e.is_done,
        }),
        BusEvent::ToolStart(e) => serde_json::json!({
            "sessionID": e.session_id,
            "tool": e.tool_name,
            "input": e.tool_input,
            "toolCallID": e.tool_call_id,
        }),
        BusEvent::ToolComplete(e) => {
            let mut props = serde_json::Map::new();
            props.insert("sessionID".to_string(), serde_json::json!(e.session_id));
            props.insert("tool".to_string(), serde_json::json!(e.tool_name));
            props.insert("output".to_string(), e.tool_output.clone());
            props.insert("toolCallID".to_string(), serde_json::json!(e.tool_call_id));
            insert_optional(&mut props, "durationMS", &e.duration_ms);
            serde_json::Value::Object(props)
        }
        BusEvent::ToolError(e) => serde_json::json!({
            "sessionID": e.session_id,
            "tool": e.tool_name,
            "error": e.error,
            "toolCallID": e.tool_call_id,
        }),
        BusEvent::McpConnected(e) => {
            let mut props = serde_json::Map::new();
            props.insert("server".to_string(), serde_json::json!(e.server_name));
            insert_optional(&mut props, "url", &e.server_url);
            serde_json::Value::Object(props)
        }
        BusEvent::McpDisconnected(e) => {
            let mut props = serde_json::Map::new();
            props.insert("server".to_string(), serde_json::json!(e.server_name));
            insert_optional(&mut props, "reason", &e.reason);
            serde_json::Value::Object(props)
        }
        BusEvent::McpToolsChanged(e) => serde_json::json!({
            "server": e.server_name,
            "toolsAdded": e.tools_added,
            "toolsRemoved": e.tools_removed,
        }),
        BusEvent::PermissionAsked(e) => {
            let mut props = serde_json::Map::new();
            props.insert("id".to_string(), serde_json::json!(e.permission_id));
            props.insert("sessionID".to_string(), serde_json::json!(e.session_id));
            props.insert(
                "permission".to_string(),
                serde_json::json!(e.permission_type),
            );
            props.insert("metadata".to_string(), e.metadata.clone());
            insert_optional(&mut props, "toolCallID", &e.tool_call_id);
            insert_optional(&mut props, "tool", &e.tool_name);
            serde_json::Value::Object(props)
        }
        BusEvent::MessagePartUpdated(e) => serde_json::json!({
            "sessionID": e.session_id,
            "messageID": e.message_id,
            "partID": e.part_id,
            "partType": e.part_type,
            "part": e.part,
        }),
        BusEvent::MessagePartDelta(e) => serde_json::json!({
            "sessionID": e.session_id,
            "messageID": e.message_id,
            "partID": e.part_id,
            "field": e.field,
            "delta": e.delta,
        }),
    }
}

fn insert_optional<T: serde::Serialize>(
    props: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &Option<T>,
) {
    if let Some(value) = value {
        props.insert(key.to_string(), serde_json::json!(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::event::MessageRole;

    #[test]
    fn connected_payload_matches_opencode_event_envelope() {
        let payload = connected_payload();

        assert_eq!(payload["type"], "server.connected");
        assert_eq!(payload["properties"], serde_json::json!({}));
        assert!(payload["id"].as_str().is_some_and(|id| !id.is_empty()));
        assert!(payload.get("session_id").is_none());
    }

    #[test]
    fn bus_event_payload_uses_properties_envelope() {
        let event = BusEvent::message_create("ses_123", "msg_456", MessageRole::Assistant);
        let payload = bus_event_payload(&event);

        assert_eq!(payload["id"], "msg_456");
        assert_eq!(payload["type"], "message.create");
        assert_eq!(payload["properties"]["sessionID"], "ses_123");
        assert_eq!(payload["properties"]["messageID"], "msg_456");
        assert_eq!(payload["properties"]["role"], "assistant");
        assert!(payload.get("session_id").is_none());
        assert!(payload.get("message_id").is_none());
    }

    #[test]
    fn tool_event_payload_preserves_tool_call_id_and_duration() {
        let event = BusEvent::ToolComplete(crate::bus::event::ToolCompleteEvent {
            session_id: "ses_123".to_string(),
            tool_name: "bash".to_string(),
            tool_output: serde_json::json!({ "ok": true }),
            tool_call_id: "tool_789".to_string(),
            duration_ms: Some(42),
        });
        let payload = bus_event_payload(&event);

        assert_eq!(payload["id"], "tool_789");
        assert_eq!(payload["type"], "tool.complete");
        assert_eq!(payload["properties"]["sessionID"], "ses_123");
        assert_eq!(payload["properties"]["toolCallID"], "tool_789");
        assert_eq!(payload["properties"]["durationMS"], 42);
        assert_eq!(
            payload["properties"]["output"],
            serde_json::json!({ "ok": true })
        );
    }
}
