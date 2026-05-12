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
    let output = Command::new("cargo")
        .args(["check", "--message-format=short", "2>&1"])
        .current_dir(working_dir)
        .output()?;
    
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, stderr);
    
    let diagnostics: Vec<String> = combined.lines()
        .filter(|line| line.contains("error") || line.contains("warning"))
        .filter(|line| {
            match severity {
                Some("error") => line.contains("error") && !line.contains("warning"),
                Some("warning") => line.contains("warning"),
                _ => true,
            }
        })
        .map(|line| line.to_string())
        .collect();
    
    if diagnostics.is_empty() {
        Ok("No Rust diagnostics found - code appears clean".to_string())
    } else {
        Ok(diagnostics.join("\n"))
    }
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