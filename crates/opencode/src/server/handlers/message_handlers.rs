use axum::{
    extract::{Path, Json},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct PromptRequest {
    message: String,
}

#[derive(Serialize)]
pub struct PromptResponse {
    content: String,
}

pub async fn list_messages(
    Path(_id): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    Ok(Json(vec![]))
}

pub async fn prompt(
    Path(id): Path<String>,
    Json(req): Json<PromptRequest>,
) -> Result<Json<PromptResponse>, StatusCode> {
    Ok(Json(PromptResponse {
        content: format!("Session {}: Received: {}", id, req.message),
    }))
}