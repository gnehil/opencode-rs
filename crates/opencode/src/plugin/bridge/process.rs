//! Subprocess manager for the external-plugin JS host.
//!
//! Spawns node/bun running [`HOST_SCRIPT`], performs the init handshake, and
//! exposes [`trigger`](PluginBridge::trigger) / [`notify`](PluginBridge::notify)
//! for routing hook calls. A background reader task parses host stdout into
//! [`HostEvent`]s and correlates `trigger` responses by id.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use super::protocol::{
    HostEvent, HostRequest, LoadedPlugin, PluginInputData, PluginLoadError, PluginToLoad,
    HOST_SCRIPT,
};

/// A JavaScript runtime capable of running the plugin host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsRuntime {
    Bun,
    Node,
}

impl JsRuntime {
    fn command(&self) -> &'static str {
        match self {
            JsRuntime::Bun => "bun",
            JsRuntime::Node => "node",
        }
    }
}

/// Locate a JavaScript runtime for the plugin host, preferring `bun` (the
/// runtime opencode plugins are authored against) and falling back to `node`.
pub fn detect_js_runtime() -> Option<(JsRuntime, PathBuf)> {
    for runtime in [JsRuntime::Bun, JsRuntime::Node] {
        if let Ok(path) = which::which(runtime.command()) {
            return Some((runtime, path));
        }
    }
    None
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// A running plugin host subprocess.
pub struct PluginBridge {
    child: Child,
    stdin: Mutex<ChildStdin>,
    pending: PendingMap,
    next_id: AtomicU64,
    loaded: Vec<LoadedPlugin>,
    errors: Vec<PluginLoadError>,
    // Kept alive so the host script file is not deleted while the host runs.
    _script: tempfile::TempPath,
}

impl PluginBridge {
    /// Spawn the host, initialize the given plugins, and complete the init
    /// handshake. Returns once every plugin has either loaded or failed.
    pub async fn spawn(
        runtime: JsRuntime,
        plugins: Vec<PluginToLoad>,
        input: PluginInputData,
    ) -> anyhow::Result<Self> {
        let mut script = tempfile::Builder::new()
            .prefix("opencode-plugin-host")
            .suffix(".mjs")
            .tempfile()?;
        {
            use std::io::Write;
            script.write_all(HOST_SCRIPT.as_bytes())?;
            script.flush()?;
        }
        let script_path = script.into_temp_path();

        let mut child = Command::new(runtime.command())
            .arg(&script_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("plugin host stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("plugin host stdout unavailable"))?;
        let mut reader = BufReader::new(stdout).lines();

        write_request(&mut stdin, &HostRequest::Init { plugins, input }).await?;

        // Read until the `ready` handshake completes, surfacing any log lines
        // the host emits while loading plugins.
        let (loaded, errors) = loop {
            let Some(line) = reader.next_line().await? else {
                anyhow::bail!("plugin host exited before init handshake");
            };
            match parse_event(&line) {
                Some(HostEvent::Ready { plugins, errors }) => break (plugins, errors),
                Some(HostEvent::Log { level, message }) => log_host(&level, &message),
                Some(other) => {
                    tracing::warn!("unexpected plugin host event before ready: {other:?}");
                }
                None => {}
            }
        };

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        spawn_reader(reader, Arc::clone(&pending));

        Ok(Self {
            child,
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            loaded,
            errors,
            _script: script_path,
        })
    }

    /// Plugins that initialized successfully, with their registered hooks.
    pub fn loaded_plugins(&self) -> &[LoadedPlugin] {
        &self.loaded
    }

    /// Plugins that failed to import or initialize.
    pub fn load_errors(&self) -> &[PluginLoadError] {
        &self.errors
    }

    /// Whether any loaded plugin registered `hook`.
    pub fn has_hook(&self, hook: &str) -> bool {
        self.loaded
            .iter()
            .any(|plugin| plugin.hooks.iter().any(|name| name == hook))
    }

    /// Invoke a trigger-style hook. `output` is passed to every plugin's hook
    /// in registration order; the mutated result is returned.
    pub async fn trigger(&self, hook: &str, input: Value, output: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let request = HostRequest::Trigger {
            id,
            hook: hook.to_string(),
            input,
            output,
        };
        if let Err(err) = write_request(&mut *self.stdin.lock().await, &request).await {
            self.pending.lock().await.remove(&id);
            return Err(err);
        }

        match rx.await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(error)) => Err(anyhow::anyhow!("plugin hook '{hook}' failed: {error}")),
            Err(_) => anyhow::bail!("plugin host closed while waiting for hook '{hook}'"),
        }
    }

    /// Invoke a one-way notification hook (`event`, `config`). The host does
    /// not reply, so this returns once the request is written.
    pub async fn notify(&self, hook: &str, input: Value) -> anyhow::Result<()> {
        let request = HostRequest::Notify {
            hook: hook.to_string(),
            input,
        };
        write_request(&mut *self.stdin.lock().await, &request).await
    }

    /// Ask the host to exit and wait for the subprocess to finish.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        let _ = write_request(&mut *self.stdin.lock().await, &HostRequest::Shutdown).await;
        let _ = self.child.wait().await;
        Ok(())
    }
}

