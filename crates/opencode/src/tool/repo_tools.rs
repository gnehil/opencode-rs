use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default, alias = "maxFiles")]
    pub max_files: Option<usize>,
}

struct RepoOverviewTarget {
    path: PathBuf,
    repository: Option<String>,
}

struct RepoOverviewEntry {
    name: String,
    path: PathBuf,
    directory: bool,
}

const REPO_OVERVIEW_IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "__pycache__",
    ".venv",
    "dist",
    "build",
    ".next",
    "target",
    "vendor",
];
const REPO_OVERVIEW_STRUCTURE_LIMIT: usize = 200;
const REPO_OVERVIEW_MAX_STRUCTURE_LIMIT: usize = 1000;
const REPO_OVERVIEW_DEPENDENCY_FILES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "bun.lock",
    "bun.lockb",
    "pnpm-lock.yaml",
    "yarn.lock",
    "requirements.txt",
    "pyproject.toml",
    "go.mod",
    "Cargo.toml",
    "Gemfile",
    "build.gradle",
    "build.gradle.kts",
    "pom.xml",
    "composer.json",
];
const REPO_OVERVIEW_COMMON_ENTRYPOINTS: &[&str] = &[
    "index.ts",
    "index.tsx",
    "index.js",
    "index.mjs",
    "main.ts",
    "main.js",
    "src/index.ts",
    "src/index.tsx",
    "src/index.js",
    "src/main.ts",
    "src/main.js",
];

fn resolve_repo_overview_target(
    params: &RepoOverviewParams,
    working_dir: &Path,
) -> Result<RepoOverviewTarget> {
    if let Some(requested_path) = params.path.as_deref().filter(|p| !p.trim().is_empty()) {
        let path = Path::new(requested_path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            working_dir.join(path)
        };
        return Ok(RepoOverviewTarget {
            path,
            repository: params.repository.clone(),
        });
    }

    let repository = params
        .repository
        .as_deref()
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Either repository or path is required"))?;
    let reference = parse_remote_repository_reference(repository)?;
    Ok(RepoOverviewTarget {
        path: repository_cache_path(&reference),
        repository: Some(reference.label),
    })
}

fn repo_overview_depth(depth: Option<usize>) -> usize {
    match depth {
        Some(depth) if (1..=6).contains(&depth) => depth,
        _ => 3,
    }
}

fn repo_overview_max_files(max_files: Option<usize>) -> usize {
    max_files
        .filter(|max| *max > 0)
        .unwrap_or(REPO_OVERVIEW_STRUCTURE_LIMIT)
        .min(REPO_OVERVIEW_MAX_STRUCTURE_LIMIT)
}

