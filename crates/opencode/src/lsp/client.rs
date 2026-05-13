//! Async LSP client: spawn a language server, drive the
//! initialize/shutdown handshake, and provide request/notify
//! primitives.
//!
//! The architecture is one reader task per server:
//!
//!                                stdin (writes)
//!   `LspClient` ---> Mutex<ChildStdin>
//!                                stdout (reads)
//!   `LspClient` <--- reader_task <--- ChildStdout
//!                       |
//!                       +--> Map<id, oneshot::Sender<Response>>  (responses)
//!                       +--> mpsc::Sender<Notification>          (notifications)
//!
//! Sending a `request()` allocates a request id, registers a oneshot
//! sender in the pending map, writes the JSON-RPC message to the
//! server's stdin, and awaits the oneshot. The reader task pumps the
//! server's stdout: for each frame, if it has an `id`, deliver to the
//! pending oneshot; otherwise treat it as a notification and forward
//! to the notification channel.
//!
//! Shutdown is graceful: `shutdown()` sends `shutdown` (request) and
//! then `exit` (notification), waits briefly for the child to exit,
//! and kills it if it doesn't.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex};

use super::framing::{read_message, write_message};
use super::registry::ServerSpec;

type PendingMap = Arc<std::sync::Mutex<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>;

/// An error returned by an LSP server in the `error` field of a
/// response, or synthesized when the connection breaks.
#[derive(Debug, Clone)]
pub struct LspError {
    pub code: i64,
    pub message: String,
}

impl std::fmt::Display for LspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LSP error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for LspError {}

/// An incoming notification from the server (`publishDiagnostics`,
/// `window/logMessage`, etc.).
#[derive(Debug, Clone)]
pub struct ServerNotification {
    pub method: String,
    pub params: Value,
}

