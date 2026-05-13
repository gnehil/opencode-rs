use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum ProviderCredential {
    Api {
        key: String,
        #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
        metadata: BTreeMap<String, String>,
    },
    Wellknown {
        key: String,
        token: String,
    },
    Oauth {
        refresh: String,
        access: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        expires: Option<i64>,
        #[serde(flatten)]
        extra: BTreeMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLoginMode {
    WellKnownUrl { base_url: String },
    ApiKey { provider: String },
    SelectProvider,
}

pub(crate) struct ProviderAuthStore {
    path: PathBuf,
}

impl ProviderAuthStore {
    pub(crate) fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: data_dir.into().join("auth.json"),
        }
    }

    pub(crate) fn auth_path(&self) -> &std::path::Path {
        &self.path
    }

    pub(crate) fn load(&self) -> anyhow::Result<BTreeMap<String, ProviderCredential>> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let text = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&text)?)
    }

    pub(crate) fn save(
        &self,
        credentials: &BTreeMap<String, ProviderCredential>,
    ) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(credentials)?;
        std::fs::write(&self.path, format!("{text}\n"))?;
        Ok(())
    }

    pub(crate) fn set(&self, key: &str, credential: ProviderCredential) -> anyhow::Result<()> {
        let key = normalize_store_key(key)?;
        let mut credentials = self.load()?;
        credentials.insert(key, credential);
        self.save(&credentials)
    }

    pub(crate) fn remove(&self, key: &str) -> anyhow::Result<bool> {
        let key = normalize_store_key(key)?;
        let mut credentials = self.load()?;
        let removed = credentials.remove(&key).is_some();
        self.save(&credentials)?;
        Ok(removed)
    }
}

pub(crate) fn choose_provider_login_mode(
    url: Option<&str>,
    provider: Option<&str>,
) -> anyhow::Result<ProviderLoginMode> {
    if let Some(url) = url {
        let base_url = normalize_well_known_base_url(url)?;
        return Ok(ProviderLoginMode::WellKnownUrl { base_url });
    }
    if let Some(provider) = provider {
        let provider = normalize_provider_id(provider)?;
        return Ok(ProviderLoginMode::ApiKey { provider });
    }
    Ok(ProviderLoginMode::SelectProvider)
}

pub(crate) fn api_key_credential(key: &str) -> anyhow::Result<ProviderCredential> {
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("API key is required");
    }
    Ok(ProviderCredential::Api {
        key: key.to_string(),
        metadata: BTreeMap::new(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct WellKnownProviderMetadata {
    pub(crate) auth: WellKnownAuthMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct WellKnownAuthMetadata {
    pub(crate) command: Vec<String>,
    pub(crate) env: String,
}

pub(crate) async fn fetch_well_known_metadata(
    client: &reqwest::Client,
    base_url: &str,
) -> anyhow::Result<WellKnownProviderMetadata> {
    let base_url = normalize_well_known_base_url(base_url)?;
    let metadata = client
        .get(format!("{base_url}/.well-known/opencode"))
        .send()
        .await?
        .error_for_status()?
        .json::<WellKnownProviderMetadata>()
        .await?;
    if metadata.auth.command.is_empty() {
        anyhow::bail!("well-known auth command is empty");
    }
    if metadata.auth.env.trim().is_empty() {
        anyhow::bail!("well-known auth env key is empty");
    }
    Ok(metadata)
}

pub(crate) async fn run_well_known_auth_command(command: &[String]) -> anyhow::Result<String> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("well-known auth command is empty"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("well-known auth command did not expose stdout"))?;
    let mut token = String::new();
    stdout.read_to_string(&mut token).await?;
    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("well-known auth command exited with {status}");
    }
    let token = token.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("well-known auth command returned an empty token");
    }
    Ok(token)
}

pub(crate) fn well_known_credential(
    env_key: &str,
    token: &str,
) -> anyhow::Result<ProviderCredential> {
    let env_key = env_key.trim();
    let token = token.trim();
    if env_key.is_empty() {
        anyhow::bail!("well-known auth env key is required");
    }
    if token.is_empty() {
        anyhow::bail!("well-known auth token is required");
    }
    Ok(ProviderCredential::Wellknown {
        key: env_key.to_string(),
        token: token.to_string(),
    })
}

fn normalize_provider_id(provider: &str) -> anyhow::Result<String> {
    let provider = provider.trim().trim_start_matches("@ai-sdk/");
    if provider.is_empty() {
        anyhow::bail!("provider id is required");
    }
    Ok(provider.to_string())
}

fn normalize_store_key(key: &str) -> anyhow::Result<String> {
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("credential key is required");
    }
    Ok(key.to_string())
}

fn normalize_well_known_base_url(url: &str) -> anyhow::Result<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        anyhow::bail!("provider login URL is required");
    }
    let parsed = reqwest::Url::parse(trimmed)?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        anyhow::bail!("provider login URL must use http or https");
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_auth_store_round_trips_api_and_wellknown_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProviderAuthStore::new(dir.path());

        store
            .set(
                "openai",
                ProviderCredential::Api {
                    key: "sk-test".to_string(),
                    metadata: BTreeMap::from([("team".to_string(), "infra".to_string())]),
                },
            )
            .unwrap();
        store
            .set(
                "https://auth.example.com",
                ProviderCredential::Wellknown {
                    key: "OPENCODE_TOKEN".to_string(),
                    token: "token-123".to_string(),
                },
            )
            .unwrap();

        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(store.auth_path()).unwrap()).unwrap();
        assert_eq!(raw["openai"]["type"], "api");
        assert_eq!(raw["openai"]["key"], "sk-test");
        assert_eq!(raw["openai"]["metadata"], json!({ "team": "infra" }));
        assert_eq!(raw["https://auth.example.com"]["type"], "wellknown");
        assert_eq!(raw["https://auth.example.com"]["key"], "OPENCODE_TOKEN");
        assert_eq!(raw["https://auth.example.com"]["token"], "token-123");

        let reloaded = ProviderAuthStore::new(dir.path()).load().unwrap();
        assert_eq!(reloaded.len(), 2);
        assert!(matches!(
            reloaded.get("openai"),
            Some(ProviderCredential::Api { key, .. }) if key == "sk-test"
        ));
    }

    #[test]
    fn provider_logout_removes_only_the_named_credential() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProviderAuthStore::new(dir.path());
        store
            .set("openai", api_key_credential("sk-openai").unwrap())
            .unwrap();
        store
            .set("anthropic", api_key_credential("sk-ant").unwrap())
            .unwrap();

        assert!(store.remove("openai").unwrap());

        let credentials = store.load().unwrap();
        assert!(!credentials.contains_key("openai"));
        assert!(credentials.contains_key("anthropic"));
    }

    #[test]
    fn choose_provider_login_mode_trims_well_known_urls_and_provider_ids() {
        assert_eq!(
            choose_provider_login_mode(Some("https://auth.example.com///"), Some("openai"))
                .unwrap(),
            ProviderLoginMode::WellKnownUrl {
                base_url: "https://auth.example.com".to_string()
            }
        );
        assert_eq!(
            choose_provider_login_mode(None, Some("  openai  ")).unwrap(),
            ProviderLoginMode::ApiKey {
                provider: "openai".to_string()
            }
        );
        assert!(matches!(
            choose_provider_login_mode(None, None).unwrap(),
            ProviderLoginMode::SelectProvider
        ));
    }

    #[test]
    fn api_key_credentials_reject_empty_keys() {
        assert!(api_key_credential("   ").is_err());
    }
}
