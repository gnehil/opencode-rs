use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use super::session_handlers::AppState;
use crate::cli::mcp_cli::{discover_oauth_metadata, mcp_auth_target, McpAuthTarget};
use crate::mcp::{
    generate_code_challenge, McpAuthEntry, McpOAuthProvider, McpServerStatus, OAuthCallbackServer,
    OAuthClientInfo, OAuthTokens,
};

#[derive(Deserialize)]
pub struct AddMcpRequest {
    pub name: String,
    pub config: crate::config::McpServerConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAuthStartResponse {
    pub authorization_url: String,
    pub oauth_state: String,
}

#[derive(Deserialize)]
pub struct McpAuthCallbackPayload {
    pub code: String,
}

#[derive(Serialize)]
pub struct McpAuthRemoveResponse {
    pub success: bool,
}

pub async fn mcp_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let manager = state.mcp_manager.read().await;
    Ok(Json(json!(manager.status())))
}

pub async fn mcp_add(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AddMcpRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut manager = state.mcp_manager.write().await;
    manager
        .start_server(&body.name, &body.config)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(json!(manager.status())))
}

pub async fn mcp_auth_start(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<McpAuthStartResponse>, StatusCode> {
    Ok(Json(start_auth_flow(&state, &name).await?))
}

pub async fn mcp_auth_callback(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<McpAuthCallbackPayload>,
) -> Result<Json<McpServerStatus>, StatusCode> {
    let target = auth_target(&state, &name)?;
    state
        .mcp_auth_store
        .load()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let client = reqwest::Client::new();
    let metadata = discover_oauth_metadata(&client, &target.url)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let tokens = exchange_authorization_code(&state, &target, &metadata.token_endpoint, &body.code)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    save_completed_auth(&state, &target, tokens)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(McpServerStatus::NeedsAuth))
}

pub async fn mcp_auth_authenticate(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<McpServerStatus>, StatusCode> {
    let _ = start_auth_flow(&state, &name).await?;
    Ok(Json(McpServerStatus::NeedsAuth))
}

pub async fn mcp_auth_remove(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<McpAuthRemoveResponse>, StatusCode> {
    let _ = auth_target(&state, &name)?;
    state
        .mcp_auth_store
        .load()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .mcp_auth_store
        .remove(&name)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(McpAuthRemoveResponse { success: true }))
}

pub async fn mcp_connect(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>, StatusCode> {
    let mut manager = state.mcp_manager.write().await;
    manager
        .connect_server(&name)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(true))
}

pub async fn mcp_disconnect(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<bool>, StatusCode> {
    let mut manager = state.mcp_manager.write().await;
    manager
        .stop_server(&name)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(true))
}

pub async fn mcp_list_resources(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let manager = state.mcp_manager.read().await;
    let resources = manager.list_all_resources().await;
    Ok(Json(json!(resources)))
}

fn auth_target(state: &AppState, name: &str) -> Result<McpAuthTarget, StatusCode> {
    let config = state.config.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    mcp_auth_target(config, name).map_err(|err| {
        let message = err.to_string();
        if message.contains("not found") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        }
    })
}

