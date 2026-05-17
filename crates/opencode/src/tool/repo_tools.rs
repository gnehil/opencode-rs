use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use super::context::ToolContext;
use super::r#trait::Tool;
use super::result::ToolResult;

#[derive(Debug, Deserialize)]
pub struct RepoCloneParams {
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub branch: Option<String>,
}

impl RepoCloneParams {
    fn repository(&self) -> &str {
        self.repository
            .as_deref()
            .or(self.url.as_deref())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepositoryReference {
    host: String,
    segments: Vec<String>,
    remote: String,
    label: String,
}

fn normalize_repository_input(input: &str) -> String {
    input
        .trim()
        .trim_start_matches("git+")
        .split('#')
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string()
}

fn trim_git_suffix(input: &str) -> &str {
    input.strip_suffix(".git").unwrap_or(input)
}

fn repository_parts(input: &str) -> Vec<String> {
    input
        .split('/')
        .map(|item| trim_git_suffix(item.trim()).to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn safe_host(input: &str) -> bool {
    !input.is_empty()
        && !input.starts_with('-')
        && !input
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '/' || ch == '\\')
}

fn safe_segment(input: &str) -> bool {
    input != "."
        && input != ".."
        && !input.contains(':')
        && !input
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '/' || ch == '\\')
}

fn host_like(input: &str) -> bool {
    input.contains('.') || input.contains(':') || input == "localhost"
}

fn github_remote(pathname: &str) -> String {
    if let Ok(base) = std::env::var("OPENCODE_REPO_CLONE_GITHUB_BASE_URL") {
        let base = base.trim_end_matches('/');
        return format!("{base}/{pathname}.git");
    }
    format!("https://github.com/{pathname}.git")
}

fn build_reference(
    host: impl Into<String>,
    segments: Vec<String>,
    remote: Option<String>,
) -> Option<RepositoryReference> {
    let segments: Vec<String> = segments
        .into_iter()
        .map(|segment| trim_git_suffix(segment.trim()).to_string())
        .filter(|segment| !segment.is_empty())
        .collect();
    let host = host.into();
    if !safe_host(&host) || segments.is_empty() || segments.iter().any(|s| !safe_segment(s)) {
        return None;
    }

    let host = host.to_lowercase();
    let path = segments.join("/");
    let remote = remote.unwrap_or_else(|| {
        if host == "github.com" {
            github_remote(&path)
        } else {
            format!("https://{host}/{path}.git")
        }
    });
    let label = if host == "github.com" && segments.len() == 2 {
        path
    } else {
        format!("{host}/{path}")
    };

    Some(RepositoryReference {
        host,
        segments,
        remote,
        label,
    })
}

fn parse_remote_repository_reference(input: &str) -> Result<RepositoryReference> {
    let cleaned = normalize_repository_input(input);
    if cleaned.is_empty() {
        anyhow::bail!(
            "Repository must be a git URL, host/path reference, or GitHub owner/repo shorthand"
        );
    }

    if let Some(rest) = cleaned.strip_prefix("github:") {
        let direct = repository_parts(rest);
        if direct.len() == 2 {
            if let Some(reference) = build_reference("github.com", direct, None) {
                return Ok(reference);
            }
        }
    }

    if !cleaned.contains("://") {
        if let Some((host_part, path_part)) = cleaned.split_once(':') {
            let host = host_part.rsplit('@').next().unwrap_or(host_part);
            if !host.contains('/') && !host.is_empty() {
                if let Some(reference) =
                    build_reference(host, repository_parts(path_part), Some(cleaned.clone()))
                {
                    return Ok(reference);
                }
            }
        }

        let direct = repository_parts(&cleaned);
        if direct.len() >= 2 && host_like(&direct[0]) {
            if let Some(reference) = build_reference(direct[0].clone(), direct[1..].to_vec(), None)
            {
                return Ok(reference);
            }
        }
        if direct.len() == 2 {
            if let Some(reference) = build_reference("github.com", direct, None) {
                return Ok(reference);
            }
        }
    }

    if let Ok(url) = reqwest::Url::parse(&cleaned) {
        if url.scheme() == "file" {
            anyhow::bail!("Local file repositories are not supported");
        }
        let host = url.host_str().unwrap_or_default();
        let path = repository_parts(url.path());
        let remote = if host == "github.com" {
            Some(github_remote(&path.join("/")))
        } else {
            Some(cleaned)
        };
        if let Some(reference) = build_reference(host, path, remote) {
            return Ok(reference);
        }
    }

    anyhow::bail!(
        "Repository must be a git URL, host/path reference, or GitHub owner/repo shorthand"
    )
}

fn validate_repository_branch(branch: &str) -> Result<()> {
    let valid = !branch.starts_with('-')
        && !branch.contains("..")
        && !branch.is_empty()
        && branch
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '.' | '-'));
    if !valid {
        anyhow::bail!(
            "Branch must contain only alphanumeric characters, /, _, ., and -, and cannot start with - or contain .."
        );
    }
    Ok(())
}

