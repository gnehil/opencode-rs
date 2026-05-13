use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::{McpConfigEntry, McpOAuthConfig};
use crate::mcp::{generate_code_challenge, McpAuthStore, McpOAuthProvider, OAuthCallbackServer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpAddSpec {
    Local { command: Vec<String> },
    Remote { url: String, oauth: McpOAuthChoice },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum McpOAuthChoice {
    Disabled,
    Default,
    Dynamic,
    Client {
        client_id: String,
        client_secret: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct McpAuthTarget {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) client_id: Option<String>,
    pub(crate) client_secret: Option<String>,
    pub(crate) scope: Option<String>,
    pub(crate) redirect_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct OAuthServerMetadata {
    pub(crate) authorization_endpoint: String,
    pub(crate) token_endpoint: String,
    pub(crate) registration_endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct McpOAuthStart {
    pub(crate) authorization_url: String,
    pub(crate) token_endpoint: String,
    pub(crate) state: String,
    pub(crate) redirect_uri: String,
}

pub(crate) struct McpOAuthSession {
    pub(crate) start: McpOAuthStart,
    pub(crate) provider: McpOAuthProvider,
    pub(crate) callback_server: Arc<OAuthCallbackServer>,
}

pub(crate) fn mcp_config_value(spec: McpAddSpec) -> anyhow::Result<serde_json::Value> {
    match spec {
        McpAddSpec::Local { command } => {
            if command.is_empty() || command.iter().any(|part| part.trim().is_empty()) {
                anyhow::bail!("local MCP command is required");
            }
            Ok(serde_json::json!({
                "type": "local",
                "command": command,
            }))
        }
        McpAddSpec::Remote { url, oauth } => {
            let url = normalize_url(&url)?;
            let mut entry = serde_json::json!({
                "type": "remote",
                "url": url,
            });
            let object = entry
                .as_object_mut()
                .expect("remote MCP entry is an object");
            match oauth {
                McpOAuthChoice::Disabled => {
                    object.insert("oauth".to_string(), serde_json::Value::Bool(false));
                }
                McpOAuthChoice::Default => {}
                McpOAuthChoice::Dynamic => {
                    object.insert("oauth".to_string(), serde_json::json!({}));
                }
                McpOAuthChoice::Client {
                    client_id,
                    client_secret,
                } => {
                    let client_id = require_non_empty("client ID", &client_id)?;
                    let mut oauth = serde_json::json!({ "clientId": client_id });
                    if let Some(client_secret) = client_secret
                        .as_deref()
                        .map(str::trim)
                        .filter(|secret| !secret.is_empty())
                    {
                        oauth["clientSecret"] =
                            serde_json::Value::String(client_secret.to_string());
                    }
                    object.insert("oauth".to_string(), oauth);
                }
            }
            Ok(entry)
        }
    }
}

pub(crate) fn upsert_mcp_config_text(
    existing: &str,
    name: &str,
    entry: serde_json::Value,
) -> anyhow::Result<String> {
    let name = require_non_empty("MCP server name", name)?;
    let mut root = if existing.trim().is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        parse_jsonc_value(existing)?
    };
    let root_object = root
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("opencode config must be a JSON object"))?;
    let mcp = root_object
        .entry("mcp")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let mcp_object = mcp
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("opencode config mcp field must be a JSON object"))?;
    mcp_object.insert(name, entry);
    Ok(format!("{}\n", serde_json::to_string_pretty(&root)?))
}

pub(crate) fn resolve_mcp_config_path(base_dir: &Path, global: bool) -> PathBuf {
    let candidates = if global {
        vec![
            base_dir.join("opencode.jsonc"),
            base_dir.join("opencode.json"),
        ]
    } else {
        vec![
            base_dir.join("opencode.jsonc"),
            base_dir.join("opencode.json"),
            base_dir.join(".opencode").join("opencode.jsonc"),
            base_dir.join(".opencode").join("opencode.json"),
        ]
    };
    for candidate in &candidates {
        if candidate.is_file() {
            return candidate.clone();
        }
    }
    if global {
        base_dir.join("opencode.json")
    } else {
        base_dir.join("opencode.jsonc")
    }
}

pub(crate) fn mcp_auth_target(
    config: &crate::config::Config,
    name: &str,
) -> anyhow::Result<McpAuthTarget> {
    let server = config
        .mcp
        .as_ref()
        .and_then(|mcp| mcp.get(name))
        .ok_or_else(|| anyhow::anyhow!("MCP server not found: {name}"))?;
    let McpConfigEntry::Full(server) = server else {
        anyhow::bail!("MCP server {name} is disabled or incomplete");
    };
    if server.kind.as_deref() != Some("remote") && server.url.is_none() {
        anyhow::bail!("MCP server {name} is not a remote server");
    }
    if matches!(server.oauth, Some(McpOAuthConfig::Enabled(false))) {
        anyhow::bail!("MCP server {name} has OAuth disabled");
    }
    let (client_id, client_secret, scope, redirect_uri) = match &server.oauth {
        Some(McpOAuthConfig::Options(options)) => (
            options.client_id.clone(),
            options.client_secret.clone(),
            options.scope.clone(),
            options.redirect_uri.clone(),
        ),
        _ => (None, None, None, None),
    };
    Ok(McpAuthTarget {
        name: name.to_string(),
        url: normalize_url(
            server
                .url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("MCP server {name} has no remote URL"))?,
        )?,
        client_id,
        client_secret,
        scope,
        redirect_uri,
    })
}

pub(crate) async fn discover_oauth_metadata(
    client: &reqwest::Client,
    server_url: &str,
) -> anyhow::Result<OAuthServerMetadata> {
    let url = reqwest::Url::parse(&normalize_url(server_url)?)?;
    let origin = format!(
        "{}://{}",
        url.scheme(),
        url.host_str()
            .ok_or_else(|| anyhow::anyhow!("remote MCP URL has no host"))?
    );
    let port_suffix = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let discovery_url = format!("{origin}{port_suffix}/.well-known/oauth-authorization-server");
    Ok(client
        .get(discovery_url)
        .send()
        .await?
        .error_for_status()?
        .json::<OAuthServerMetadata>()
        .await?)
}

pub(crate) async fn start_mcp_oauth(
    data_dir: PathBuf,
    target: McpAuthTarget,
    metadata: OAuthServerMetadata,
) -> anyhow::Result<McpOAuthSession> {
    let store = Arc::new(McpAuthStore::new(data_dir));
    store.load().await?;
    let mut callback_server = OAuthCallbackServer::default();
    callback_server.start().await?;
    let callback_server = Arc::new(callback_server);
    let provider = McpOAuthProvider::new(
        target.name.clone(),
        target.url.clone(),
        store.clone(),
        callback_server.clone(),
    );
    let state = provider.start_auth().await?;
    let redirect_uri = target
        .redirect_uri
        .clone()
        .unwrap_or_else(|| provider.redirect_url());
    let verifier = provider
        .code_verifier()
        .await
        .ok_or_else(|| anyhow::anyhow!("MCP OAuth code verifier was not stored"))?;
    let client_id = match target.client_id.clone() {
        Some(client_id) => client_id,
        None => {
            let registration_endpoint = metadata
                .registration_endpoint
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("MCP OAuth server does not advertise dynamic registration and no clientId is configured"))?;
            register_oauth_client(&reqwest::Client::new(), registration_endpoint, &provider).await?
        }
    };
    let authorization_url = build_authorization_url(
        &metadata.authorization_endpoint,
        &client_id,
        &redirect_uri,
        target.scope.as_deref(),
        &state,
        &generate_code_challenge(&verifier),
    )?;
    let start = McpOAuthStart {
        authorization_url,
        token_endpoint: metadata.token_endpoint,
        state,
        redirect_uri,
    };
    Ok(McpOAuthSession {
        start,
        provider,
        callback_server,
    })
}