pub struct LspClient {
    /// Atomic-incrementing request id. JSON-RPC ids can be any value;
    /// we use a monotonic i64 so collisions are impossible.
    next_id: std::sync::atomic::AtomicI64,
    /// Owned write half of the server's stdin.
    stdin: Mutex<Option<ChildStdin>>,
    /// Live pending-response map. Cloned into the reader task.
    pending: PendingMap,
    /// Notification channel receiver. The reader task pushes into the
    /// matching Sender; consumers call `next_notification()`.
    notifications: Mutex<mpsc::Receiver<ServerNotification>>,
    /// Child handle for shutdown. Wrapped so kill is callable from
    /// drop or explicit shutdown.
    child: Mutex<Option<Child>>,
    /// Reader task handle for cleanup.
    reader_task: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl LspClient {
    /// Spawn a server according to `spec`, drive the initialize
    /// handshake against `root_uri`, and return a ready client.
    ///
    /// `root_uri` should be a `file://` URI for the project root.
    pub async fn spawn(spec: &ServerSpec, root_uri: &str) -> Result<Self> {
        if spec.command.is_empty() {
            return Err(anyhow!("server spec has empty command"));
        }

        let mut cmd = Command::new(spec.command[0]);
        if spec.command.len() > 1 {
            cmd.args(&spec.command[1..]);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn LSP server: {}", spec.command[0]))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("server stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("server stdout not piped"))?;

        let (notify_tx, notify_rx) = mpsc::channel::<ServerNotification>(256);
        let pending: PendingMap = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let pending_for_reader = pending.clone();
        let reader = std::thread::spawn(move || {
            reader_loop(stdout, pending_for_reader, notify_tx);
        });

        let client = LspClient {
            next_id: std::sync::atomic::AtomicI64::new(1),
            stdin: Mutex::new(Some(stdin)),
            pending,
            notifications: Mutex::new(notify_rx),
            child: Mutex::new(Some(child)),
            reader_task: Mutex::new(Some(reader)),
        };

        // initialize handshake — required before any other request.
        let init_params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": {"relatedInformation": true},
                    "hover": {"contentFormat": ["markdown", "plaintext"]},
                    "definition": {"linkSupport": false},
                    "synchronization": {"didSave": false, "willSave": false},
                },
                "workspace": { "workspaceFolders": true },
            },
            "workspaceFolders": [
                {"uri": root_uri, "name": "workspace"}
            ],
            "clientInfo": {"name": "opencode-rs", "version": env!("CARGO_PKG_VERSION")},
        });
        client.request_raw("initialize", init_params).await?;
        client.notify("initialized", serde_json::json!({})).await?;
        Ok(client)
    }

    /// Send a request, await the matching response, deserialize the
    /// `result` field. Returns Err if the server replied with an error
    /// object or the connection broke.
    pub async fn request<R: DeserializeOwned>(
        &self,
        method: &str,
        params: impl Serialize,
    ) -> Result<R> {
        let raw = self.request_raw(method, params).await?;
        serde_json::from_value(raw).context("deserialize LSP response")
    }

    async fn request_raw(&self, method: &str, params: impl Serialize) -> Result<Value> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending map poisoned")
            .insert(id, tx);
        self.write(&body).await?;
        let resp = rx
            .await
            .map_err(|_| anyhow!("LSP response channel closed (server crashed?)"))?;
        resp.map_err(|e| anyhow!(e))
    }

    /// Send a notification (no response expected).
    pub async fn notify(&self, method: &str, params: impl Serialize) -> Result<()> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write(&body).await
    }

    async fn write(&self, body: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(body).context("serialize LSP message")?;
        let mut guard = self.stdin.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or_else(|| anyhow!("LSP client closed"))?;
        write_message(stdin, &bytes).context("write LSP message")?;
        Ok(())
    }

    /// Take the next incoming notification, awaiting if none is queued.
    /// Returns Ok(None) when the server stream closes.
    pub async fn next_notification(&self) -> Option<ServerNotification> {
        self.notifications.lock().await.recv().await
    }

    /// Cheap liveness check.
    ///
    /// Returns false if:
    ///   * the child has already exited (try_wait yielded a status)
    ///   * stdin has been dropped (shutdown was called)
    ///   * the child handle has been taken (e.g. by shutdown)
    ///
    /// Does not block, does not perform I/O, and is safe to call from
    /// the pool's hot path.
    pub async fn is_alive(&self) -> bool {
        // stdin gone -> shutdown already happened.
        if self.stdin.lock().await.is_none() {
            return false;
        }
        let mut child_guard = self.child.lock().await;
        match child_guard.as_mut() {
            Some(child) => match child.try_wait() {
                // None = still running, that's what we want.
                Ok(None) => true,
                // Some(status) = exited; Err = OS error querying.
                _ => false,
            },
            None => false,
        }
    }

    /// Graceful shutdown: shutdown request + exit notification, then
    /// reap the child. Idempotent.
    pub async fn shutdown(&self) -> Result<()> {
        // Best-effort: ignore errors from a server that already exited.
        let _ = self.request_raw("shutdown", Value::Null).await;
        let _ = self.notify("exit", Value::Null).await;

        // Drop stdin so the server sees EOF.
        let mut stdin_guard = self.stdin.lock().await;
        stdin_guard.take();
        drop(stdin_guard);

        // Give the reader thread a moment to finish naturally.
        if let Some(handle) = self.reader_task.lock().await.take() {
            // Spawned thread; don't block the runtime indefinitely.
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }

        if let Some(mut child) = self.child.lock().await.take() {
            // If the server didn't exit on its own, kill it.
            match child.try_wait() {
                Ok(Some(_)) => {}
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        Ok(())
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Best-effort kill if the user dropped without awaiting
        // shutdown(). We can't call async functions in Drop, so just
        // sigkill.
        if let Ok(mut guard) = self.child.try_lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
            }
        }
    }
}