fn repo_overview_entries(dir: &Path) -> Vec<RepoOverviewEntry> {
    let ignored: HashSet<&str> = REPO_OVERVIEW_IGNORED_DIRS.iter().copied().collect();
    let mut entries = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if ignored.contains(name.as_str()) {
                return None;
            }
            let file_type = entry.file_type().ok()?;
            Some(RepoOverviewEntry {
                name,
                path: entry.path(),
                directory: file_type.is_dir(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

fn repo_overview_structure(root: &Path, depth: usize, max_files: usize) -> (Vec<String>, bool) {
    fn visit(
        dir: &Path,
        level: usize,
        depth: usize,
        max_files: usize,
        lines: &mut Vec<String>,
        truncated: &mut bool,
    ) {
        if level >= depth {
            return;
        }
        for entry in repo_overview_entries(dir) {
            if lines.len() >= max_files {
                *truncated = true;
                return;
            }
            lines.push(format!(
                "{}{}{}",
                "  ".repeat(level),
                entry.name,
                if entry.directory { "/" } else { "" }
            ));
            if entry.directory {
                visit(&entry.path, level + 1, depth, max_files, lines, truncated);
            }
        }
    }

    let mut lines = Vec::new();
    let mut truncated = false;
    visit(root, 0, depth, max_files, &mut lines, &mut truncated);
    (lines, truncated)
}

fn repo_overview_package_manager(files: &HashSet<String>) -> Option<&'static str> {
    if files.contains("bun.lock") || files.contains("bun.lockb") {
        Some("bun")
    } else if files.contains("pnpm-lock.yaml") {
        Some("pnpm")
    } else if files.contains("yarn.lock") {
        Some("yarn")
    } else if files.contains("package-lock.json") {
        Some("npm")
    } else {
        None
    }
}

fn repo_overview_ecosystems(files: &HashSet<String>) -> Vec<&'static str> {
    let mut result = Vec::new();
    if files.contains("package.json") {
        result.push("Node.js");
    }
    if files.contains("pyproject.toml") || files.contains("requirements.txt") {
        result.push("Python");
    }
    if files.contains("go.mod") {
        result.push("Go");
    }
    if files.contains("Cargo.toml") {
        result.push("Rust");
    }
    if files.contains("Gemfile") {
        result.push("Ruby");
    }
    if files.contains("build.gradle")
        || files.contains("build.gradle.kts")
        || files.contains("pom.xml")
    {
        result.push("Java/Kotlin");
    }
    if files.contains("composer.json") {
        result.push("PHP");
    }
    result
}

fn repo_overview_package_entrypoints(root: &Path, files: &HashSet<String>) -> Vec<String> {
    if !files.contains("package.json") {
        return Vec::new();
    }
    let package_json = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .unwrap_or_else(|| json!({}));
    let mut entrypoints = Vec::new();
    if let Some(main) = package_json.get("main").and_then(|value| value.as_str()) {
        entrypoints.push(format!("main: {main}"));
    }
    if let Some(module) = package_json.get("module").and_then(|value| value.as_str()) {
        entrypoints.push(format!("module: {module}"));
    }
    if let Some(types) = package_json.get("types").and_then(|value| value.as_str()) {
        entrypoints.push(format!("types: {types}"));
    }
    if let Some(bin) = package_json.get("bin") {
        if let Some(bin) = bin.as_str() {
            entrypoints.push(format!("bin: {bin}"));
        } else if let Some(map) = bin.as_object() {
            entrypoints.extend(map.keys().map(|name| format!("bin: {name}")));
        }
    }
    if let Some(exports) = package_json
        .get("exports")
        .and_then(|value| value.as_object())
    {
        entrypoints.extend(
            exports
                .keys()
                .take(10)
                .map(|name| format!("exports: {name}")),
        );
    }
    entrypoints
}

fn repo_overview_common_entrypoints(root: &Path, top_level: &HashSet<String>) -> Vec<String> {
    REPO_OVERVIEW_COMMON_ENTRYPOINTS
        .iter()
        .filter(|file| {
            if !file.contains('/') {
                top_level.contains(**file)
            } else {
                root.join(file).is_file()
            }
        })
        .map(|file| format!("file: {file}"))
        .collect()
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
                "repository": {
                    "type": "string",
                    "description": "Cached repository to inspect, as a git URL, host/path reference, or GitHub owner/repo shorthand"
                },
                "path": {
                    "type": "string",
                    "description": "Directory path to inspect instead of a cached repository"
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum structure depth to include. Defaults to 3."
                },
                "maxFiles": {
                    "type": "integer",
                    "description": "Maximum files to include in the structure. Defaults to 200."
                },
                "max_files": {
                    "type": "integer",
                    "description": "Legacy alias for maxFiles"
                }
            }
        })
    }

    fn execute(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + '_>> {
        Box::pin(async move {
            let params: RepoOverviewParams = serde_json::from_value(params)
                .map_err(|e| anyhow::anyhow!("Invalid repo_overview parameters: {}", e))?;

            let target = resolve_repo_overview_target(&params, &ctx.working_dir)?;
            let depth = repo_overview_depth(params.depth);
            let max_files = repo_overview_max_files(params.max_files);
            let target_display = target.path.display().to_string();
            let permission_pattern = target
                .repository
                .as_deref()
                .unwrap_or(&target_display)
                .to_string();

            super::assert_external_directory(
                &ctx,
                &target.path,
                super::ExternalKind::Directory,
                false,
            )
            .await?;
            ctx.check_permission("repo_overview", &permission_pattern)
                .await?;

            if !target.path.exists() {
                if let Some(repository) = &target.repository {
                    return Err(anyhow::anyhow!(
                        "Repository is not cloned: {repository}. Use repo_clone first. Expected path: {target_display}"
                    ));
                }
                return Err(anyhow::anyhow!("Directory not found: {target_display}"));
            }
            if !target.path.is_dir() {
                return Err(anyhow::anyhow!("Path is not a directory: {target_display}"));
            }

            let entries = repo_overview_entries(&target.path);
            let top_level = entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<HashSet<_>>();
            let dependency_files = REPO_OVERVIEW_DEPENDENCY_FILES
                .iter()
                .filter(|file| top_level.contains(**file))
                .copied()
                .collect::<Vec<_>>();
            let package_manager = repo_overview_package_manager(&top_level);
            let ecosystems = repo_overview_ecosystems(&top_level);
            let mut entrypoints = repo_overview_package_entrypoints(&target.path, &top_level);
            entrypoints.extend(repo_overview_common_entrypoints(&target.path, &top_level));
            let (structure, truncated) = repo_overview_structure(&target.path, depth, max_files);

            let branch = git_output(
                &["symbolic-ref", "--quiet", "--short", "HEAD"],
                &target.path,
            );
            let head = git_output(&["rev-parse", "HEAD"], &target.path);

            let metadata = json!({
                "path": target_display,
                "repository": target.repository.clone(),
                "branch": branch.clone(),
                "head": head.clone(),
                "package_manager": package_manager,
                "ecosystems": ecosystems.clone(),
                "dependency_files": dependency_files.clone(),
                "entrypoints": entrypoints.clone(),
                "depth": depth,
                "maxFiles": max_files,
                "truncated": truncated,
            });

            let mut output = vec![format!("Path: {target_display}")];
            if let Some(repository) = &target.repository {
                output.push(format!("Repository: {repository}"));
            }
            if let Some(branch) = &branch {
                output.push(format!("Branch: {branch}"));
            }
            if let Some(head) = &head {
                output.push(format!("HEAD: {head}"));
            }
            if !ecosystems.is_empty() {
                output.push(format!("Ecosystems: {}", ecosystems.join(", ")));
            }
            if let Some(package_manager) = package_manager {
                output.push(format!("Package manager: {package_manager}"));
            }
            if !dependency_files.is_empty() {
                output.push(format!("Dependency files: {}", dependency_files.join(", ")));
            }
            if !entrypoints.is_empty() {
                output.push("Likely entrypoints:".to_string());
                output.extend(entrypoints.iter().map(|entry| format!("- {entry}")));
            }
            output.push("Top-level structure:".to_string());
            output.extend(structure);
            if truncated {
                output.push("(Structure truncated)".to_string());
            }

            Ok(ToolResult::with_metadata(output.join("\n"), metadata))
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

    #[test]
    fn repo_overview_schema_matches_upstream_repository_parameters() {
        let schema = RepoOverviewTool.parameters_schema();
        let properties = schema["properties"].as_object().unwrap();

        assert!(properties.contains_key("repository"));
        assert!(properties.contains_key("path"));
        assert!(properties.contains_key("depth"));
        assert!(properties.contains_key("maxFiles"));
        assert!(properties.contains_key("max_files"));
        assert!(schema.get("required").is_none() || schema["required"] == json!([]));
    }

    #[tokio::test]
    async fn repo_overview_resolves_repository_to_managed_cache_target() {
        let err = RepoOverviewTool
            .execute(
                json!({
                    "repository": "owner/project",
                    "depth": 2
                }),
                ctx(vec![
                    PermissionRule::allow_tool("external_directory"),
                    PermissionRule::allow_tool("repo_overview"),
                ]),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(
            err.contains("Repository is not cloned: owner/project"),
            "got: {err}"
        );
        assert!(
            err.contains(&crate::global::repos().display().to_string()),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn repo_overview_prefers_path_and_keeps_legacy_relative_path_compatibility() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workspace");
        let repo = root.join("local-repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("package.json"), r#"{"main":"index.js"}"#).unwrap();
        std::fs::write(repo.join("src").join("main.rs"), "fn main() {}\n").unwrap();

        let result = RepoOverviewTool
            .execute(
                json!({
                    "repository": "owner/ignored",
                    "path": "local-repo",
                    "depth": 2
                }),
                ctx_with_working_dir(
                    root.clone(),
                    vec![PermissionRule::allow_tool("repo_overview")],
                ),
            )
            .await
            .unwrap();

        let metadata = result.metadata.unwrap();
        assert_eq!(metadata["path"], json!(repo.display().to_string()));
        assert_eq!(metadata["repository"], json!("owner/ignored"));
        assert_eq!(metadata["depth"], json!(2));
        assert!(result.output.contains(&format!("Path: {}", repo.display())));
        assert!(result.output.contains("Repository: owner/ignored"));
    }

    #[tokio::test]
    async fn repo_overview_uses_repo_overview_permission_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let err = RepoOverviewTool
            .execute(
                json!({
                    "path": repo.display().to_string()
                }),
                ctx_with_working_dir(
                    tmp.path().to_path_buf(),
                    vec![PermissionRule::deny_tool("repo_overview")],
                ),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("denied by permission rule"), "got: {err}");
    }

    #[tokio::test]
    async fn repo_overview_requires_external_directory_permission_for_unsafe_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let external = tmp.path().join("external");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&external).unwrap();

        let err = RepoOverviewTool
            .execute(
                json!({
                    "path": external.display().to_string()
                }),
                ctx_with_working_dir(workspace, vec![PermissionRule::allow_tool("repo_overview")]),
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(err.contains("external_directory"), "got: {err}");
    }

    fn ctx_with_working_dir(
        working_dir: PathBuf,
        rules: crate::permission::Ruleset,
    ) -> ToolContext {
        ToolContext {
            working_dir,
            ..ctx(rules)
        }
    }
}