async fn write_request(stdin: &mut ChildStdin, request: &HostRequest) -> anyhow::Result<()> {
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

fn parse_event(line: &str) -> Option<HostEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str::<HostEvent>(trimmed) {
        Ok(event) => Some(event),
        Err(err) => {
            tracing::warn!("unparseable plugin host line: {err}");
            None
        }
    }
}

fn log_host(level: &str, message: &str) {
    match level {
        "error" => tracing::error!("plugin host: {message}"),
        "warn" => tracing::warn!("plugin host: {message}"),
        _ => tracing::info!("plugin host: {message}"),
    }
}

/// Drain host stdout for the lifetime of the bridge, correlating trigger
/// responses to their pending senders and logging everything else.
fn spawn_reader<R>(mut reader: tokio::io::Lines<BufReader<R>>, pending: PendingMap)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => match parse_event(&line) {
                    Some(HostEvent::TriggerResult { id, output }) => {
                        if let Some(tx) = pending.lock().await.remove(&id) {
                            let _ = tx.send(Ok(output));
                        }
                    }
                    Some(HostEvent::TriggerError { id, error }) => {
                        if let Some(tx) = pending.lock().await.remove(&id) {
                            let _ = tx.send(Err(error));
                        }
                    }
                    Some(HostEvent::Log { level, message }) => log_host(&level, &message),
                    Some(HostEvent::Ready { .. }) => {
                        tracing::warn!("unexpected second plugin host ready event");
                    }
                    None => {}
                },
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!("plugin host reader error: {err}");
                    break;
                }
            }
        }
        // The host is gone; fail every in-flight trigger so callers unblock.
        let mut pending = pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err("plugin host closed".to_string()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_plugin(dir: &std::path::Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        format!("file://{}", path.to_string_lossy())
    }

    fn sample_input() -> PluginInputData {
        PluginInputData {
            directory: "/work".to_string(),
            worktree: "/work".to_string(),
            project: json!({ "id": "p1" }),
            server_url: "http://localhost:4096".to_string(),
        }
    }

    #[test]
    fn detect_js_runtime_does_not_panic() {
        let _ = detect_js_runtime();
    }

    #[tokio::test]
    async fn bridge_initializes_and_triggers_hooks() {
        let Some((runtime, _)) = detect_js_runtime() else {
            eprintln!("skipping: no JS runtime available");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let entry = write_plugin(
            dir.path(),
            "plugin.mjs",
            r#"export default async function () {
                return {
                    "tool.execute.before": async (input, output) => {
                        output.args.injectedTool = input.tool
                    },
                }
            }"#,
        );

        let bridge = PluginBridge::spawn(
            runtime,
            vec![PluginToLoad {
                spec: "./plugin.mjs".to_string(),
                entry,
                options: None,
            }],
            sample_input(),
        )
        .await
        .unwrap();

        assert_eq!(bridge.loaded_plugins().len(), 1);
        assert!(bridge.load_errors().is_empty());
        assert!(bridge.has_hook("tool.execute.before"));
        assert!(!bridge.has_hook("config"));

        let output = bridge
            .trigger(
                "tool.execute.before",
                json!({ "tool": "bash", "sessionID": "s1", "callID": "c1" }),
                json!({ "args": {} }),
            )
            .await
            .unwrap();
        assert_eq!(output["args"]["injectedTool"], "bash");

        bridge.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn bridge_reports_plugin_load_errors() {
        let Some((runtime, _)) = detect_js_runtime() else {
            eprintln!("skipping: no JS runtime available");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let entry = write_plugin(dir.path(), "broken.mjs", "this is not valid javascript ===");

        let bridge = PluginBridge::spawn(
            runtime,
            vec![PluginToLoad {
                spec: "./broken.mjs".to_string(),
                entry,
                options: None,
            }],
            sample_input(),
        )
        .await
        .unwrap();

        assert!(bridge.loaded_plugins().is_empty());
        assert_eq!(bridge.load_errors().len(), 1);
        assert_eq!(bridge.load_errors()[0].spec, "./broken.mjs");

        bridge.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn bridge_surfaces_hook_errors() {
        let Some((runtime, _)) = detect_js_runtime() else {
            eprintln!("skipping: no JS runtime available");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let entry = write_plugin(
            dir.path(),
            "thrower.mjs",
            r#"export default async function () {
                return {
                    "tool.execute.before": async () => {
                        throw new Error("hook exploded")
                    },
                }
            }"#,
        );

        let bridge = PluginBridge::spawn(
            runtime,
            vec![PluginToLoad {
                spec: "./thrower.mjs".to_string(),
                entry,
                options: None,
            }],
            sample_input(),
        )
        .await
        .unwrap();

        let err = bridge
            .trigger("tool.execute.before", json!({}), json!({ "args": {} }))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("hook exploded"), "got: {err}");

        bridge.shutdown().await.unwrap();
    }
}
