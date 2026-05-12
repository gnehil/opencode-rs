//! High-level "give me diagnostics for this file" wrapper over `LspClient`.
//!
//! Flow:
//!   1. Look up the server for the file's language. If none, return Err.
//!   2. Spawn it (with the workspace dir as rootUri).
//!   3. Send textDocument/didOpen with the file's current contents.
//!   4. Drain notifications until we see a `publishDiagnostics` for
//!      this file's URI. Time out after a few seconds so a hung server
//!      doesn't block the agent forever.
//!   5. Shutdown the server and return the diagnostics.
//!
//! We tear the server down each call. That's wasteful for high-frequency
//! polling, but `tool/lsp.rs` invocations are user-initiated and rare,
//! and per-call spawning keeps the failure mode simple (no shared state
//! to corrupt). A pooled-server design is a follow-up.
//!
//! Tested with rust-analyzer in mind, but the protocol path is generic.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use super::client::LspClient;
use super::registry;

#[derive(Debug, Clone, Deserialize)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Option<u32>,
    pub code: Option<serde_json::Value>,
    pub source: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Diagnostic {
    /// Severity label per LSP spec: 1=Error, 2=Warning, 3=Information,
    /// 4=Hint. Missing severity is treated as Error (LSP default).
    pub fn severity_label(&self) -> &'static str {
        match self.severity {
            Some(1) | None => "error",
            Some(2) => "warning",
            Some(3) => "info",
            Some(4) => "hint",
            _ => "unknown",
        }
    }

    pub fn format_line(&self, path: &Path) -> String {
        let code = match &self.code {
            Some(serde_json::Value::String(s)) => format!(" [{}]", s),
            Some(other) if !other.is_null() => format!(" [{}]", other),
            _ => String::new(),
        };
        format!(
            "{}:{}:{} [{}]{}: {}",
            path.display(),
            self.range.start.line + 1, // LSP is 0-indexed; human-readable is 1-indexed.
            self.range.start.character + 1,
            self.severity_label(),
            code,
            self.message,
        )
    }
}

/// Fetch diagnostics for one file via the appropriate LSP server.
///
/// `workspace_root` should be the directory the user considers the
/// project root (usually `ctx.working_dir`). We spawn the server with
/// that as `rootUri` so it picks up the right config.
///
/// Returns the raw diagnostics list. `tool/lsp.rs` formats them.
pub async fn fetch(file_path: &Path, workspace_root: &Path) -> Result<Vec<Diagnostic>> {
    let spec = registry::for_path(file_path)
        .ok_or_else(|| anyhow!("no LSP server registered for {}", file_path.display()))?;

    let file_text = std::fs::read_to_string(file_path)
        .with_context(|| format!("read source file: {}", file_path.display()))?;
    let file_uri = path_to_uri(file_path)?;
    let root_uri = path_to_uri(workspace_root)?;

    let client = LspClient::spawn(spec, &root_uri)
        .await
        .with_context(|| format!("spawn LSP server: {}", spec.command[0]))?;

    // Open the document. The server is now obligated (per LSP spec) to
    // publish diagnostics for it.
    client
        .notify(
            "textDocument/didOpen",
            serde_json::json!({
                "textDocument": {
                    "uri": file_uri,
                    "languageId": spec.language_id,
                    "version": 1,
                    "text": file_text,
                }
            }),
        )
        .await?;

    // Drain notifications. rust-analyzer emits multiple
    // publishDiagnostics events as it processes the file: first an
    // empty one (clearing prior state), then the real one once
    // analysis completes. We take the LAST one we see within the
    // timeout window.
    let timeout = Duration::from_secs(15);
    let diagnostics = tokio::time::timeout(timeout, async {
        let mut latest: Option<Vec<Diagnostic>> = None;
        let mut idle_after_first = std::pin::pin!(tokio::time::sleep(Duration::from_secs(60 * 60)));
        loop {
            tokio::select! {
                notif = client.next_notification() => {
                    let Some(notif) = notif else { break; };
                    if notif.method == "textDocument/publishDiagnostics" {
                        let uri = notif.params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                        if uri != file_uri { continue; }
                        let diags: Vec<Diagnostic> = serde_json::from_value(
                            notif.params.get("diagnostics").cloned().unwrap_or(serde_json::Value::Null),
                        ).unwrap_or_default();
                        latest = Some(diags);
                        // After first diagnostic, give the server a short
                        // window to send a follow-up (rust-analyzer
                        // typically emits 2-3 events for a single file as
                        // it processes deps). Reset the idle timer.
                        idle_after_first.as_mut().reset(tokio::time::Instant::now() + Duration::from_millis(400));
                    }
                }
                _ = idle_after_first.as_mut(), if latest.is_some() => {
                    break;
                }
            }
        }
        latest.unwrap_or_default()
    })
    .await;

    // Shut the server down before returning so processes don't pile up.
    let _ = client.shutdown().await;

    diagnostics.map_err(|_| anyhow!("LSP server did not publish diagnostics within 15s"))
}

/// Convert a filesystem path to a `file://` URI per RFC 8089 §3, well
/// enough for what LSP servers expect.
fn path_to_uri(path: &Path) -> Result<String> {
    let abs = path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", path.display()))?;
    let s = abs.to_string_lossy();
    // On Unix, paths start with "/", so "file://" + path is correct.
    // On Windows, paths start with "C:\..." → "file:///C:/...". We're
    // not currently supporting Windows; assume Unix-shaped paths.
    if s.starts_with('/') {
        Ok(format!("file://{}", s))
    } else {
        Ok(format!("file:///{}", s.replace('\\', "/")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn diag(severity: u32, code: serde_json::Value, message: &str) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position { line: 9, character: 4 },
                end: Position { line: 9, character: 10 },
            },
            severity: Some(severity),
            code: if code.is_null() { None } else { Some(code) },
            source: None,
            message: message.to_string(),
        }
    }

    #[test]
    fn severity_label_known_values() {
        assert_eq!(diag(1, serde_json::Value::Null, "x").severity_label(), "error");
        assert_eq!(diag(2, serde_json::Value::Null, "x").severity_label(), "warning");
        assert_eq!(diag(3, serde_json::Value::Null, "x").severity_label(), "info");
        assert_eq!(diag(4, serde_json::Value::Null, "x").severity_label(), "hint");
    }

    #[test]
    fn missing_severity_defaults_to_error() {
        let d = Diagnostic {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 0 },
            },
            severity: None,
            code: None,
            source: None,
            message: "x".to_string(),
        };
        assert_eq!(d.severity_label(), "error");
    }

    #[test]
    fn format_line_is_one_indexed() {
        let d = diag(1, serde_json::json!("E0308"), "type mismatch");
        let line = d.format_line(&PathBuf::from("src/foo.rs"));
        // LSP positions are 0-indexed; human-readable is 1-indexed.
        // Range start was line:9 char:4 → display as 10:5.
        assert_eq!(line, "src/foo.rs:10:5 [error] [E0308]: type mismatch");
    }

    #[test]
    fn format_line_with_numeric_code() {
        let d = diag(2, serde_json::json!(42), "unused");
        let line = d.format_line(&PathBuf::from("a.rs"));
        assert!(line.contains("[42]"), "{line}");
    }

    #[test]
    fn format_line_without_code() {
        let d = diag(2, serde_json::Value::Null, "unused");
        let line = d.format_line(&PathBuf::from("a.rs"));
        assert_eq!(line, "a.rs:10:5 [warning]: unused");
    }
}
