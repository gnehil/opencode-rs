use axum::response::sse::{Event, KeepAlive, Sse};
use axum::extract::{State, Query};
use futures::stream::{Stream, StreamExt};
use std::convert::Infallible;
use std::sync::Arc;
use serde::Deserialize;

use crate::bus::EventBus;
use crate::bus::event::Event as BusEvent;
use super::session_handlers::AppState;

#[derive(Deserialize)]
pub struct EventQuery {
    session_id: Option<String>,
}

pub async fn sse_events(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EventQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let bus = EventBus::new();
    
    let stream = async_stream::stream! {
        let mut rx = bus.listener();
        
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let session_id = event.session_id();
                    if let Some(filter_sid) = &query.session_id {
                        if session_id != *filter_sid {
                            continue;
                        }
                    }
                    
                    let event_type = event.type_name();
                    let data = match &event {
                        BusEvent::SessionCreate { id, .. } => serde_json::json!({
                            "type": "session.create",
                            "session_id": id
                        }),
                        BusEvent::MessageCreate { session_id, message_id, role } => serde_json::json!({
                            "type": "message.create",
                            "session_id": session_id,
                            "message_id": message_id,
                            "role": role.to_string()
                        }),
                        BusEvent::MessageStream { session_id, message_id, delta } => serde_json::json!({
                            "type": "message.stream",
                            "session_id": session_id,
                            "message_id": message_id,
                            "delta": delta
                        }),
                        BusEvent::ToolStart { session_id, tool_name, params } => serde_json::json!({
                            "type": "tool.start",
                            "session_id": session_id,
                            "tool": tool_name,
                            "params": params
                        }),
                        BusEvent::ToolComplete { session_id, tool_name, result } => serde_json::json!({
                            "type": "tool.complete",
                            "session_id": session_id,
                            "tool": tool_name,
                            "result": result
                        }),
                        BusEvent::ToolError { session_id, tool_name, error } => serde_json::json!({
                            "type": "tool.error",
                            "session_id": session_id,
                            "tool": tool_name,
                            "error": error
                        }),
                        BusEvent::McpConnected { server_name } => serde_json::json!({
                            "type": "mcp.connected",
                            "server": server_name
                        }),
                        BusEvent::McpDisconnected { server_name } => serde_json::json!({
                            "type": "mcp.disconnected",
                            "server": server_name
                        }),
                        _ => serde_json::json!({
                            "type": event_type,
                            "session_id": session_id
                        }),
                    };
                    
                    yield Ok(Event::default()
                        .event(event_type)
                        .data(data.to_string()));
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    };
    
    Sse::new(stream).keep_alive(KeepAlive::default())
}