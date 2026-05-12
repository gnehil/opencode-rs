use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use serde::{Deserialize, Serialize};
use axum::{
    Router,
    routing::get,
    extract::Query,
    response::{Html, IntoResponse},
    http::StatusCode,
};
use tower_http::cors::CorsLayer;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClientInfo {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub client_id_issued_at: Option<i64>,
    pub client_secret_expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpAuthEntry {
    pub tokens: Option<OAuthTokens>,
    pub client_info: Option<OAuthClientInfo>,
    pub code_verifier: Option<String>,
    pub oauth_state: Option<String>,
    pub server_url: Option<String>,
}

pub struct McpAuthStore {
    filepath: PathBuf,
    entries: Arc<RwLock<HashMap<String, McpAuthEntry>>>,
}

impl McpAuthStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            filepath: data_dir.join("mcp-auth.json"),
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn load(&self) -> anyhow::Result<()> {
        if self.filepath.exists() {
            let content = std::fs::read_to_string(&self.filepath)?;
            let data: HashMap<String, McpAuthEntry> = serde_json::from_str(&content)?;
            let mut entries = self.entries.write().await;
            entries.extend(data);
        }
        Ok(())
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let entries = self.entries.read().await;
        let content = serde_json::to_string(&entries)?;
        std::fs::write(&self.filepath, content)?;
        Ok(())
    }

    pub async fn all(&self) -> HashMap<String, McpAuthEntry> {
        self.entries.read().await.clone()
    }

    pub async fn get(&self, mcp_name: &str) -> Option<McpAuthEntry> {
        self.entries.read().await.get(mcp_name).cloned()
    }

    pub async fn get_for_url(&self, mcp_name: &str, server_url: &str) -> Option<McpAuthEntry> {
        let entries = self.entries.read().await;
        let entry = entries.get(mcp_name)?;
        if entry.server_url.as_deref()? != server_url {
            return None;
        }
        Some(entry.clone())
    }

    pub async fn set(&self, mcp_name: &str, entry: McpAuthEntry) -> anyhow::Result<()> {
        let mut entries = self.entries.write().await;
        entries.insert(mcp_name.to_string(), entry);
        self.save().await?;
        Ok(())
    }

    pub async fn remove(&self, mcp_name: &str) -> anyhow::Result<()> {
        let mut entries = self.entries.write().await;
        entries.remove(mcp_name);
        self.save().await?;
        Ok(())
    }

    pub async fn update_tokens(&self, mcp_name: &str, tokens: OAuthTokens, server_url: Option<&str>) -> anyhow::Result<()> {
        let mut entries = self.entries.write().await;
        let entry = entries.entry(mcp_name.to_string()).or_default();
        entry.tokens = Some(tokens);
        if let Some(url) = server_url {
            entry.server_url = Some(url.to_string());
        }
        self.save().await?;
        Ok(())
    }

    pub async fn update_client_info(&self, mcp_name: &str, client_info: OAuthClientInfo, server_url: Option<&str>) -> anyhow::Result<()> {
        let mut entries = self.entries.write().await;
        let entry = entries.entry(mcp_name.to_string()).or_default();
        entry.client_info = Some(client_info);
        if let Some(url) = server_url {
            entry.server_url = Some(url.to_string());
        }
        self.save().await?;
        Ok(())
    }

    pub async fn update_code_verifier(&self, mcp_name: &str, verifier: &str) -> anyhow::Result<()> {
        let mut entries = self.entries.write().await;
        let entry = entries.entry(mcp_name.to_string()).or_default();
        entry.code_verifier = Some(verifier.to_string());
        self.save().await?;
        Ok(())
    }

    pub async fn update_oauth_state(&self, mcp_name: &str, state: &str) -> anyhow::Result<()> {
        let mut entries = self.entries.write().await;
        let entry = entries.entry(mcp_name.to_string()).or_default();
        entry.oauth_state = Some(state.to_string());
        self.save().await?;
        Ok(())
    }

    pub async fn is_token_expired(&self, mcp_name: &str) -> Option<bool> {
        let entries = self.entries.read().await;
        let entry = entries.get(mcp_name)?;
        let tokens = entry.tokens.as_ref()?;
        let expires_at = tokens.expires_at?;
        Some(expires_at < chrono::Utc::now().timestamp())
    }
}

pub const OAUTH_CALLBACK_PORT: u16 = 19876;
pub const OAUTH_CALLBACK_PATH: &str = "/mcp/oauth/callback";
pub const CALLBACK_TIMEOUT_MS: u64 = 5 * 60 * 1000;