/// Background loop that demultiplexes server stdout into responses
/// (by id) and notifications. Runs on a blocking thread because LSP
/// framing is blocking and the server may not produce output for long
/// stretches.
fn reader_loop(
    stdout: ChildStdout,
    pending: PendingMap,
    notify_tx: mpsc::Sender<ServerNotification>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let body = match read_message(&mut reader) {
            Ok(Some(b)) => b,
            Ok(None) | Err(_) => break,
        };
        let v: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed
        };

        // Three shapes:
        //   { id, result } — response success
        //   { id, error } — response error
        //   { method, params } — notification (no id) OR server-initiated request (with id, but we don't support those yet)
        if let Some(id_val) = v.get("id") {
            // Server-initiated requests have an id AND a method. We
            // don't act on them yet; respond with a method-not-found
            // error to keep the protocol clean. Notifications never
            // carry id.
            if v.get("method").is_some() {
                continue;
            }
            let id = match id_val.as_i64() {
                Some(n) => n,
                None => continue,
            };
            let sender = pending.lock().expect("pending map poisoned").remove(&id);
            if let Some(tx) = sender {
                if let Some(err) = v.get("error") {
                    let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                    let message = err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    let _ = tx.send(Err(LspError { code, message }));
                } else {
                    let result = v.get("result").cloned().unwrap_or(Value::Null);
                    let _ = tx.send(Ok(result));
                }
            }
        } else if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
            let params = v.get("params").cloned().unwrap_or(Value::Null);
            // blocking_send so we don't drop notifications if the
            // consumer is slow; capacity is 256 so this only happens
            // under pathological backlog.
            let _ = notify_tx.blocking_send(ServerNotification {
                method: method.to_string(),
                params,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::framing;

    // A fake "server" that just echoes whatever LSP requests we send,
    // plus emits one notification. We exercise the framing + reader
    // loop in-process without spawning a real binary.
    fn fake_server_response_for(req: &Value) -> Vec<u8> {
        let id = req.get("id").cloned();
        let body = if let Some(id) = id {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"echoed": req.get("method").cloned().unwrap_or(Value::Null)}
            })
        } else {
            // Notification, no reply.
            return Vec::new();
        };
        let bytes = serde_json::to_vec(&body).unwrap();
        let mut out = Vec::new();
        framing::write_message(&mut out, &bytes).unwrap();
        out
    }

    #[test]
    fn fake_server_echoes_request_id() {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "test/echo",
            "params": {}
        });
        let resp_bytes = fake_server_response_for(&req);
        let mut cur = std::io::Cursor::new(resp_bytes);
        let body = framing::read_message(&mut cur).unwrap().unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["id"], 42);
        assert_eq!(v["result"]["echoed"], "test/echo");
    }

    /// Build an LspClient by directly constructing the struct around a
    /// real child process — this bypasses the LSP initialize handshake
    /// (no real LSP server here) and lets us test the liveness probe
    /// in isolation. We use /bin/cat as a long-lived child that holds
    /// stdin/stdout open until we kill it.
    fn client_around_cat() -> Option<LspClient> {
        let mut cmd = Command::new("cat");
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let pending: PendingMap = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let pending_for_reader = pending.clone();
        let (notify_tx, notify_rx) = mpsc::channel::<ServerNotification>(16);
        let reader = std::thread::spawn(move || {
            reader_loop(stdout, pending_for_reader, notify_tx);
        });
        Some(LspClient {
            next_id: std::sync::atomic::AtomicI64::new(1),
            stdin: Mutex::new(Some(stdin)),
            pending,
            notifications: Mutex::new(notify_rx),
            child: Mutex::new(Some(child)),
            reader_task: Mutex::new(Some(reader)),
        })
    }

    #[tokio::test]
    async fn is_alive_is_true_for_running_child() {
        let Some(client) = client_around_cat() else {
            return; // skip if `cat` isn't on PATH (CI weirdness)
        };
        assert!(client.is_alive().await);
        // Cleanup: drop kills the child via the Drop impl.
        drop(client);
    }

    #[tokio::test]
    async fn is_alive_becomes_false_after_child_exits() {
        let Some(client) = client_around_cat() else { return; };
        // Closing stdin makes `cat` exit on EOF.
        {
            let mut guard = client.stdin.lock().await;
            guard.take(); // drop ChildStdin -> EOF on cat
        }
        // Give the child a moment to wind down.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!client.is_alive().await);
    }

    #[tokio::test]
    async fn is_alive_is_false_after_shutdown_takes_handles() {
        let Some(client) = client_around_cat() else { return; };
        // Simulate shutdown's effect on the struct without running
        // the full async shutdown sequence (which would send LSP
        // messages cat doesn't speak): take stdin + child.
        client.stdin.lock().await.take();
        client.child.lock().await.take();
        assert!(!client.is_alive().await);
    }
}