fn repository_cache_path(reference: &RepositoryReference) -> PathBuf {
    let mut path = crate::global::repos().clone();
    for part in reference.host.split(':') {
        path.push(part);
    }
    for segment in &reference.segments {
        path.push(segment);
    }
    path
}

fn same_repository_reference(left: &RepositoryReference, right: &RepositoryReference) -> bool {
    left.host == right.host && left.segments == right.segments
}

fn git_output(args: &[&str], cwd: &std::path::Path) -> Option<String> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

pub struct RepoCloneTool;

impl Tool for RepoCloneTool {
    fn name(&self) -> &str {
        "repo_clone"
    }

    fn description(&self) -> &str {
        "Clone a repository from URL into the managed cache for dependency inspection."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "repository": {
                    "type": "string",
                    "description": "Repository to clone, as a git URL, host/path reference, or GitHub owner/repo shorthand"
                },
                "refresh": {
                    "type": "boolean",
                    "description": "When true, fetches the latest remote state into the managed cache"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch or ref to clone and inspect"
                },
                "url": {
                    "type": "string",
                    "description": "Legacy repository URL; prefer repository"
                }
            },
            "required": ["repository"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: RepoCloneParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid repo_clone parameters: {}", e))?;
            let reference = parse_remote_repository_reference(params.repository())?;
            if let Some(branch) = &params.branch {
                validate_repository_branch(branch)?;
            }

            let repository = reference.label.clone();
            let remote = reference.remote.clone();
            let target_path = repository_cache_path(&reference);
            let target_display = target_path.display().to_string();

            ctx.check_permission("repo_clone", &repository).await?;

            let has_git_dir = target_path.join(".git").is_dir();
            let origin_reference = if has_git_dir {
                git_output(&["config", "--get", "remote.origin.url"], &target_path)
                    .and_then(|origin| parse_remote_repository_reference(&origin).ok())
            } else {
                None
            };
            let reuse = has_git_dir
                && origin_reference
                    .as_ref()
                    .is_some_and(|origin| same_repository_reference(origin, &reference));
            if target_path.exists() && !reuse {
                if target_path.is_dir() {
                    std::fs::remove_dir_all(&target_path)?;
                } else {
                    std::fs::remove_file(&target_path)?;
                }
            }

            let current_branch = if reuse {
                git_output(
                    &["symbolic-ref", "--quiet", "--short", "HEAD"],
                    &target_path,
                )
            } else {
                None
            };
            let branch_matches = params.branch.as_ref().map(|branch| {
                current_branch
                    .as_ref()
                    .is_some_and(|current| current == branch)
            });
            let status = if reuse && !params.refresh && branch_matches != Some(false) {
                "cached"
            } else if reuse {
                "refreshed"
            } else {
                "cloned"
            };

            if status == "cloned" {
                if let Some(parent) = target_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if target_path.exists() {
                    std::fs::remove_dir_all(&target_path)?;
                }
                let mut args = vec![
                    "clone".to_string(),
                    "--depth".to_string(),
                    "100".to_string(),
                ];
                if let Some(branch) = &params.branch {
                    args.extend(["--branch".to_string(), branch.clone()]);
                }
                args.extend(["--".to_string(), remote.clone(), target_display.clone()]);
                let output = std::process::Command::new("git").args(&args).output();
                match output {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        return Err(anyhow::anyhow!("Git clone failed: {}", stderr.trim()));
                    }
                    Err(e) => return Err(anyhow::anyhow!("Failed to execute git: {}", e)),
                }
            }