const HTML_SUCCESS: &str = r#"<!DOCTYPE html>
<html>
<head>
  <title>OpenCode - Authorization Successful</title>
  <style>
    body { font-family: system-ui, -apple-system, sans-serif; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #1a1a2e; color: #eee; }
    .container { text-align: center; padding: 2rem; }
    h1 { color: #4ade80; margin-bottom: 1rem; }
    p { color: #aaa; }
  </style>
</head>
<body>
  <div class="container">
    <h1>Authorization Successful</h1>
    <p>You can close this window and return to OpenCode.</p>
  </div>
  <script>setTimeout(() => window.close(), 2000);</script>
</body>
</html>"#;

fn html_error(error: &str) -> String {
    format!(r#"<!DOCTYPE html>
<html>
<head>
  <title>OpenCode - Authorization Failed</title>
  <style>
    body { font-family: system-ui, -apple-system, sans-serif; display: flex; justify-content: center; align-items: center; height: 100vh; margin: 0; background: #1a1a2e; color: #eee; }
    .container { text-align: center; padding: 2rem; }
    h1 { color: #f87171; margin-bottom: 1rem; }
    p { color: #aaa; }
    .error { color: #fca5a5; font-family: monospace; margin-top: 1rem; padding: 1rem; background: rgba(248,113,113,0.1); border-radius: 0.5rem; }
  </style>
</head>
<body>
  <div class="container">
    <h1>Authorization Failed</h1>
    <p>An error occurred during authorization.</p>
    <div class="error">{}</div>
  </div>
</body>
</html>"#, error)
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub struct PendingAuth {
    resolve: mpsc::Sender<String>,
    oauth_state: String,
}

pub struct OAuthCallbackServer {
    port: u16,
    pending: Arc<RwLock<HashMap<String, PendingAuth>>>,
    server: Option<tokio::task::JoinHandle<()>>,
}

impl OAuthCallbackServer {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            pending: Arc::new(RwLock::new(HashMap::new())),
            server: None,
        }
    }

    pub fn default() -> Self {
        Self::new(OAUTH_CALLBACK_PORT)
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        let pending = self.pending.clone();
        let port = self.port;

        let app = Router::new()
            .route(OAUTH_CALLBACK_PATH, get(|query: Query<CallbackQuery>| {
                let pending = pending.clone();
                async move {
                    handle_callback(query, pending).await
                }
            }))
            .layer(CorsLayer::permissive());

        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await?;

        self.server = Some(tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        }));

        Ok(())
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(server) = self.server.take() {
            server.abort();
        }
        self.pending.write().await.clear();
        Ok(())
    }

    pub async fn wait_for_callback(&self, oauth_state: &str, mcp_name: &str) -> anyhow::Result<String> {
        let (tx, mut rx) = mpsc::channel::<String>(1);

        {
            let mut pending = self.pending.write().await;
            pending.insert(mcp_name.to_string(), PendingAuth {
                resolve: tx,
                oauth_state: oauth_state.to_string(),
            });
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(CALLBACK_TIMEOUT_MS),
            rx.recv()
        ).await?;

        {
            let mut pending = self.pending.write().await;
            pending.remove(mcp_name);
        }

        result.ok_or_else(|| anyhow::anyhow!("No authorization code received"))
    }

    pub async fn cancel_pending(&self, mcp_name: &str) {
        let mut pending = self.pending.write().await;
        pending.remove(mcp_name);
    }

    pub async fn is_port_in_use(&self) -> bool {
        tokio::net::TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], self.port)))
            .await
            .is_err()
    }
}

async fn handle_callback(
    query: Query<CallbackQuery>,
    pending: Arc<RwLock<HashMap<String, PendingAuth>>>,
) -> impl IntoResponse {
    let query = query.0;

    if let Some(error) = query.error {
        let msg = query.error_description.unwrap_or(error);
        return (StatusCode::BAD_REQUEST, Html(html_error(&msg)));
    }

    if query.state.is_none() {
        return (StatusCode::BAD_REQUEST, Html(html_error("Missing state parameter")));
    }

    if query.code.is_none() {
        return (StatusCode::BAD_REQUEST, Html(html_error("No authorization code provided")));
    }

    let state = query.state.unwrap();
    let code = query.code.unwrap();

    {
        let pending_map = pending.read().await;
        let matching = pending_map.values().find(|p| p.oauth_state == state);

        if matching.is_none() {
            return (StatusCode::BAD_REQUEST, Html(html_error("Invalid state parameter")));
        }

        if let Some(auth) = matching {
            let _ = auth.resolve.send(code.clone()).await;
        }
    }

    (StatusCode::OK, Html(HTML_SUCCESS))
}

pub fn generate_state() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    hex::encode(bytes)
}

