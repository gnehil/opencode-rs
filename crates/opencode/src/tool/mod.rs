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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryOptions {
    pub experimental_scout: bool,
    pub experimental_lsp_tool: bool,
    pub experimental_plan_mode: bool,
    pub client: String,
}

impl Default for RegistryOptions {
    fn default() -> Self {
        Self {
            experimental_scout: false,
            experimental_lsp_tool: false,
            experimental_plan_mode: false,
            client: std::env::var("OPENCODE_CLIENT").unwrap_or_else(|_| "cli".to_string()),
        }
    }
}

impl RegistryOptions {
    pub fn from_config_and_env(config: Option<&crate::config::Config>) -> Self {
        let experimental = env_enabled("OPENCODE_EXPERIMENTAL");
        let experimental_config = config.and_then(|config| config.experimental.as_ref());
        Self {
            experimental_scout: experimental || env_enabled("OPENCODE_EXPERIMENTAL_SCOUT"),
            experimental_lsp_tool: experimental || env_enabled("OPENCODE_EXPERIMENTAL_LSP_TOOL"),
            experimental_plan_mode: experimental || env_enabled("OPENCODE_EXPERIMENTAL_PLAN_MODE"),
            client: std::env::var("OPENCODE_CLIENT").unwrap_or_else(|_| "cli".to_string()),
        }
        .with_primary_tools(experimental_config.and_then(|config| config.primary_tools.as_ref()))
    }

    fn with_primary_tools(mut self, primary_tools: Option<&Vec<String>>) -> Self {
        if let Some(primary_tools) = primary_tools {
            self.experimental_scout = self.experimental_scout
                || primary_tools
                    .iter()
                    .any(|tool| matches!(tool.as_str(), "repo_clone" | "repo_overview"));
            self.experimental_lsp_tool =
                self.experimental_lsp_tool || primary_tools.iter().any(|tool| tool == "lsp");
            self.experimental_plan_mode =
                self.experimental_plan_mode || primary_tools.iter().any(|tool| tool == "plan");
        }
        self
    }
}

fn env_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

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

pub fn registry_for_options(options: RegistryOptions) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(BashTool),
        Arc::new(ReadTool),
        Arc::new(WriteTool),
        Arc::new(EditTool),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(ApplyPatchTool),
        Arc::new(CodeSearchTool),
        Arc::new(AstGrepSearchTool),
        Arc::new(AstGrepReplaceTool),
        Arc::new(RepoSearchTool),
        Arc::new(WebFetchTool),
        Arc::new(WebSearchTool),
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
    ];

    if options.experimental_scout {
        tools.push(Arc::new(RepoCloneTool));
        tools.push(Arc::new(RepoOverviewTool));
    }
    if options.experimental_lsp_tool {
        tools.push(Arc::new(LspTool));
    }
    if options.experimental_plan_mode && options.client == "cli" {
        tools.push(Arc::new(PlanTool));
    }
    tools
}

pub fn registry_with(mut extra: Vec<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
    let mut tools = default_registry();
    tools.append(&mut extra);
    tools
}

pub fn registry_with_options(
    options: RegistryOptions,
    mut extra: Vec<Arc<dyn Tool>>,
) -> Vec<Arc<dyn Tool>> {
    let mut tools = registry_for_options(options);
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

    #[test]
    fn registry_for_options_gates_experimental_tools_like_upstream() {
        let default = super::registry_for_options(super::RegistryOptions::default());
        assert!(default.iter().any(|tool| tool.name() == "bash"));
        assert!(default.iter().any(|tool| tool.name() == "question"));
        assert!(!default.iter().any(|tool| tool.name() == "repo_clone"));
        assert!(!default.iter().any(|tool| tool.name() == "repo_overview"));
        assert!(!default.iter().any(|tool| tool.name() == "lsp"));
        assert!(!default.iter().any(|tool| tool.name() == "plan"));

        let enabled = super::registry_for_options(super::RegistryOptions {
            experimental_scout: true,
            experimental_lsp_tool: true,
            experimental_plan_mode: true,
            client: "cli".to_string(),
        });
        assert!(enabled.iter().any(|tool| tool.name() == "repo_clone"));
        assert!(enabled.iter().any(|tool| tool.name() == "repo_overview"));
        assert!(enabled.iter().any(|tool| tool.name() == "lsp"));
        assert!(enabled.iter().any(|tool| tool.name() == "plan"));

        let app = super::registry_for_options(super::RegistryOptions {
            client: "app".to_string(),
            ..super::RegistryOptions::default()
        });
        assert!(!app.iter().any(|tool| tool.name() == "plan"));
    }
}
