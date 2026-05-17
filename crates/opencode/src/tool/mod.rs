pub mod ast_grep;
pub mod background_tools;
pub mod bash;
pub mod codesearch;
pub mod context;
pub mod edit;
pub mod external_directory;
pub mod glob;
pub mod grep;
pub mod interactive_bash;
pub mod lsp;
pub mod patch;
pub mod plan;
pub mod question;
pub mod read;
pub mod repo_search;
pub mod repo_tools;
pub mod result;
pub mod session_tools;
pub mod skill;
pub mod task;
pub mod todo;
pub mod r#trait;
pub mod truncate;
pub mod webfetch;
pub mod websearch;
pub mod write;

pub use ast_grep::{AstGrepReplaceTool, AstGrepSearchTool};
pub use background_tools::{BackgroundCancelTool, BackgroundOutputTool};
pub use bash::BashTool;
pub use codesearch::CodeSearchTool;
pub use context::ToolContext;
pub use edit::EditTool;
pub use external_directory::{assert_external_directory, ExternalKind};
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use interactive_bash::InteractiveBashTool;
pub use lsp::LspTool;
pub use patch::ApplyPatchTool;
pub use plan::PlanTool;
pub use question::QuestionTool;
pub use r#trait::Tool;
pub use read::ReadTool;
pub use repo_search::RepoSearchTool;
pub use repo_tools::{RepoCloneTool, RepoOverviewTool};
pub use result::ToolResult;
pub use session_tools::{SessionInfoTool, SessionListTool, SessionReadTool, SessionSearchTool};
pub use skill::SkillTool;
pub use task::TaskTool;
pub use todo::{TodoItem, TodoWriteTool};
pub use truncate::TruncateTool;
pub use webfetch::WebFetchTool;
pub use websearch::WebSearchTool;
pub use write::WriteTool;

use std::sync::Arc;

/// Build the default tool registry an agent sees in build mode.
///
/// This is the union of:
///   * File I/O: read, write, edit, glob, grep, apply_patch
///   * Process: bash, background_output (+ cancel), interactive_bash
///   * Code intelligence: lsp, codesearch, ast_grep_search/replace,
///     repo_search, repo_overview
///   * Web: webfetch, websearch
///   * Workflow: plan, question, skill, task, todowrite, truncate
///   * Session introspection: session_list, session_info, session_read,
///     session_search
///   * Repository: repo_clone
///
/// Tools still run through the permission layer after being exposed. The
/// default build agent denies `repo_clone`; specialist agents can opt in.
///
/// Provider-internal tools:
///   * `LspTool` is exposed but its
///     workspace-symbol op needs a non-empty query so the agent rarely
///     finds it useful without prompting; we expose it anyway).
///
/// Returns Arc'd trait objects so the registry is cheap to clone into
/// processor + ACP agent.
pub fn default_registry() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(BashTool),
        Arc::new(ReadTool),
        Arc::new(WriteTool),
        Arc::new(EditTool),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(ApplyPatchTool),
        Arc::new(LspTool),
        Arc::new(CodeSearchTool),
        Arc::new(AstGrepSearchTool),
        Arc::new(AstGrepReplaceTool),
        Arc::new(RepoSearchTool),
        Arc::new(RepoCloneTool),
        Arc::new(RepoOverviewTool),
        Arc::new(WebFetchTool),
        Arc::new(WebSearchTool),
        Arc::new(PlanTool),
        Arc::new(QuestionTool),
        Arc::new(SkillTool),
        Arc::new(TaskTool),
        Arc::new(TodoWriteTool),
        Arc::new(TruncateTool),
        Arc::new(SessionListTool),
        Arc::new(SessionInfoTool),
        Arc::new(SessionReadTool),
        Arc::new(SessionSearchTool),
        Arc::new(BackgroundOutputTool),
        Arc::new(BackgroundCancelTool),
        Arc::new(InteractiveBashTool),
    ]
}

pub fn registry_with(mut extra: Vec<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
    let mut tools = default_registry();
    tools.append(&mut extra);
    tools
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_registry_exposes_repo_clone() {
        let tools = super::default_registry();

        assert!(
            tools.iter().any(|tool| tool.name() == "repo_clone"),
            "repo_clone should be available for agents whose permissions allow it"
        );
    }
}