            if status == "refreshed" {
                let output = std::process::Command::new("git")
                    .args(["fetch", "--all", "--prune"])
                    .current_dir(&target_path)
                    .output();
                match output {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        return Err(anyhow::anyhow!("Git fetch failed: {}", stderr.trim()));
                    }
                    Err(e) => return Err(anyhow::anyhow!("Failed to execute git: {}", e)),
                }

                if let Some(branch) = &params.branch {
                    let target = format!("origin/{branch}");
                    let output = std::process::Command::new("git")
                        .args(["checkout", "-B", branch, &target])
                        .current_dir(&target_path)
                        .output();
                    match output {
                        Ok(o) if o.status.success() => {}
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr);
                            return Err(anyhow::anyhow!("Git checkout failed: {}", stderr.trim()));
                        }
                        Err(e) => return Err(anyhow::anyhow!("Failed to execute git: {}", e)),
                    }
                }

                let target = params
                    .branch
                    .as_ref()
                    .map(|branch| format!("origin/{branch}"))
                    .or_else(|| {
                        git_output(&["symbolic-ref", "refs/remotes/origin/HEAD"], &target_path).map(
                            |head| {
                                head.strip_prefix("refs/remotes/")
                                    .unwrap_or(&head)
                                    .to_string()
                            },
                        )
                    })
                    .or_else(|| {
                        current_branch
                            .as_ref()
                            .map(|branch| format!("origin/{branch}"))
                    })
                    .unwrap_or_else(|| "HEAD".to_string());
                let output = std::process::Command::new("git")
                    .args(["reset", "--hard", &target])
                    .current_dir(&target_path)
                    .output();
                match output {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        return Err(anyhow::anyhow!("Git reset failed: {}", stderr.trim()));
                    }
                    Err(e) => return Err(anyhow::anyhow!("Failed to execute git: {}", e)),
                }
            }

            let head = git_output(&["rev-parse", "HEAD"], &target_path);
            let branch = git_output(
                &["symbolic-ref", "--quiet", "--short", "HEAD"],
                &target_path,
            );

            Ok(ToolResult::with_metadata(
                [
                    format!("Repository ready: {repository}"),
                    format!("Status: {status}"),
                    format!("Local path: {target_display}"),
                ]
                .into_iter()
                .chain(branch.as_ref().map(|b| format!("Branch: {b}")))
                .chain(head.as_ref().map(|h| format!("HEAD: {h}")))
                .collect::<Vec<_>>()
                .join("\n"),
                json!({
                    "repository": repository,
                    "host": reference.host,
                    "remote": remote,
                    "localPath": target_display,
                    "status": status,
                    "head": head,
                    "branch": branch,
                }),
            ))
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct RepoOverviewParams {
    pub path: String,
    #[serde(default)]
    pub max_files: Option<usize>,
}

pub struct RepoOverviewTool;

impl Tool for RepoOverviewTool {
    fn name(&self) -> &str {
        "repo_overview"
    }

