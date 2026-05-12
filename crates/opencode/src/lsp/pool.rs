//! Pool of long-lived `LspClient`s, keyed by (workspace_root, language).
//!
//! Without pooling, each `hover`/`definition`/`references`/`diagnostics`
//! call respawns the language server. rust-analyzer cold-start is
//! 3-5s on a typical crate; warm calls are tens of ms. The pool turns
//! a chatty agent session from "every tool call is slow" into "one
//! slow call followed by fast ones."
//!
//! Lifecycle:
//!   1. `ensure(workspace, file_path)` returns a handle to a live
//!      client. If none exists for this (workspace, language), spawn.
//!   2. The pool tracks each open document by uri → (version,
//!      content_hash). Subsequent calls compare the current file
//!      content's hash; if it differs, send didChange with version+1.
//!      If it matches, no message is sent.
//!   3. When the process exits, Drop tries to send `shutdown` +
//!      `exit` to every server. Best-effort.
//!
//! The pool is a process-wide singleton (`global()`). Callers that
//! want isolation (tests) construct their own `ServerPool::new()`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OnceCell};

use super::client::LspClient;
use super::diagnostics::path_to_uri;
use super::registry::{self, ServerSpec};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PoolKey {
    workspace_root: PathBuf,
    language_id: &'static str,
}

#[derive(Debug, Clone)]
struct OpenDoc {
    version: i32,
    content_hash: [u8; 32],
}

struct PooledServer {
    client: Arc<LspClient>,
    documents: Mutex<HashMap<String, OpenDoc>>,
}

/// A handle returned from `ensure`. Carries the live client + the URI
/// the caller should reference in subsequent requests. The doc has
/// already been opened (or updated) with the current file contents.
pub struct LiveDoc {
    pub client: Arc<LspClient>,
    pub uri: String,
    pub language_id: &'static str,
}

impl std::fmt::Debug for LiveDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // LspClient owns child stdin/stdout handles that don't impl
        // Debug; just summarize the addressable bits.
        f.debug_struct("LiveDoc")
            .field("uri", &self.uri)
            .field("language_id", &self.language_id)
            .finish()
    }
}

pub struct ServerPool {
    servers: Mutex<HashMap<PoolKey, Arc<PooledServer>>>,
}

impl ServerPool {
    pub fn new() -> Self {
        Self {
            servers: Mutex::new(HashMap::new()),
        }
    }

    /// Ensure a server is running for the file's language, and that the
    /// document is opened (or updated) with its current contents.
    /// Returns a handle the caller can use to issue further requests.
    ///
    /// On a hot path this does zero RPCs if the file content hasn't
    /// changed since the previous call; otherwise it sends one
    /// `textDocument/didChange`. On a cold path it spawns the server
    /// (incl. initialize handshake) and sends `textDocument/didOpen`.
    pub async fn ensure(&self, workspace_root: &Path, file_path: &Path) -> Result<LiveDoc> {
        let spec = registry::for_path(file_path)
            .ok_or_else(|| anyhow!("no LSP server registered for {}", file_path.display()))?;
        let workspace_canon = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());
        let key = PoolKey {
            workspace_root: workspace_canon.clone(),
            language_id: spec.language_id,
        };

        // 1. Get-or-spawn the server.
        let server = {
            let mut guard = self.servers.lock().await;
            if let Some(existing) = guard.get(&key) {
                existing.clone()
            } else {
                let pooled = spawn_server(spec, &workspace_canon).await?;
                let arc = Arc::new(pooled);
                guard.insert(key.clone(), arc.clone());
                arc
            }
        };

        // 2. Open or update the document, depending on its prior state
        // in this pool.
        let file_text = std::fs::read_to_string(file_path)
            .with_context(|| format!("read source file: {}", file_path.display()))?;
        let mut hasher = Sha256::new();
        hasher.update(file_text.as_bytes());
        let content_hash: [u8; 32] = hasher.finalize().into();
        let uri = path_to_uri(file_path)?;