pub(crate) async fn complete_mcp_oauth(
    session: &McpOAuthSession,
    code: &str,
) -> anyhow::Result<crate::mcp::OAuthTokens> {
    session
        .provider
        .complete_auth(code, &session.start.token_endpoint)
        .await
}

async fn register_oauth_client(
    client: &reqwest::Client,
    registration_endpoint: &str,
    provider: &McpOAuthProvider,
) -> anyhow::Result<String> {
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
        .await?
        .error_for_status()?
        .json::<RegistrationResponse>()
        .await?;
    let client_id = response.client_id.clone();
    provider
        .save_client_info(crate::mcp::OAuthClientInfo {
            client_id: response.client_id,
            client_secret: response.client_secret,
            client_id_issued_at: response.client_id_issued_at,
            client_secret_expires_at: response.client_secret_expires_at,
        })
        .await?;
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

fn normalize_url(url: &str) -> anyhow::Result<String> {
    let url = url.trim();
    if url.is_empty() {
        anyhow::bail!("URL is required");
    }
    let parsed = reqwest::Url::parse(url)?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        anyhow::bail!("URL must use http or https");
    }
    Ok(url.trim_end_matches('/').to_string())
}

fn require_non_empty(label: &str, value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{label} is required");
    }
    Ok(value.to_string())
}

