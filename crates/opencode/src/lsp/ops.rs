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

/// Go through the global pool: ensure the language server is running
/// for this workspace, open (or update) the document. Returns a handle
/// the caller uses to issue further LSP requests. Callers do NOT call
/// shutdown — the pool owns the server's lifetime.
async fn prepare(file_path: &Path, workspace_root: &Path) -> Result<super::pool::LiveDoc> {
    let pool = super::pool::global().await;
    pool.ensure(workspace_root, file_path).await
}

/// `textDocument/hover` — return the hover text (markdown or plain).
pub async fn hover(
    file_path: &Path,
    workspace_root: &Path,
    line: u32,
    character: u32,
) -> Result<String> {
    let live = prepare(file_path, workspace_root).await?;
    let client = &live.client;
    let file_uri = &live.uri;

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

    let Some(resp) = result else {
        return Ok(format!(
            "No hover info for {}:{}:{}",
            file_path.display(),
            line + 1,
            character + 1
        ));
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
    let live = prepare(file_path, workspace_root).await?;
    let client = &live.client;
    let file_uri = &live.uri;
    let result: Result<Value, _> = client
        .request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
            }),
        )
        .await;
    Ok(parse_locations(result.unwrap_or(Value::Null)))
}

/// `textDocument/references` — return all locations referring to the symbol.
pub async fn find_references(
    file_path: &Path,
    workspace_root: &Path,
    line: u32,
    character: u32,
) -> Result<Vec<Location>> {
    let live = prepare(file_path, workspace_root).await?;
    let client = &live.client;
    let file_uri = &live.uri;
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
    Ok(parse_locations(result.unwrap_or(Value::Null)))
}

/// `textDocument/documentSymbol` — symbols defined in this file.
///
/// LSP allows two response shapes here:
///   * SymbolInformation[] (legacy, flat list with location.range).
///   * DocumentSymbol[] (modern, hierarchical with optional children).
///
/// We flatten both into `(name, kind, line)` for a simple tool output.
pub async fn document_symbols(
    file_path: &Path,
    workspace_root: &Path,
) -> Result<Vec<SymbolSummary>> {
    let live = prepare(file_path, workspace_root).await?;
    let client = &live.client;
    let file_uri = &live.uri;
    let result: Result<Value, _> = client
        .request(
            "textDocument/documentSymbol",
            serde_json::json!({
                "textDocument": {"uri": file_uri},
            }),
        )
        .await;
    Ok(parse_symbols(result.unwrap_or(Value::Null)))
}

/// `workspace/symbol` — symbols across the project matching a query.
pub async fn workspace_symbols(
    workspace_root: &Path,
    seed_file: &Path,
    query: &str,
) -> Result<Vec<SymbolSummary>> {
    // `workspace/symbol` is a server-level request, but the LSP server
    // still needs to be initialized for the workspace. We piggy-back
    // on the pool: use any registered file in the workspace to ensure
    // a server is running.
    let live = prepare(seed_file, workspace_root).await?;
    let client = &live.client;
    let result: Result<Value, _> = client
        .request("workspace/symbol", serde_json::json!({ "query": query }))
        .await;
    Ok(parse_symbols(result.unwrap_or(Value::Null)))
}

/// `textDocument/rename` — request a rename + return the per-file edits.
///
/// The server returns a WorkspaceEdit; we render it as a human-readable
/// summary so the agent can decide whether to apply it. We DO NOT
/// apply edits automatically — that would conflict with the agent's
/// own permission and revert flows.
pub async fn rename(
    file_path: &Path,
    workspace_root: &Path,
    line: u32,
    character: u32,
    new_name: &str,
) -> Result<String> {
    let live = prepare(file_path, workspace_root).await?;
    let client = &live.client;
    let file_uri = &live.uri;
    let result: Result<Value, _> = client
        .request(
            "textDocument/rename",
            serde_json::json!({
                "textDocument": {"uri": file_uri},
                "position": {"line": line, "character": character},
                "newName": new_name,
            }),
        )
        .await;
    Ok(format_workspace_edit(
        &result.unwrap_or(Value::Null),
        new_name,
    ))
}