    fn description(&self) -> &str {
        "Generate an overview analysis of a repository structure and content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Repository path to analyze" },
                "max_files": { "type": "integer", "description": "Maximum files to include in overview" }
            },
            "required": ["path"]
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: RepoOverviewParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid repo_overview parameters: {}", e))?;

            let path = std::path::PathBuf::from(&params.path);
            if !path.exists() {
                return Err(anyhow::anyhow!("Path not found: {}", params.path));
            }

            let mut overview = format!("# Repository Overview: {}\n\n", params.path);

            if let Ok(entries) = std::fs::read_dir(&path) {
                let mut dirs = Vec::new();
                let mut files = Vec::new();

                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    if entry.path().is_dir() {
                        dirs.push(name);
                    } else {
                        files.push(name);
                    }
                }

                overview.push_str(&format!(
                    "## Directories ({})\n{}\n\n",
                    dirs.len(),
                    dirs.join(", ")
                ));
                overview.push_str(&format!(
                    "## Root Files ({})\n{}\n\n",
                    files.len(),
                    files.join(", ")
                ));
            }

            if let Ok(output) = std::process::Command::new("git")
                .args(["log", "--oneline", "-10"])
                .current_dir(&path)
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                overview.push_str(&format!("## Recent Commits\n{}\n", stdout));
            }

            Ok(ToolResult::new(overview))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SessionID;
    use crate::permission::PermissionRule;
    use crate::tool::Tool;

    fn ctx(rules: crate::permission::Ruleset) -> ToolContext {
        ToolContext {
            session_id: SessionID::new(),
            working_dir: PathBuf::from("/tmp/workspace"),
            permission_rules: rules,
            event_bus: None,
            permission_broker: None,
            provider: None,
            store: None,
            config: None,
            agent_name: None,
            model_id: None,
            plugin_manager: None,
            question_broker: None,
            skip_permissions: false,
        }
    }

    #[test]
    fn repo_clone_schema_matches_upstream_parameters() {
        let schema = RepoCloneTool.parameters_schema();
        let properties = schema["properties"].as_object().unwrap();

        assert!(properties.contains_key("repository"));
        assert!(properties.contains_key("refresh"));
        assert!(properties.contains_key("branch"));
        assert!(properties.contains_key("url"));
        assert!(!properties.contains_key("path"));
        assert!(!properties.contains_key("depth"));
        assert_eq!(schema["required"], json!(["repository"]));
    }

    #[test]
    fn repo_clone_accepts_repository_and_legacy_url() {
        let upstream: RepoCloneParams = serde_json::from_value(json!({
            "repository": "owner/project",
            "refresh": true,
            "branch": "main"
        }))
        .unwrap();
        assert_eq!(upstream.repository.as_deref(), Some("owner/project"));
        assert_eq!(upstream.repository(), "owner/project");
        assert!(upstream.refresh);
        assert_eq!(upstream.branch.as_deref(), Some("main"));

        let legacy: RepoCloneParams = serde_json::from_value(json!({
            "url": "https://github.com/owner/project.git"
        }))
        .unwrap();
        assert_eq!(legacy.repository(), "https://github.com/owner/project.git");
    }

    #[test]
    fn repo_clone_rejects_arbitrary_path_parameter_surface() {
        let schema = RepoCloneTool.parameters_schema();

        assert!(schema["properties"]["path"].is_null());
    }

    #[test]
    fn repo_clone_cache_path_is_managed_and_derived_from_repository() {
        let reference =
            parse_remote_repository_reference("https://github.com/owner/project.git").unwrap();
        let path = repository_cache_path(&reference);

        assert!(path.starts_with(crate::global::repos()));
        assert!(path.ends_with(PathBuf::from("github.com").join("owner").join("project")));
    }

    #[test]
    fn repo_clone_compares_cache_origin_by_normalized_repository() {
        let shorthand = parse_remote_repository_reference("owner/project").unwrap();
        let github_url =
            parse_remote_repository_reference("https://github.com/owner/project.git").unwrap();
        let other = parse_remote_repository_reference("owner/other").unwrap();

        assert!(same_repository_reference(&shorthand, &github_url));
        assert!(!same_repository_reference(&shorthand, &other));
    }

    #[test]
    fn repo_clone_rejects_local_repositories_and_unsafe_branches() {
        assert!(parse_remote_repository_reference("file:///tmp/repo").is_err());
        assert!(parse_remote_repository_reference("owner/../repo").is_err());
        assert!(validate_repository_branch("../main").is_err());
        assert!(validate_repository_branch("-main").is_err());
        assert!(validate_repository_branch("feature/main").is_ok());
    }

    #[tokio::test]
    async fn repo_clone_uses_permission_gate_before_running_git() {
        let err = RepoCloneTool
            .execute(
                json!({
                    "repository": "owner/project",
                    "path": "/tmp/should-not-be-used"
                }),
                ctx(vec![PermissionRule::deny_tool("repo_clone")]),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("denied by permission rule"), "got: {err}");
    }
}
