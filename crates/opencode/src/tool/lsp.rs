use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;

use super::context::ToolContext;
use super::result::ToolResult;
use super::r#trait::Tool;

const OPERATIONS: &[&str] = &[
    "diagnostics",
    "goToDefinition",
    "findReferences",
    "hover",
    "documentSymbol",
    "workspaceSymbol",
    "goToImplementation",
    "prepareRename",
    "rename",
];

#[derive(Debug, Deserialize)]
pub struct LspParams {
    pub operation: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(default)]
    pub line: Option<usize>,
    #[serde(default)]
    pub character: Option<usize>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(rename = "newName")]
    pub new_name: Option<String>,
}

#[derive(Debug)]
pub struct Diagnostic {
    pub severity: String,
    pub message: String,
    pub line: usize,
    pub character: usize,
    pub source: String,
}

pub struct LspTool;

impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Get errors, warnings, hints from language server BEFORE running build."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": OPERATIONS,
                    "description": "The LSP operation to perform"
                },
                "filePath": {
                    "type": "string",
                    "description": "File or directory path to check diagnostics for"
                },
                "line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Line number (1-based)"
                },
                "character": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Character offset (0-based)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query for workspaceSymbol"
                },
                "severity": {
                    "type": "string",
                    "enum": ["error", "warning", "information", "hint", "all"],
                    "description": "Filter by severity level"
                },
                "newName": {
                    "type": "string",
                    "description": "New symbol name for rename operation"
                }
            },
            "required": ["operation", "filePath"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: LspParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid lsp parameters: {}", e))?;

            if !OPERATIONS.contains(&params.operation.as_str()) {
                return Err(anyhow::anyhow!(
                    "Invalid operation: {}. Valid operations: {}",
                    params.operation,
                    OPERATIONS.join(", ")
                ));
            }

            let file_path = if PathBuf::from(&params.file_path).is_absolute() {
                PathBuf::from(&params.file_path)
            } else {
                ctx.working_dir.join(&params.file_path)
            };

            if !file_path.exists() {
                return Err(anyhow::anyhow!("File not found: {}", file_path.display()));
            }

            let output = match params.operation.as_str() {
                "diagnostics" => {
                    get_diagnostics(&file_path, &ctx.working_dir, params.severity.as_deref())?
                }
                "goToDefinition" | "findReferences" | "hover" => {
                    let line = params.line.unwrap_or(1);
                    let char = params.character.unwrap_or(0);
                    format!(
                        "{} for {} at {}:{} - use lsp_goto_definition or lsp_find_references tool",
                        params.operation,
                        file_path.display(),
                        line,
                        char
                    )
                }
                "documentSymbol" => {
                    format!("Document symbols for {} - use lsp_symbols tool", file_path.display())
                }
                "workspaceSymbol" => {
                    let query = params.query.unwrap_or_default();
                    format!("Workspace symbol search for '{}' - use lsp_symbols(scope='workspace') tool", query)
                }
                "rename" => {
                    let new_name = params.new_name.unwrap_or_default();
                    let line = params.line.unwrap_or(1);
                    let char = params.character.unwrap_or(0);
                    format!(
                        "Rename symbol to '{}' at {}:{}:{} - use lsp_rename tool",
                        new_name,
                        file_path.display(),
                        line,
                        char
                    )
                }
                _ => format!("{} requested", params.operation),
            };

            Ok(ToolResult::with_metadata(
                output,
                json!({
                    "operation": params.operation,
                    "file_path": file_path.display().to_string(),
                    "line": params.line,
                    "character": params.character,
                }),
            ))
        })
    }
}

fn get_diagnostics(file_path: &PathBuf, working_dir: &PathBuf, severity: Option<&str>) -> Result<String> {
    let ext = file_path.extension().and_then(|e| e.to_str());
    
    match ext {
        Some("rs") => get_rust_diagnostics(file_path, working_dir, severity),
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") => get_js_diagnostics(file_path, working_dir, severity),
        Some("py") => get_python_diagnostics(file_path, working_dir, severity),
        _ => Ok(format!("No LSP diagnostics available for {} - unsupported file type", file_path.display())),
    }
}

