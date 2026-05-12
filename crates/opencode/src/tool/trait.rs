use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde::Deserialize;

use super::context::ToolContext;
use super::result::ToolResult;

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, anyhow::Error>> + Send + '_>>;
}

#[derive(Debug, Deserialize)]
pub struct BashParams {
    pub command: String,
    pub workdir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct ReadParams {
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(default = "default_offset")]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_offset() -> usize {
    1
}

fn default_limit() -> usize {
    2000
}

#[derive(Debug, Deserialize)]
pub struct WriteParams {
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct EditParams {
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "oldString")]
    pub old_string: String,
    #[serde(rename = "newString")]
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Deserialize)]
pub struct GlobParams {
    pub pattern: String,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct GrepParams {
    pub pattern: String,
    pub path: Option<PathBuf>,
    pub include: Option<String>,
}
