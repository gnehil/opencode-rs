//! High-level request/response LSP operations: hover, goToDefinition,
//! findReferences, documentSymbol.
//!
//! Each operation follows the same shape as `diagnostics::fetch`:
//! spawn server → didOpen the source file → send the request → parse
//! the response → shutdown. We respawn the server for each call. As
//! noted in `diagnostics.rs`, this is wasteful at scale but keeps the
//! failure mode simple. A server pool keyed by (workspace, language)
//! is a follow-up.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::client::LspClient;
use super::registry;

/// Spawn the server, open the file, and return both. Caller drives
/// the actual LSP request and calls shutdown() on the client.
async fn prepare(file_path: &Path, workspace_root: &Path) -> Result<(LspClient, String, &'static str)> {
    let spec = registry::for_path(file_path)
        .ok_or_else(|| anyhow!("no LSP server registered for {}", file_path.display()))?;
    let file_text = std::fs::read_to_string(file_path)
        .with_context(|| format!("read source file: {}", file_path.display()))?;
    let file_uri = super::diagnostics::path_to_uri(file_path)?;
    let root_uri = super::diagnostics::path_to_uri(workspace_root)?;

    let client = LspClient::spawn(spec, &root_uri).await?;
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
    Ok((client, file_uri, spec.language_id))
}

/// `textDocument/hover` — return the hover text (markdown or plain).
pub async fn hover(
    file_path: &Path,
    workspace_root: &Path,
    line: u32,
    character: u32,
) -> Result<String> {
    let (client, file_uri, _lang) = prepare(file_path, workspace_root).await?;

    #[derive(Deserialize)]
    struct HoverResponse {
        contents: Value,
    }

    let result: Option<HoverResponse> = client
        .request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
            }),
        )
        .await
        .ok();

    let _ = client.shutdown().await;

    let Some(resp) = result else {
        return Ok(format!("No hover info for {}:{}:{}", file_path.display(), line + 1, character + 1));
    };
    Ok(extract_markup(&resp.contents))
}

/// Hover contents can be:
///   * a string (plain text)
///   * { kind, value } MarkupContent
///   * { language, value } MarkedString (legacy)
///   * an array of any of the above
/// Flatten to plain text for tool output.
fn extract_markup(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get("value") {
                return s.clone();
            }
            v.to_string()
        }
        Value::Array(items) => items
            .iter()
            .map(extract_markup)
            .collect::<Vec<_>>()
            .join("\n---\n"),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

/// `textDocument/definition` — return locations of the symbol's definition.
pub async fn goto_definition(
    file_path: &Path,
    workspace_root: &Path,
    line: u32,
    character: u32,
) -> Result<Vec<Location>> {
    let (client, file_uri, _lang) = prepare(file_path, workspace_root).await?;
    let result: Result<Value, _> = client
        .request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
            }),
        )
        .await;
    let _ = client.shutdown().await;
    Ok(parse_locations(result.unwrap_or(Value::Null)))
}

/// `textDocument/references` — return all locations referring to the symbol.
pub async fn find_references(
    file_path: &Path,
    workspace_root: &Path,
    line: u32,
    character: u32,
) -> Result<Vec<Location>> {
    let (client, file_uri, _lang) = prepare(file_path, workspace_root).await?;
    let result: Result<Value, _> = client
        .request(
            "textDocument/references",
            serde_json::json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
                "context": {"includeDeclaration": true},
            }),
        )
        .await;
    let _ = client.shutdown().await;
    Ok(parse_locations(result.unwrap_or(Value::Null)))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Location {
    pub uri: String,
    pub range: super::diagnostics::Range,
}

impl Location {
    /// Format as `path:line:col` (1-indexed), stripping the `file://`
    /// scheme so the result is greppable.
    pub fn format(&self) -> String {
        let path = self.uri.strip_prefix("file://").unwrap_or(&self.uri);
        format!(
            "{}:{}:{}",
            path,
            self.range.start.line + 1,
            self.range.start.character + 1
        )
    }
}

/// definition/references can return:
///   * null
///   * a single Location object
///   * a single LocationLink object
///   * an array of any of the above
fn parse_locations(v: Value) -> Vec<Location> {
    match v {
        Value::Null => Vec::new(),
        Value::Object(_) => parse_one(&v).into_iter().collect(),
        Value::Array(items) => items.iter().filter_map(parse_one).collect(),
        _ => Vec::new(),
    }
}

fn parse_one(v: &Value) -> Option<Location> {
    // LocationLink uses `targetUri` + `targetSelectionRange`; Location
    // uses `uri` + `range`. Normalize both.
    if let Some(uri) = v.get("uri").and_then(|u| u.as_str()) {
        let range = v.get("range").cloned()?;
        return serde_json::from_value(serde_json::json!({"uri": uri, "range": range})).ok();
    }
    if let Some(uri) = v.get("targetUri").and_then(|u| u.as_str()) {
        let range = v
            .get("targetSelectionRange")
            .or_else(|| v.get("targetRange"))
            .cloned()?;
        return serde_json::from_value(serde_json::json!({"uri": uri, "range": range})).ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_markup_plain_string() {
        assert_eq!(extract_markup(&Value::String("hello".to_string())), "hello");
    }

    #[test]
    fn extract_markup_object_with_value_field() {
        let v = serde_json::json!({"kind": "markdown", "value": "**bold**"});
        assert_eq!(extract_markup(&v), "**bold**");
    }

    #[test]
    fn extract_markup_legacy_marked_string_array() {
        let v = serde_json::json!([
            {"language": "rust", "value": "fn foo()"},
            "Documentation"
        ]);
        assert_eq!(extract_markup(&v), "fn foo()\n---\nDocumentation");
    }

    #[test]
    fn parse_locations_handles_single_location() {
        let v = serde_json::json!({
            "uri": "file:///foo.rs",
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 5}}
        });
        let locs = parse_locations(v);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "file:///foo.rs");
    }

    #[test]
    fn parse_locations_handles_array() {
        let v = serde_json::json!([
            {"uri": "file:///a.rs", "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 3}}},
            {"uri": "file:///b.rs", "range": {"start": {"line": 5, "character": 4}, "end": {"line": 5, "character": 7}}}
        ]);
        let locs = parse_locations(v);
        assert_eq!(locs.len(), 2);
    }

    #[test]
    fn parse_locations_handles_location_link() {
        // LSP `textDocument/definition` can return LocationLink form
        // when clientCapabilities.textDocument.definition.linkSupport
        // is true. We don't currently set that, but parse it defensively.
        let v = serde_json::json!({
            "targetUri": "file:///foo.rs",
            "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 5, "character": 0}},
            "targetSelectionRange": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 6}}
        });
        let locs = parse_locations(v);
        assert_eq!(locs.len(), 1);
        // We prefer targetSelectionRange (the symbol itself, not the
        // whole defining span).
        assert_eq!(locs[0].range.start.character, 3);
    }

    #[test]
    fn parse_locations_handles_null() {
        assert!(parse_locations(Value::Null).is_empty());
    }

    #[test]
    fn location_format_strips_file_scheme() {
        let loc: Location = serde_json::from_value(serde_json::json!({
            "uri": "file:///workspace/foo.rs",
            "range": {"start": {"line": 9, "character": 4}, "end": {"line": 9, "character": 8}}
        })).unwrap();
        assert_eq!(loc.format(), "/workspace/foo.rs:10:5");
    }
}