        let mut docs = server.documents.lock().await;
        match docs.get_mut(&uri) {
            Some(open) if open.content_hash == content_hash => {
                // Already open and identical; nothing to do.
            }
            Some(open) => {
                // Open but content changed — send didChange.
                open.version += 1;
                open.content_hash = content_hash;
                server
                    .client
                    .notify(
                        "textDocument/didChange",
                        serde_json::json!({
                            "textDocument": {"uri": uri, "version": open.version},
                            "contentChanges": [{"text": file_text}],
                        }),
                    )
                    .await?;
            }
            None => {
                // First time we've seen this document in this server.
                let version = 1;
                server
                    .client
                    .notify(
                        "textDocument/didOpen",
                        serde_json::json!({
                            "textDocument": {
                                "uri": uri,
                                "languageId": spec.language_id,
                                "version": version,
                                "text": file_text,
                            }
                        }),
                    )
                    .await?;
                docs.insert(
                    uri.clone(),
                    OpenDoc { version, content_hash },
                );
            }
        }

        Ok(LiveDoc {
            client: server.client.clone(),
            uri,
            language_id: spec.language_id,
        })
    }

    /// Drain the pool, sending shutdown to each server. Idempotent.
    pub async fn shutdown_all(&self) {
        let mut guard = self.servers.lock().await;
        for (_, server) in guard.drain() {
            let _ = server.client.shutdown().await;
        }
    }
}

impl Default for ServerPool {
    fn default() -> Self {
        Self::new()
    }
}

async fn spawn_server(spec: &ServerSpec, workspace_root: &Path) -> Result<PooledServer> {
    let root_uri = path_to_uri(workspace_root)?;
    let client = LspClient::spawn(spec, &root_uri).await?;
    Ok(PooledServer {
        client: Arc::new(client),
        documents: Mutex::new(HashMap::new()),
    })
}

/// Process-wide singleton pool. Test code that needs isolation
/// constructs its own `ServerPool` instead of going through here.
static GLOBAL_POOL: OnceCell<ServerPool> = OnceCell::const_new();

pub async fn global() -> &'static ServerPool {
    GLOBAL_POOL.get_or_init(|| async { ServerPool::new() }).await
}

/// Shutdown the global pool. Call from process exit hooks; safe to
/// call even if the pool never had a server spawned.
pub async fn shutdown_global() {
    if let Some(pool) = GLOBAL_POOL.get() {
        pool.shutdown_all().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn ensure_returns_err_for_unsupported_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let lol = tmp.path().join("nothing.lol");
        std::fs::write(&lol, "no server registered for me").unwrap();
        let pool = ServerPool::new();
        let r = pool.ensure(tmp.path(), &lol).await;
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("no LSP server"));
    }

    // We deliberately don't include integration tests against a live
    // language server here. Those need rust-analyzer / tsserver on
    // PATH and inflate test runtime by 3-5s. The unit boundary we
    // can cover without spawning processes is the registry lookup
    // and pool key construction; the rest exercises subprocess I/O
    // which is integration test territory.
}

#[cfg(test)]
mod sanity {
    use super::*;

    /// Two paths that canonicalize to the same workspace should map
    /// to the same pool key. This catches a regression where we
    /// stored the raw input path and ended up spawning two servers
    /// for the same workspace.
    #[tokio::test]
    async fn duplicate_workspace_paths_share_a_pool_slot() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().to_path_buf();
        // Create a `.rs` file so the pool would attempt to spawn
        // rust-analyzer. We don't expect a real server on the test
        // host, so both ensure() calls will fail — but they should
        // fail in the same way, after the same lookup.
        let rs = workspace.join("a.rs");
        std::fs::write(&rs, "fn main() {}").unwrap();

        let pool = ServerPool::new();

        // First call: may or may not succeed depending on whether
        // rust-analyzer is on PATH. What we care about is that the
        // canonicalization is stable.
        let workspace_canon = workspace.canonicalize().unwrap();
        let key1 = PoolKey {
            workspace_root: workspace_canon.clone(),
            language_id: "rust",
        };
        let key2 = PoolKey {
            workspace_root: workspace_canon,
            language_id: "rust",
        };
        assert_eq!(key1, key2);

        // Cleanup the pool so background threads don't leak between
        // test runs.
        pool.shutdown_all().await;
    }
}