async fn start_auth_flow(state: &AppState, name: &str) -> Result<McpAuthStartResponse, StatusCode> {
    let target = auth_target(state, name)?;
    state
        .mcp_auth_store
        .load()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let client = reqwest::Client::new();
    let metadata = discover_oauth_metadata(&client, &target.url)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let callback_server = Arc::new(OAuthCallbackServer::default());
    let provider = McpOAuthProvider::new(
        target.name.clone(),
        target.url.clone(),
        state.mcp_auth_store.clone(),
        callback_server,
    );
    let oauth_state = provider
        .start_auth()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let verifier = provider
        .code_verifier()
        .await
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let client_id = resolve_client_id(&client, &target, &metadata, &provider).await?;
    let redirect_uri = target
        .redirect_uri
        .clone()
        .unwrap_or_else(|| provider.redirect_url());
    let authorization_url = build_authorization_url(
        &metadata.authorization_endpoint,
        &client_id,
        &redirect_uri,
        target.scope.as_deref(),
        &oauth_state,
        &generate_code_challenge(&verifier),
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;

    Ok(McpAuthStartResponse {
        authorization_url,
        oauth_state,
    })
}

async fn resolve_client_id(
    client: &reqwest::Client,
    target: &McpAuthTarget,
    metadata: &crate::cli::mcp_cli::OAuthServerMetadata,
    provider: &McpOAuthProvider,
) -> Result<String, StatusCode> {
    if let Some(client_id) = target.client_id.as_ref().filter(|id| !id.trim().is_empty()) {
        return Ok(client_id.trim().to_string());
    }
    if let Some(info) = provider.client_info().await {
        return Ok(info.client_id);
    }
    let registration_endpoint = metadata
        .registration_endpoint
        .as_deref()
        .ok_or(StatusCode::BAD_REQUEST)?;
    register_oauth_client(client, registration_endpoint, provider).await
}

async fn register_oauth_client(
    client: &reqwest::Client,
    registration_endpoint: &str,
    provider: &McpOAuthProvider,
) -> Result<String, StatusCode> {
    #[derive(Deserialize)]
    struct RegistrationResponse {
        client_id: String,
        client_secret: Option<String>,
        client_id_issued_at: Option<i64>,
        client_secret_expires_at: Option<i64>,
    }

    let response = client
        .post(registration_endpoint)
        .json(&provider.client_metadata().await)
        .send()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .error_for_status()
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .json::<RegistrationResponse>()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let client_id = response.client_id.clone();
    provider
        .save_client_info(OAuthClientInfo {
            client_id: response.client_id,
            client_secret: response.client_secret,
            client_id_issued_at: response.client_id_issued_at,
            client_secret_expires_at: response.client_secret_expires_at,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(client_id)
}

fn build_authorization_url(
    authorization_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    scope: Option<&str>,
    state: &str,
    code_challenge: &str,
) -> anyhow::Result<String> {
    let mut url = reqwest::Url::parse(authorization_endpoint)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256");
        if let Some(scope) = scope.filter(|scope| !scope.trim().is_empty()) {
            query.append_pair("scope", scope.trim());
        }
    }
    Ok(url.to_string())
}

async fn exchange_authorization_code(
    state: &AppState,
    target: &McpAuthTarget,
    token_endpoint: &str,
    code: &str,
) -> anyhow::Result<OAuthTokens> {
    #[derive(Deserialize)]
    struct TokenEndpointResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u32>,
        scope: Option<String>,
        token_type: String,
    }

    let entry = state
        .mcp_auth_store
        .get(&target.name)
        .await
        .ok_or_else(|| anyhow::anyhow!("No pending OAuth flow for MCP server: {}", target.name))?;
    let code_verifier = entry
        .code_verifier
        .ok_or_else(|| anyhow::anyhow!("No code verifier found"))?;
    let callback_server = OAuthCallbackServer::default();
    let redirect_uri = target.redirect_uri.clone().unwrap_or_else(|| {
        McpOAuthProvider::new(
            target.name.clone(),
            target.url.clone(),
            state.mcp_auth_store.clone(),
            Arc::new(callback_server),
        )
        .redirect_url()
    });
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.to_string()),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];
    if let Some(client_id) = target.client_id.as_ref().filter(|id| !id.trim().is_empty()) {
        form.push(("client_id", client_id.trim().to_string()));
    } else if let Some(client_info) = entry.client_info {
        form.push(("client_id", client_info.client_id));
        if let Some(client_secret) = client_info.client_secret {
            form.push(("client_secret", client_secret));
        }
    }
    if let Some(client_secret) = target
        .client_secret
        .as_ref()
        .filter(|secret| !secret.trim().is_empty())
    {
        form.push(("client_secret", client_secret.trim().to_string()));
    }

    let response = reqwest::Client::new()
        .post(token_endpoint)
        .form(&form)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("Token exchange failed: {}", response.text().await?);
    }
    let token_response = response.json::<TokenEndpointResponse>().await?;
    let _token_type = token_response.token_type;
    Ok(OAuthTokens {
        access_token: token_response.access_token,
        refresh_token: token_response.refresh_token,
        expires_at: token_response
            .expires_in
            .map(|expires_in| chrono::Utc::now().timestamp() + expires_in as i64),
        scope: token_response.scope,
    })
}

async fn save_completed_auth(
    state: &AppState,
    target: &McpAuthTarget,
    tokens: OAuthTokens,
) -> anyhow::Result<()> {
    let mut entry = state
        .mcp_auth_store
        .get(&target.name)
        .await
        .unwrap_or_else(McpAuthEntry::default);
    entry.tokens = Some(tokens);
    entry.server_url = Some(target.url.clone());
    entry.code_verifier = None;
    entry.oauth_state = None;
    state.mcp_auth_store.set(&target.name, entry).await
}
