use axum::{
    extract::{Json, Path, Query, State},
    http::StatusCode,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::id::SessionID;
use crate::message::{Message, Part, WithParts};
use crate::storage::SessionRow;

const DEFAULT_LIMIT: usize = 50;

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageQuery {
    limit: Option<usize>,
    order: Option<String>,
    cursor: Option<String>,
    roots: Option<bool>,
    start: Option<i64>,
    search: Option<String>,
    path: Option<String>,
    directory: Option<String>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    roots: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    search: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorPayload {
    id: String,
    time: i64,
    order: String,
    direction: String,
    #[serde(flatten)]
    filters: CursorFilters,
}

pub async fn list_sessions_v2(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PageQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if query.cursor.is_some() && has_cursor_filter(&query) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let decoded = query.cursor.as_deref().map(decode_cursor).transpose()?;
    if let Some(decoded) = decoded.as_ref() {
        if query.directory.is_some() && query.directory != decoded.filters.directory {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let filters = decoded
        .as_ref()
        .map(|cursor| cursor.filters.clone())
        .unwrap_or_else(|| CursorFilters {
            directory: query.directory.clone(),
            path: query.path.clone(),
            roots: query.roots,
            start: query.start,
            search: query.search.clone(),
        });
    let order = decoded
        .as_ref()
        .map(|cursor| cursor.order.as_str())
        .or(query.order.as_deref())
        .unwrap_or("desc");
    if !matches!(order, "asc" | "desc") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = state.get_store().await;
    let mut sessions = store
        .list(None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(directory) = filters.directory.as_deref() {
        sessions.retain(|session| session.directory == directory);
    }
    if let Some(path) = filters.path.as_deref() {
        sessions.retain(|session| {
            session.path.as_deref() == Some(path)
                || session
                    .path
                    .as_deref()
                    .is_some_and(|value| value.starts_with(&format!("{path}/")))
        });
    }
    if filters.roots.unwrap_or(false) {
        sessions.retain(|session| session.parent_id.is_none());
    }
    if let Some(start) = filters.start {
        sessions.retain(|session| session.time_created >= start);
    }
    if let Some(search) = filters.search.as_deref() {
        sessions.retain(|session| session.title.contains(search));
    }
    apply_order(&mut sessions, order);
    if let Some(cursor) = decoded.as_ref() {
        apply_session_cursor(&mut sessions, cursor);
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 200);
    sessions.truncate(limit);
    let items = sessions
        .iter()
        .cloned()
        .map(session_info_v2)
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "items": items,
        "cursor": page_cursor(items.first(), items.last(), order, &filters),
    })))
}

pub async fn session_messages_v2(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if query.cursor.is_some() && query.order.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let decoded = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let order = decoded
        .as_ref()
        .map(|cursor| cursor.order.as_str())
        .or(query.order.as_deref())
        .unwrap_or("desc");
    if !matches!(order, "asc" | "desc") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    if store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut messages = store
        .get_messages_with_parts(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(message_v2)
        .collect::<Vec<_>>();
    apply_message_order(&mut messages, order);
    if let Some(cursor) = decoded.as_ref() {
        apply_value_cursor(&mut messages, cursor);
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 200);
    messages.truncate(limit);
    Ok(Json(serde_json::json!({
        "items": messages,
        "cursor": page_cursor(
            messages.first(),
            messages.last(),
            order,
            &CursorFilters::default()
        ),
    })))
}

pub async fn session_context_v2(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    if store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let messages = store
        .get_messages_with_parts(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let start = messages
        .iter()
        .rposition(|message| has_compaction_part(message))
        .unwrap_or(0);
    Ok(Json(
        messages
            .into_iter()
            .skip(start)
            .map(message_v2)
            .collect::<Vec<_>>(),
    ))
}

pub async fn session_wait_v2(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.get_store().await;
    if store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn session_compact_v2(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionID::parse(&id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = Arc::new(state.get_store().await);
    let session = store
        .get(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(provider) = state.provider.clone() else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let Some(model_id) = session
        .model
        .as_deref()
        .and_then(model_id_from_selection)
        .or_else(|| {
            state
                .default_model
                .as_deref()
                .and_then(model_id_from_selection)
        })
        .or_else(|| {
            provider
                .default_model()
                .and_then(|model| model.id.as_ref())
                .map(ToString::to_string)
        })
    else {
        return Ok(StatusCode::NO_CONTENT);
    };
    crate::session::compact_session_with_options(
        &store,
        &session_id,
        &provider,
        provider.name(),
        &model_id,
        false,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .event_bus
        .publish(crate::bus::Event::session_update(id));
    Ok(StatusCode::NO_CONTENT)
}

fn apply_order(sessions: &mut [SessionRow], order: &str) {
    match order {
        "asc" => sessions.sort_by(|a, b| {
            a.time_created
                .cmp(&b.time_created)
                .then_with(|| a.id.cmp(&b.id))
        }),
        _ => sessions.sort_by(|a, b| {
            b.time_created
                .cmp(&a.time_created)
                .then_with(|| b.id.cmp(&a.id))
        }),
    }
}

fn apply_message_order(messages: &mut [serde_json::Value], order: &str) {
    messages.sort_by(|a, b| {
        let a_time = a["time"]["created"].as_i64().unwrap_or_default();
        let b_time = b["time"]["created"].as_i64().unwrap_or_default();
        let a_id = a["id"].as_str().unwrap_or_default();
        let b_id = b["id"].as_str().unwrap_or_default();
        if order == "asc" {
            a_time.cmp(&b_time).then_with(|| a_id.cmp(b_id))
        } else {
            b_time.cmp(&a_time).then_with(|| b_id.cmp(a_id))
        }
    });
}

fn has_cursor_filter(query: &PageQuery) -> bool {
    query.order.is_some()
        || query.path.is_some()
        || query.roots.is_some()
        || query.start.is_some()
        || query.search.is_some()
}

fn apply_session_cursor(sessions: &mut Vec<SessionRow>, cursor: &CursorPayload) {
    let position = sessions
        .iter()
        .position(|session| session.id == cursor.id && session.time_created == cursor.time);
    apply_position_cursor(sessions, position, &cursor.direction);
}

fn apply_value_cursor(values: &mut Vec<serde_json::Value>, cursor: &CursorPayload) {
    let position = values.iter().position(|value| {
        value.get("id").and_then(serde_json::Value::as_str) == Some(cursor.id.as_str())
            && value
                .get("time")
                .and_then(|time| time.get("created"))
                .and_then(serde_json::Value::as_i64)
                == Some(cursor.time)
    });
    apply_position_cursor(values, position, &cursor.direction);
}

fn apply_position_cursor<T>(items: &mut Vec<T>, position: Option<usize>, direction: &str) {
    let Some(position) = position else {
        return;
    };
    match direction {
        "previous" => items.truncate(position),
        "next" => {
            items.drain(..=position);
        }
        _ => {}
    }
}

fn session_info_v2(row: SessionRow) -> serde_json::Value {
    let mut object = serde_json::json!({
        "id": row.id,
        "projectID": row.project_id,
        "workspaceID": row.workspace_id,
        "parentID": row.parent_id,
        "path": row.path.unwrap_or_default(),
        "agent": row.agent,
        "model": row.model.as_deref().and_then(model_ref_from_selection),
        "cost": 0.0,
        "tokens": {
            "input": 0.0,
            "output": 0.0,
            "reasoning": 0.0,
            "cache": {
                "read": 0.0,
                "write": 0.0,
            },
        },
        "time": {
            "created": row.time_created,
            "updated": row.time_updated,
            "archived": row.time_archived,
        },
        "title": row.title,
    });
    prune_nulls(&mut object);
    object
}

fn message_v2(message: WithParts) -> serde_json::Value {
    if has_compaction_part(&message) {
        return compaction_message_v2(message);
    }

    match message.info {
        Message::User(user) => serde_json::json!({
            "id": user.id,
            "type": "user",
            "text": text_from_parts(&message.parts),
            "files": files_from_parts(&message.parts),
            "agents": agents_from_parts(&message.parts),
            "references": [],
            "time": {
                "created": user.time.created,
            },
            "metadata": {},
        }),
        Message::Assistant(assistant) => serde_json::json!({
            "id": assistant.id,
            "type": "assistant",
            "agent": assistant.agent,
            "model": {
                "providerID": assistant.provider_id,
                "id": assistant.model_id,
                "variant": assistant.variant.unwrap_or_else(|| "default".to_string()),
            },
            "content": assistant_content_from_parts(&message.parts),
            "finish": assistant.finish,
            "cost": assistant.cost,
            "tokens": {
                "input": assistant.tokens.input,
                "output": assistant.tokens.output,
                "reasoning": assistant.tokens.reasoning,
                "cache": {
                    "read": assistant.tokens.cache.read,
                    "write": assistant.tokens.cache.write,
                },
            },
            "time": {
                "created": assistant.time.created,
                "completed": assistant.time.completed,
            },
            "metadata": {},
        }),
    }
}

fn compaction_message_v2(message: WithParts) -> serde_json::Value {
    let compaction = message.parts.iter().find_map(|part| match part {
        Part::Compaction(part) => Some(part),
        _ => None,
    });
    let (id, created) = match &message.info {
        Message::User(user) => (user.id.to_string(), user.time.created),
        Message::Assistant(assistant) => (assistant.id.to_string(), assistant.time.created),
    };
    let raw_summary = text_from_parts(&message.parts);
    let summary = raw_summary
        .strip_prefix(crate::session::compaction::COMPACTION_PREFIX)
        .unwrap_or(raw_summary.as_str())
        .trim()
        .to_string();
    serde_json::json!({
        "id": id,
        "type": "compaction",
        "reason": if compaction.is_some_and(|part| part.auto) { "auto" } else { "manual" },
        "summary": summary,
        "time": {
            "created": created,
        },
        "metadata": {},
    })
}

fn has_compaction_part(message: &WithParts) -> bool {
    message
        .parts
        .iter()
        .any(|part| matches!(part, Part::Compaction(_)))
}

fn text_from_parts(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(part) => Some(part.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn files_from_parts(parts: &[Part]) -> Vec<serde_json::Value> {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::File(part) => Some(serde_json::json!({
                "mime": part.mime.clone(),
                "filename": part.filename.clone(),
                "url": part.url.clone(),
            })),
            _ => None,
        })
        .collect()
}

fn agents_from_parts(parts: &[Part]) -> Vec<serde_json::Value> {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Agent(part) => Some(serde_json::json!({ "name": part.name.clone() })),
            _ => None,
        })
        .collect()
}

fn assistant_content_from_parts(parts: &[Part]) -> Vec<serde_json::Value> {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(part) => Some(serde_json::json!({
                "type": "text",
                "text": part.text.clone(),
            })),
            Part::Reasoning(part) => Some(serde_json::json!({
                "type": "reasoning",
                "id": part.id.to_string(),
                "text": part.text.clone(),
            })),
            Part::Tool(part) => Some(serde_json::json!({
                "type": "tool",
                "id": part.call_id.clone(),
                "name": part.tool.clone(),
                "state": part.state.clone(),
                "time": {},
            })),
            _ => None,
        })
        .collect()
}

fn model_ref_from_selection(selection: &str) -> Option<serde_json::Value> {
    let (provider, model) = selection.split_once('/')?;
    Some(serde_json::json!({
        "providerID": provider,
        "id": model,
        "variant": "default",
    }))
}

fn model_id_from_selection(selection: &str) -> Option<String> {
    selection
        .split_once('/')
        .map(|(_, model)| model.to_string())
        .or_else(|| (!selection.trim().is_empty()).then(|| selection.to_string()))
}

fn page_cursor(
    first: Option<&serde_json::Value>,
    last: Option<&serde_json::Value>,
    order: &str,
    filters: &CursorFilters,
) -> serde_json::Value {
    serde_json::json!({
        "previous": first.and_then(|value| cursor_for(value, order, "previous", filters)),
        "next": last.and_then(|value| cursor_for(value, order, "next", filters)),
    })
}

fn cursor_for(
    value: &serde_json::Value,
    order: &str,
    direction: &str,
    filters: &CursorFilters,
) -> Option<String> {
    let id = value.get("id")?.as_str()?;
    let time = value.get("time")?.get("created")?.as_i64()?;
    encode_cursor(CursorPayload {
        id: id.to_string(),
        time,
        order: order.to_string(),
        direction: direction.to_string(),
        filters: filters.clone(),
    })
}

fn encode_cursor(cursor: CursorPayload) -> Option<String> {
    serde_json::to_vec(&cursor)
        .ok()
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_cursor(input: &str) -> Result<CursorPayload, StatusCode> {
    let bytes = URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let cursor: CursorPayload =
        serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    if !matches!(cursor.order.as_str(), "asc" | "desc")
        || !matches!(cursor.direction.as_str(), "previous" | "next")
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(cursor)
}

fn prune_nulls(value: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = value {
        map.retain(|_, value| !value.is_null());
        for value in map.values_mut() {
            prune_nulls(value);
        }
    }
}
