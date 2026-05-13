use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AuthInfo {
    Api {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<HashMap<String, String>>,
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
        extra: HashMap<String, serde_json::Value>,
    },
}

impl AuthInfo {
    pub fn api(key: impl Into<String>) -> Self {
        Self::Api {
            key: key.into(),
            metadata: None,
        }
    }

    pub fn api_with_metadata(key: impl Into<String>, metadata: HashMap<String, String>) -> Self {
        Self::Api {
            key: key.into(),
            metadata: (!metadata.is_empty()).then_some(metadata),
        }
    }

    pub fn wellknown(key: impl Into<String>, token: impl Into<String>) -> Self {
        Self::Wellknown {
            key: key.into(),
            token: token.into(),
        }
    }

    pub fn api_key(&self) -> Option<&str> {
        match self {
            Self::Api { key, .. } => Some(key),
            _ => None,
        }
    }
}

pub struct AuthStore {
    filepath: PathBuf,
    entries: Arc<RwLock<HashMap<String, AuthInfo>>>,
}

impl AuthStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            filepath: data_dir.join("auth.json"),
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.filepath
    }

    pub async fn load(&self) -> anyhow::Result<()> {
        if self.filepath.exists() {
            let text = std::fs::read_to_string(&self.filepath)?;
            let data: HashMap<String, AuthInfo> = serde_json::from_str(&text)?;
            let mut entries = self.entries.write().await;
            *entries = data;
        }
        Ok(())
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        let snapshot = self.entries.read().await.clone();
        self.persist(&snapshot)
    }

    pub async fn all(&self) -> HashMap<String, AuthInfo> {
        self.entries.read().await.clone()
    }

    pub async fn get(&self, key: &str) -> Option<AuthInfo> {
        self.entries.read().await.get(key).cloned()
    }

    pub async fn set(&self, key: &str, value: AuthInfo) -> anyhow::Result<()> {
        let snapshot = {
            let mut entries = self.entries.write().await;
            entries.insert(key.to_string(), value);
            entries.clone()
        };
        self.persist(&snapshot)
    }

    pub async fn remove(&self, key: &str) -> anyhow::Result<()> {
        let snapshot = {
            let mut entries = self.entries.write().await;
            entries.remove(key);
            entries.clone()
        };
        self.persist(&snapshot)
    }

    fn persist(&self, entries: &HashMap<String, AuthInfo>) -> anyhow::Result<()> {
        if let Some(parent) = self.filepath.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(entries)?;
        std::fs::write(&self.filepath, format!("{}\n", text))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn auth_store_persists_and_loads_api_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let data_dir = temp.path().join("missing").join("opencode");
        let store = AuthStore::new(data_dir.clone());

        store
            .set("openai", AuthInfo::api("sk-test"))
            .await
            .expect("set should persist credentials");

        let reloaded = AuthStore::new(data_dir);
        reloaded.load().await.expect("load should read credentials");

        assert_eq!(reloaded.get("openai").await, Some(AuthInfo::api("sk-test")));
    }

    #[tokio::test]
    async fn auth_store_serializes_wellknown_credentials_like_typescript() {
        let temp = tempfile::tempdir().unwrap();
        let store = AuthStore::new(temp.path().to_path_buf());

        store
            .set(
                "https://auth.example.com",
                AuthInfo::wellknown("EXAMPLE_TOKEN", "token-value"),
            )
            .await
            .expect("set should persist credentials");

        let text = std::fs::read_to_string(temp.path().join("auth.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["https://auth.example.com"]["type"], "wellknown");
        assert_eq!(value["https://auth.example.com"]["key"], "EXAMPLE_TOKEN");
        assert_eq!(value["https://auth.example.com"]["token"], "token-value");
    }

    #[tokio::test]
    async fn auth_store_remove_persists_empty_map() {
        let temp = tempfile::tempdir().unwrap();
        let store = AuthStore::new(temp.path().to_path_buf());

        store.set("openai", AuthInfo::api("sk-test")).await.unwrap();
        store.remove("openai").await.unwrap();

        let reloaded = AuthStore::new(temp.path().to_path_buf());
        reloaded.load().await.unwrap();
        assert_eq!(reloaded.get("openai").await, None);
        assert!(reloaded.all().await.is_empty());
    }
}
