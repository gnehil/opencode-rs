use axum::{
    extract::{Json, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::bus::event::Event;
use crate::id::SessionID;
use crate::tui::control::TuiRequest;

const DEFAULT_TOAST_DURATION: u64 = 5_000;

#[derive(Deserialize)]
pub struct TuiPromptBody {
    #[serde(alias = "prompt")]
    text: String,
}

pub async fn append_prompt(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TuiPromptBody>,
) -> Json<bool> {
    state.event_bus.publish(Event::tui_prompt_append(body.text));
    Json(true)
}

pub async fn submit_prompt(State(state): State<Arc<AppState>>) -> Json<bool> {
    publish_command(&state, Some("prompt.submit"));
    Json(true)
}

pub async fn clear_prompt(State(state): State<Arc<AppState>>) -> Json<bool> {
    publish_command(&state, Some("prompt.clear"));
    Json(true)
}

#[derive(Deserialize)]
pub struct TuiCommandBody {
    command: String,
}

pub async fn execute_command(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TuiCommandBody>,
) -> Json<bool> {
    publish_command(&state, command_alias(&body.command));
    Json(true)
}

#[derive(Deserialize)]
pub struct TuiToastBody {
    title: Option<String>,
    message: String,
    #[serde(default = "default_toast_variant")]
    variant: String,
    #[serde(default = "default_toast_duration")]
    duration: u64,
}

pub async fn show_toast(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TuiToastBody>,
) -> Json<bool> {
    state.event_bus.publish(Event::tui_toast_show(
        body.title,
        body.message,
        body.variant,
        body.duration,
    ));
    Json(true)
}

pub async fn open_help(State(state): State<Arc<AppState>>) -> Json<bool> {
    publish_command(&state, Some("help.show"));
    Json(true)
}

pub async fn open_sessions(State(state): State<Arc<AppState>>) -> Json<bool> {
    publish_command(&state, Some("session.list"));
    Json(true)
}

pub async fn open_themes(State(state): State<Arc<AppState>>) -> Json<bool> {
    publish_command(&state, Some("session.list"));
    Json(true)
}

pub async fn open_models(State(state): State<Arc<AppState>>) -> Json<bool> {
    publish_command(&state, Some("model.list"));
    Json(true)
}

#[derive(Deserialize)]
pub struct TuiPublishBody {
    #[serde(rename = "type")]
    event_type: String,
    properties: Value,
}

pub async fn publish(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TuiPublishBody>,
) -> Result<Json<bool>, StatusCode> {
    publish_tui_event(&state, &body.event_type, body.properties)?;
    Ok(Json(true))
}

#[derive(Deserialize)]
pub struct TuiSessionBody {
    #[serde(rename = "sessionID", alias = "session_id", alias = "sessionId")]
    session_id: String,
}

pub async fn select_session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TuiSessionBody>,
) -> Result<Json<bool>, StatusCode> {
    let session_id = SessionID::parse(&body.session_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    if store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    state
        .event_bus
        .publish(Event::tui_session_select(body.session_id));
    Ok(Json(true))
}

pub async fn tui_next(State(state): State<Arc<AppState>>) -> Result<Json<TuiRequest>, StatusCode> {
    let request = state
        .tui_control
        .next_request()
        .await
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(request))
}

pub async fn tui_response(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<Json<bool>, StatusCode> {
    state
        .tui_control
        .submit_response(body)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(true))
}

fn publish_tui_event(
    state: &AppState,
    event_type: &str,
    properties: Value,
) -> Result<(), StatusCode> {
    match event_type {
        "tui.prompt.append" => {
            let body: TuiPromptBody =
                serde_json::from_value(properties).map_err(|_| StatusCode::BAD_REQUEST)?;
            state.event_bus.publish(Event::tui_prompt_append(body.text));
        }
        "tui.command.execute" => {
            let body: TuiCommandPublishBody =
                serde_json::from_value(properties).map_err(|_| StatusCode::BAD_REQUEST)?;
            publish_command(state, body.command.as_deref());
        }
        "tui.toast.show" => {
            let body: TuiToastBody =
                serde_json::from_value(properties).map_err(|_| StatusCode::BAD_REQUEST)?;
            state.event_bus.publish(Event::tui_toast_show(
                body.title,
                body.message,
                body.variant,
                body.duration,
            ));
        }
        "tui.session.select" => {
            let body: TuiSessionBody =
                serde_json::from_value(properties).map_err(|_| StatusCode::BAD_REQUEST)?;
            state
                .event_bus
                .publish(Event::tui_session_select(body.session_id));
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    }
    Ok(())
}

#[derive(Deserialize)]
struct TuiCommandPublishBody {
    command: Option<String>,
}

fn publish_command(state: &AppState, command: Option<&str>) {
    state
        .event_bus
        .publish(Event::tui_command_execute(command.map(str::to_string)));
}

fn command_alias(command: &str) -> Option<&'static str> {
    match command {
        "session_new" => Some("session.new"),
        "session_share" => Some("session.share"),
        "session_interrupt" => Some("session.interrupt"),
        "session_compact" => Some("session.compact"),
        "messages_page_up" => Some("session.page.up"),
        "messages_page_down" => Some("session.page.down"),
        "messages_line_up" => Some("session.line.up"),
        "messages_line_down" => Some("session.line.down"),
        "messages_half_page_up" => Some("session.half.page.up"),
        "messages_half_page_down" => Some("session.half.page.down"),
        "messages_first" => Some("session.first"),
        "messages_last" => Some("session.last"),
        "agent_cycle" => Some("agent.cycle"),
        _ => None,
    }
}

fn default_toast_variant() -> String {
    "info".to_string()
}

fn default_toast_duration() -> u64 {
    DEFAULT_TOAST_DURATION
}