fn get_rust_diagnostics(file_path: &PathBuf, working_dir: &PathBuf, severity: Option<&str>) -> Result<String> {
    // Use `cargo check --message-format=json` instead of `--short`. JSON
    // gives us structured per-diagnostic info (level, file, line/column,
    // code, primary message) so we can filter by both file and severity
    // without string-matching the prose output. The old `--short`
    // approach was the bug that made e.g. a *path* containing "error"
    // count as an error.
    let output = Command::new("cargo")
        .args([
            "check",
            "--message-format=json",
            "--quiet",
        ])
        .current_dir(working_dir)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    // The target file matters relative to the working directory cargo was
    // invoked in; canonicalize both to compare on a stable form.
    let target = match file_path.canonicalize() {
        Ok(p) => Some(p),
        Err(_) => None,
    };
    let working_canonical = working_dir.canonicalize().ok();

    let diagnostics = parse_cargo_diagnostics(
        &stdout,
        working_canonical.as_deref(),
        target.as_deref(),
        severity,
    );

    if diagnostics.is_empty() {
        Ok(format!(
            "No Rust diagnostics for {}",
            file_path.display()
        ))
    } else {
        Ok(diagnostics.join("\n"))
    }
}

/// Single diagnostic entry from cargo's JSON output, narrowed to the
/// fields we actually use. cargo emits many other event kinds
/// ("compiler-artifact", "build-finished", etc.) which we drop in the
/// `reason != "compiler-message"` filter.
#[derive(Debug, Deserialize)]
struct CargoEvent {
    reason: String,
    #[serde(default)]
    message: Option<CargoMessage>,
}

#[derive(Debug, Deserialize)]
struct CargoMessage {
    level: String,
    message: String,
    #[serde(default)]
    code: Option<CargoCode>,
    #[serde(default)]
    spans: Vec<CargoSpan>,
}

#[derive(Debug, Deserialize)]
struct CargoCode {
    code: String,
}

#[derive(Debug, Deserialize)]
struct CargoSpan {
    file_name: String,
    line_start: u32,
    column_start: u32,
    #[serde(default)]
    is_primary: bool,
}

/// Parse cargo's NDJSON output into a human-readable list of
/// `file:line:col [level] CODE: message` strings, filtered to entries
/// whose primary span hits `target_file` and (if specified) match
/// `severity`.
///
/// Pure function — exposed for unit testing. `cargo_working_dir` is the
/// directory cargo was invoked in (used to absolutize the relative
/// `file_name` paths cargo emits).
pub fn parse_cargo_diagnostics(
    stdout: &str,
    cargo_working_dir: Option<&std::path::Path>,
    target_file: Option<&std::path::Path>,
    severity: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let event: CargoEvent = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if event.reason != "compiler-message" {
            continue;
        }
        let Some(msg) = event.message else { continue };

        // Severity filter is "error" / "warning"; cargo levels include
        // "error", "warning", "note", "help", and "failure-note".
        if let Some(sev) = severity {
            let matches = match sev {
                "error" => msg.level == "error",
                "warning" => msg.level == "warning",
                _ => true,
            };
            if !matches {
                continue;
            }
        }

        // Find the primary span (cargo marks at most one). Fall back to
        // the first span if none is flagged primary.
        let primary = msg.spans.iter().find(|s| s.is_primary)
            .or_else(|| msg.spans.first());
        let Some(span) = primary else { continue };

        // Resolve the span's file relative to where cargo ran.
        let span_path = match cargo_working_dir {
            Some(base) => base.join(&span.file_name),
            None => std::path::PathBuf::from(&span.file_name),
        };
        if let Some(target) = target_file {
            let canonical = span_path.canonicalize().unwrap_or(span_path.clone());
            if canonical != target {
                continue;
            }
        }

        let code = msg.code.as_ref().map(|c| format!(" [{}]", c.code)).unwrap_or_default();
        out.push(format!(
            "{}:{}:{} [{}]{}: {}",
            span_path.display(),
            span.line_start,
            span.column_start,
            msg.level,
            code,
            msg.message,
        ));
    }
    out
}

fn get_js_diagnostics(file_path: &PathBuf, working_dir: &PathBuf, severity: Option<&str>) -> Result<String> {
    let tsc_output = Command::new("npx")
        .args(["tsc", "--noEmit", "--pretty", "false"])
        .current_dir(working_dir)
        .output();
    
    match tsc_output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}{}", stdout, stderr);
            
            let file_name = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let diagnostics: Vec<String> = combined.lines()
                .filter(|line| line.contains(file_name))
                .filter(|line| {
                    match severity {
                        Some("error") => line.contains("error"),
                        Some("warning") => line.contains("warning"),
                        _ => true,
                    }
                })
                .map(|line| line.to_string())
                .collect();
            
            if diagnostics.is_empty() {
                Ok("No TypeScript diagnostics found".to_string())
            } else {
                Ok(diagnostics.join("\n"))
            }
        }
        Err(_) => Ok("TypeScript compiler not available - install tsc for diagnostics".to_string()),
    }
}