fn parse_jsonc_value(text: &str) -> anyhow::Result<serde_json::Value> {
    let parsed = jsonc_parser::parse_text(text)
        .map_err(|e| anyhow::anyhow!("failed to parse opencode config: {:?}", e))?;
    match parsed.value {
        Some(value) => jsonc_to_json(value),
        None => Ok(serde_json::Value::Object(serde_json::Map::new())),
    }
}

fn jsonc_to_json(value: jsonc_parser::ast::Value) -> anyhow::Result<serde_json::Value> {
    use jsonc_parser::ast::Value;

    Ok(match value {
        Value::StringLit(v) => serde_json::Value::String(v.value.as_ref().to_string()),
        Value::NumberLit(v) => serde_json::from_str(v.value.as_ref())?,
        Value::BooleanLit(v) => serde_json::Value::Bool(v.value),
        Value::Object(v) => {
            let mut map = serde_json::Map::new();
            for prop in v.properties {
                map.insert(
                    prop.name.value.as_ref().to_string(),
                    jsonc_to_json(prop.value)?,
                );
            }
            serde_json::Value::Object(map)
        }
        Value::Array(v) => serde_json::Value::Array(
            v.elements
                .into_iter()
                .map(jsonc_to_json)
                .collect::<anyhow::Result<Vec<_>>>()?,
        ),
        Value::NullKeyword(_) => serde_json::Value::Null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_local_and_remote_mcp_entries_for_config_schema() {
        let local = mcp_config_value(McpAddSpec::Local {
            command: vec![
                "npx".into(),
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
            ],
        })
        .unwrap();
        assert_eq!(
            local,
            json!({
                "type": "local",
                "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem"]
            })
        );

        let remote = mcp_config_value(McpAddSpec::Remote {
            url: "https://example.com/mcp".into(),
            oauth: McpOAuthChoice::Client {
                client_id: "client-1".into(),
                client_secret: Some("secret-1".into()),
            },
        })
        .unwrap();
        assert_eq!(
            remote,
            json!({
                "type": "remote",
                "url": "https://example.com/mcp",
                "oauth": {
                    "clientId": "client-1",
                    "clientSecret": "secret-1"
                }
            })
        );
    }

    #[test]
    fn upsert_mcp_config_text_adds_mcp_without_dropping_existing_keys() {
        let existing = r#"{
  "$schema": "https://opencode.ai/config.json",
  "model": "openai/gpt-4.1"
}"#;
        let updated = upsert_mcp_config_text(
            existing,
            "filesystem",
            json!({ "type": "local", "command": ["npx", "-y", "server"] }),
        )
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(parsed["$schema"], "https://opencode.ai/config.json");
        assert_eq!(parsed["model"], "openai/gpt-4.1");
        assert_eq!(parsed["mcp"]["filesystem"]["type"], "local");
        assert_eq!(
            parsed["mcp"]["filesystem"]["command"],
            json!(["npx", "-y", "server"])
        );
    }

    #[test]
    fn resolve_mcp_config_path_prefers_existing_project_jsonc_then_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("opencode.json"), "{}").unwrap();
        std::fs::write(dir.path().join("opencode.jsonc"), "{}").unwrap();

        assert_eq!(
            resolve_mcp_config_path(dir.path(), false),
            dir.path().join("opencode.jsonc")
        );
    }

    #[test]
    fn mcp_auth_target_accepts_remote_default_oauth_and_rejects_disabled_oauth() {
        let config: crate::config::Config = serde_json::from_value(json!({
            "mcp": {
                "remote": {
                    "type": "remote",
                    "url": "https://example.com/mcp"
                },
                "disabled": {
                    "type": "remote",
                    "url": "https://example.com/disabled",
                    "oauth": false
                },
                "client": {
                    "type": "remote",
                    "url": "https://example.com/client",
                    "oauth": {
                        "clientId": "client-1",
                        "clientSecret": "secret-1"
                    }
                }
            }
        }))
        .unwrap();

        assert_eq!(
            mcp_auth_target(&config, "remote").unwrap(),
            McpAuthTarget {
                name: "remote".into(),
                url: "https://example.com/mcp".into(),
                client_id: None,
                client_secret: None,
                scope: None,
                redirect_uri: None,
            }
        );
        assert_eq!(
            mcp_auth_target(&config, "client").unwrap(),
            McpAuthTarget {
                name: "client".into(),
                url: "https://example.com/client".into(),
                client_id: Some("client-1".into()),
                client_secret: Some("secret-1".into()),
                scope: None,
                redirect_uri: None,
            }
        );
        assert!(mcp_auth_target(&config, "disabled").is_err());
    }
}