pub fn generate_code_verifier() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn generate_code_challenge(verifier: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

pub struct McpOAuthProvider {
    mcp_name: String,
    server_url: String,
    auth_store: Arc<McpAuthStore>,
    callback_server: Arc<OAuthCallbackServer>,
}

impl McpOAuthProvider {
    pub fn new(
        mcp_name: String,
        server_url: String,
        auth_store: Arc<McpAuthStore>,
        callback_server: Arc<OAuthCallbackServer>,
    ) -> Self {
        Self {
            mcp_name,
            server_url,
            auth_store,
            callback_server,
        }
    }

    pub fn redirect_url(&self) -> String {
        format!("http://127.0.0.1:{}{}", OAUTH_CALLBACK_PORT, OAUTH_CALLBACK_PATH)
    }

    pub async fn client_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "redirect_uris": [self.redirect_url()],
            "client_name": "OpenCode",
            "client_uri": "https://opencode.ai",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none"
        })
    }

    pub async fn client_info(&self) -> Option<OAuthClientInfo> {
        self.auth_store.get_for_url(&self.mcp_name, &self.server_url).await
            .and_then(|e| e.client_info)
    }

    pub async fn save_client_info(&self, client_info: OAuthClientInfo) -> anyhow::Result<()> {
        self.auth_store.update_client_info(&self.mcp_name, client_info, Some(&self.server_url)).await
    }

    pub async fn tokens(&self) -> Option<OAuthTokens> {
        self.auth_store.get_for_url(&self.mcp_name, &self.server_url).await
            .and_then(|e| e.tokens)
    }

    pub async fn save_tokens(&self, tokens: OAuthTokens) -> anyhow::Result<()> {
        self.auth_store.update_tokens(&self.mcp_name, tokens, Some(&self.server_url)).await
    }

    pub async fn save_code_verifier(&self, verifier: &str) -> anyhow::Result<()> {
        self.auth_store.update_code_verifier(&self.mcp_name, verifier).await
    }

    pub async fn code_verifier(&self) -> Option<String> {
        self.auth_store.get(&self.mcp_name).await
            .and_then(|e| e.code_verifier)
    }

    pub async fn save_state(&self, state: &str) -> anyhow::Result<()> {
        self.auth_store.update_oauth_state(&self.mcp_name, state).await
    }

    pub async fn state(&self) -> String {
        match self.auth_store.get(&self.mcp_name).await {
            Some(entry) if entry.oauth_state.is_some() => entry.oauth_state.unwrap(),
            _ => {
                let new_state = generate_state();
                self.save_state(&new_state).await.ok();
                new_state
            }
        }
    }

    pub async fn invalidate_credentials(&self, _type: &str) -> anyhow::Result<()> {
        self.auth_store.remove(&self.mcp_name).await
    }

    pub async fn start_auth(&self) -> anyhow::Result<String> {
        let state = generate_state();
        self.save_state(&state).await?;

        let verifier = generate_code_verifier();
        self.save_code_verifier(&verifier).await?;

        Ok(state)
    }

    pub async fn complete_auth(&self, code: &str, token_endpoint: &str) -> anyhow::Result<OAuthTokens> {
        let verifier = self.code_verifier()
            .ok_or_else(|| anyhow::anyhow!("No code verifier found"))?;

        let redirect_uri = self.redirect_url();
        
        let client = reqwest::Client::new();
        let response = client
            .post(token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &redirect_uri),
                ("code_verifier", &verifier),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Token exchange failed: {}", error_text));
        }

        let token_response: TokenEndpointResponse = response.json().await?;

        let tokens = OAuthTokens {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token,
            expires_at: token_response.expires_in.map(|exp| {
                chrono::Utc::now().timestamp() + exp as i64
            }),
            scope: token_response.scope,
        };

        self.save_tokens(tokens.clone()).await?;

        Ok(tokens)
    }

    pub async fn refresh_tokens(&self, refresh_token: &str, token_endpoint: &str) -> anyhow::Result<OAuthTokens> {
        let client = reqwest::Client::new();
        let response = client
            .post(token_endpoint)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Token refresh failed: {}", error_text));
        }

        let token_response: TokenEndpointResponse = response.json().await?;

        let tokens = OAuthTokens {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token.or(Some(refresh_token.to_string())),
            expires_at: token_response.expires_in.map(|exp| {
                chrono::Utc::now().timestamp() + exp as i64
            }),
            scope: token_response.scope,
        };

        self.save_tokens(tokens.clone()).await?;

        Ok(tokens)
    }
}

#[derive(Debug, Deserialize)]
struct TokenEndpointResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u32>,
    scope: Option<String>,
    token_type: String,
}