#[derive(Debug, Clone)]
pub struct SymbolSummary {
    pub name: String,
    pub kind: u32,
    pub line: u32,
    /// For workspaceSymbol responses, the location may live in a
    /// different file than the request seed.
    pub uri: String,
}

impl SymbolSummary {
    pub fn format(&self) -> String {
        let path = self.uri.strip_prefix("file://").unwrap_or(&self.uri);
        format!(
            "{}:{} [{}] {}",
            path,
            self.line + 1,
            symbol_kind_label(self.kind),
            self.name
        )
    }
}

/// LSP SymbolKind enum (subset; full list at
/// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#symbolKind).
fn symbol_kind_label(k: u32) -> &'static str {
    match k {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enummember",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "typeparameter",
        _ => "symbol",
    }
}

fn parse_symbols(v: Value) -> Vec<SymbolSummary> {
    let Some(items) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        // SymbolInformation (legacy / workspace symbol):
        //   { name, kind, location: { uri, range } }
        // DocumentSymbol (modern textDocument/documentSymbol):
        //   { name, kind, range, selectionRange, children? }
        if let Some(loc) = item.get("location") {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let uri = loc
                .get("uri")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let line = loc
                .get("range")
                .and_then(|r| r.get("start"))
                .and_then(|s| s.get("line"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            out.push(SymbolSummary {
                name,
                kind,
                line,
                uri,
            });
            continue;
        }
        // DocumentSymbol shape (recurses into children).
        flatten_document_symbol(item, "", &mut out);
    }
    out
}

fn flatten_document_symbol(item: &Value, current_uri: &str, out: &mut Vec<SymbolSummary>) {
    let name = item
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let line = item
        .get("selectionRange")
        .or_else(|| item.get("range"))
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("line"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    out.push(SymbolSummary {
        name,
        kind,
        line,
        uri: current_uri.to_string(),
    });
    if let Some(children) = item.get("children").and_then(|c| c.as_array()) {
        for child in children {
            flatten_document_symbol(child, current_uri, out);
        }
    }
}

/// Render a WorkspaceEdit as a human-readable diff summary. We
/// intentionally don't apply edits; the agent's edit tool owns that
/// (with its diff display, permission checks, etc.).
fn format_workspace_edit(v: &Value, new_name: &str) -> String {
    if v.is_null() {
        return format!(
            "Server declined rename to '{}' (no edits returned)",
            new_name
        );
    }

    let mut lines: Vec<String> = Vec::new();
    let mut total_edits = 0usize;

    // `changes` field: { uri: [TextEdit, ...] }
    if let Some(changes) = v.get("changes").and_then(|c| c.as_object()) {
        for (uri, edits) in changes {
            let count = edits.as_array().map(|a| a.len()).unwrap_or(0);
            total_edits += count;
            let path = uri.strip_prefix("file://").unwrap_or(uri);
            lines.push(format!("  {} ({} edits)", path, count));
        }
    }

    // `documentChanges` field: array of TextDocumentEdit (preferred shape).
    if let Some(doc_changes) = v.get("documentChanges").and_then(|c| c.as_array()) {
        for doc in doc_changes {
            let uri = doc
                .get("textDocument")
                .and_then(|td| td.get("uri"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            let edits = doc
                .get("edits")
                .and_then(|e| e.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            total_edits += edits;
            let path = uri.strip_prefix("file://").unwrap_or(uri);
            lines.push(format!("  {} ({} edits)", path, edits));
        }
    }

    if lines.is_empty() {
        return format!(
            "Rename to '{}' produced no edits (server may not support rename)",
            new_name
        );
    }
    format!(
        "Rename to '{}' would apply {} edit(s) across {} file(s):\n{}\n\n(Run the `edit` tool manually if you want to apply these — `lsp` does not auto-apply rename edits.)",
        new_name,
        total_edits,
        lines.len(),
        lines.join("\n")
    )
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
        }))
        .unwrap();
        assert_eq!(loc.format(), "/workspace/foo.rs:10:5");
    }

    #[test]
    fn symbol_kind_label_known_values() {
        assert_eq!(symbol_kind_label(12), "function");
        assert_eq!(symbol_kind_label(5), "class");
        assert_eq!(symbol_kind_label(23), "struct");
        assert_eq!(symbol_kind_label(999), "symbol");
    }

    #[test]
    fn parse_symbols_handles_flat_symbol_information() {
        let v = serde_json::json!([
            {
                "name": "main",
                "kind": 12,
                "location": {
                    "uri": "file:///a.rs",
                    "range": {"start": {"line": 4, "character": 3}, "end": {"line": 4, "character": 7}}
                }
            }
        ]);
        let out = parse_symbols(v);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "main");
        assert_eq!(out[0].kind, 12);
        assert_eq!(out[0].line, 4);
        assert_eq!(out[0].uri, "file:///a.rs");
    }

    #[test]
    fn parse_symbols_handles_hierarchical_document_symbol() {
        let v = serde_json::json!([
            {
                "name": "Foo",
                "kind": 23, // struct
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 20, "character": 0}},
                "selectionRange": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 10}},
                "children": [
                    {
                        "name": "bar",
                        "kind": 6, // method
                        "range": {"start": {"line": 5, "character": 4}, "end": {"line": 8, "character": 5}},
                        "selectionRange": {"start": {"line": 5, "character": 7}, "end": {"line": 5, "character": 10}}
                    }
                ]
            }
        ]);
        let out = parse_symbols(v);
        // Flattened: Foo at line 0, bar at line 5.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "Foo");
        assert_eq!(out[0].line, 0);
        assert_eq!(out[1].name, "bar");
        assert_eq!(out[1].line, 5);
    }

    #[test]
    fn symbol_format_combines_path_kind_name() {
        let s = SymbolSummary {
            name: "process_stream".to_string(),
            kind: 12,
            line: 41,
            uri: "file:///repo/src/processor.rs".to_string(),
        };
        assert_eq!(
            s.format(),
            "/repo/src/processor.rs:42 [function] process_stream"
        );
    }

    #[test]
    fn format_workspace_edit_null_means_declined() {
        let out = format_workspace_edit(&Value::Null, "newName");
        assert!(out.contains("declined"));
    }

    #[test]
    fn format_workspace_edit_summarizes_changes_field() {
        let edit = serde_json::json!({
            "changes": {
                "file:///a.rs": [
                    {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}}, "newText": "foo"},
                    {"range": {"start": {"line": 5, "character": 0}, "end": {"line": 5, "character": 3}}, "newText": "foo"}
                ],
                "file:///b.rs": [
                    {"range": {"start": {"line": 10, "character": 0}, "end": {"line": 10, "character": 3}}, "newText": "foo"}
                ]
            }
        });
        let out = format_workspace_edit(&edit, "foo");
        // 3 total edits across 2 files; both filenames appear.
        assert!(out.contains("3 edit"));
        assert!(out.contains("2 file"));
        assert!(out.contains("/a.rs"));
        assert!(out.contains("/b.rs"));
    }

    #[test]
    fn format_workspace_edit_summarizes_document_changes_field() {
        let edit = serde_json::json!({
            "documentChanges": [
                {
                    "textDocument": {"uri": "file:///a.rs", "version": 1},
                    "edits": [
                        {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}}, "newText": "foo"}
                    ]
                }
            ]
        });
        let out = format_workspace_edit(&edit, "foo");
        assert!(out.contains("1 edit"));
        assert!(out.contains("/a.rs"));
    }
}