fn get_python_diagnostics(file_path: &PathBuf, working_dir: &PathBuf, severity: Option<&str>) -> Result<String> {
    let pylint_output = Command::new("pylint")
        .args([file_path.to_str().unwrap_or_default(), "--output-format=text"])
        .current_dir(working_dir)
        .output();
    
    match pylint_output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            
            let diagnostics: Vec<String> = stdout.lines()
                .filter(|line| {
                    match severity {
                        Some("error") => line.contains("E") || line.contains("F"),
                        Some("warning") => line.contains("W"),
                        _ => true,
                    }
                })
                .map(|line| line.to_string())
                .collect();
            
            if diagnostics.is_empty() {
                Ok("No Python diagnostics found".to_string())
            } else {
                Ok(diagnostics.join("\n"))
            }
        }
        Err(_) => {
            let pyflakes_output = Command::new("pyflakes")
                .args([file_path.to_str().unwrap_or_default()])
                .current_dir(working_dir)
                .output();
            
            match pyflakes_output {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.is_empty() {
                        Ok("No Python diagnostics found".to_string())
                    } else {
                        Ok(stdout.to_string())
                    }
                }
                Err(_) => Ok("Python linter not available - install pylint or pyflakes".to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-ish cargo JSON output: one warning in foo.rs, one error in
    /// bar.rs, plus a non-diagnostic event we should drop.
    const SAMPLE_NDJSON: &str = concat!(
        r#"{"reason":"compiler-artifact","package_id":"x"}"#,
        "\n",
        r#"{"reason":"compiler-message","package_id":"x","message":{"level":"warning","message":"unused variable","code":{"code":"unused_variables"},"spans":[{"file_name":"src/foo.rs","line_start":10,"column_start":5,"is_primary":true,"byte_start":0,"byte_end":1,"line_end":10,"column_end":6,"text":[]}]}}"#,
        "\n",
        r#"{"reason":"compiler-message","package_id":"x","message":{"level":"error","message":"mismatched types","code":{"code":"E0308"},"spans":[{"file_name":"src/bar.rs","line_start":20,"column_start":12,"is_primary":true,"byte_start":0,"byte_end":1,"line_end":20,"column_end":13,"text":[]}]}}"#,
    );

    #[test]
    fn drops_non_compiler_messages() {
        let diags = parse_cargo_diagnostics(SAMPLE_NDJSON, None, None, None);
        assert_eq!(diags.len(), 2, "{:?}", diags);
    }

    #[test]
    fn severity_filter_keeps_errors_only() {
        let diags = parse_cargo_diagnostics(SAMPLE_NDJSON, None, None, Some("error"));
        assert_eq!(diags.len(), 1);
        assert!(diags[0].contains("[error]"));
        assert!(diags[0].contains("E0308"));
    }

    #[test]
    fn severity_filter_keeps_warnings_only() {
        let diags = parse_cargo_diagnostics(SAMPLE_NDJSON, None, None, Some("warning"));
        assert_eq!(diags.len(), 1);
        assert!(diags[0].contains("[warning]"));
    }

    #[test]
    fn file_filter_narrows_to_target() {
        // We canonicalize the target, so pick a path that doesn't have
        // to exist on disk: use the relative path directly (None for
        // working dir → relative path stays relative, no canonicalize
        // happens, comparison is on the raw PathBuf).
        let target = std::path::PathBuf::from("src/bar.rs");
        // To keep the test hermetic we pass None for working dir and a
        // non-canonicalized target. The canonicalize().unwrap_or(...)
        // fallback keeps it relative.
        let diags = parse_cargo_diagnostics(SAMPLE_NDJSON, None, Some(&target), None);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].contains("bar.rs"));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let mixed = format!(
            "not json at all\n   \n{}\n{{\"reason\":\"compiler-message\"}}\n",
            SAMPLE_NDJSON.lines().nth(1).unwrap()
        );
        // Should parse the one valid diagnostic and skip the broken
        // ones (incomplete message, plain text).
        let diags = parse_cargo_diagnostics(&mixed, None, None, None);
        assert_eq!(diags.len(), 1);
    }
}