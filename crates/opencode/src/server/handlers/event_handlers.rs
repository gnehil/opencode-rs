use axum::response::sse::{Event, KeepAlive, Sse};
use axum::extract::{State, Query};
use futures::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use serde::Deserialize;
use tokio::sync::broadcast;

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
    let bus: EventBus = state.event_bus.clone();
    let filter_sid = query.session_id;

    let stream = async_stream::stream! {
        let mut rx = bus.listener();
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let session_id = event.session_id();
                    if let Some(filter) = &filter_sid {
                        if &session_id != filter {
                            continue;
                        }
                    }

                    let event_type = event.type_name();
                    let data = match &event {
                        BusEvent::SessionCreate(e) => serde_json::json!({
                            "type": "session.create",
                            "session_id": e.session_id,
                        }),
                        BusEvent::MessageCreate(e) => serde_json::json!({
                            "type": "message.create",
                            "session_id": e.session_id,
                            "message_id": e.message_id,
                            "role": e.role,
                        }),
                        BusEvent::MessageStream(e) => serde_json::json!({
                            "type": "message.stream",
                            "session_id": e.session_id,
                            "message_id": e.message_id,
                            "delta": e.delta,
                        }),
                        BusEvent::ToolStart(e) => serde_json::json!({
                            "type": "tool.start",
                            "session_id": e.session_id,
                            "tool": e.tool_name,
                            "input": e.tool_input,
                        }),
                        BusEvent::ToolComplete(e) => serde_json::json!({
                            "type": "tool.complete",
                            "session_id": e.session_id,
                            "tool": e.tool_name,
                            "output": e.tool_output,
                        }),
                        BusEvent::ToolError(e) => serde_json::json!({
                            "type": "tool.error",
                            "session_id": e.session_id,
                            "tool": e.tool_name,
                            "error": e.error,
                        }),
                        BusEvent::McpConnected(e) => serde_json::json!({
                            "type": "mcp.connected",
                            "server": e.server_name,
                        }),
                        BusEvent::McpDisconnected(e) => serde_json::json!({
                            "type": "mcp.disconnected",
                            "server": e.server_name,
                        }),
                        _ => serde_json::json!({
                            "type": event_type,
                            "session_id": session_id,
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
