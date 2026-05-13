use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
};

use super::session_handlers::AppState;

const GIT_ARGS: &[&str] = &[
    "--no-optional-locks",
    "-c",
    "core.autocrlf=false",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.longpaths=true",
    "-c",
    "core.symlinks=true",
    "-c",
    "core.quotepath=false",
];

const MAX_PATCH_UNIFIED_ARG: &str = "--unified=2147483647";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInfo {
    home: String,
    state: String,
    config: String,
    worktree: String,
    directory: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitStatusKind {
    Added,
    Deleted,
    Modified,
}

impl GitStatusKind {
    fn from_code(code: &str) -> Self {
        if code == "??" || (code.contains('A') && !code.contains('D')) {
            Self::Added
        } else if code.contains('D') && !code.contains('A') {
            Self::Deleted
        } else {
            Self::Modified
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Modified => "modified",
        }
    }
}

#[derive(Clone, Debug)]
struct GitItem {
    file: String,
    code: String,
    status: GitStatusKind,
}

#[derive(Clone, Debug, Default)]
struct GitStat {
    additions: usize,
    deletions: usize,
}

#[derive(Clone, Debug)]
struct GitBase {
    name: String,
    reference: String,
}

#[derive(Serialize)]
pub struct VcsInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_branch: Option<String>,
}

#[derive(Serialize)]
pub struct VcsFileStatus {
    file: String,
    additions: usize,
    deletions: usize,
    status: &'static str,
}

#[derive(Serialize)]
pub struct VcsFileDiff {
    file: String,
    patch: String,
    additions: usize,
    deletions: usize,
    status: &'static str,
}

#[derive(Deserialize)]
pub struct VcsDiffQuery {
    mode: Option<String>,
}

#[derive(Deserialize)]
pub struct VcsApplyInput {
    patch: String,
}

#[derive(Serialize)]
pub struct FormatterStatus {
    name: &'static str,
    extensions: Vec<&'static str>,
    enabled: bool,
}

struct FormatterInfo {
    name: &'static str,
    command: &'static str,
    extensions: &'static [&'static str],
}

const FORMATTERS: &[FormatterInfo] = &[
    FormatterInfo {
        name: "gofmt",
        command: "gofmt",
        extensions: &[".go"],
    },
    FormatterInfo {
        name: "mix",
        command: "mix",
        extensions: &[".ex", ".exs", ".eex", ".heex", ".leex", ".neex", ".sface"],
    },
    FormatterInfo {
        name: "prettier",
        command: "prettier",
        extensions: &[
            ".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts", ".html", ".htm", ".css",
            ".scss", ".sass", ".less", ".vue", ".svelte", ".json", ".jsonc", ".yaml", ".yml",
            ".toml", ".xml", ".md", ".mdx", ".graphql", ".gql",
        ],
    },
    FormatterInfo {
        name: "oxfmt",
        command: "oxfmt",
        extensions: &[".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts"],
    },
    FormatterInfo {
        name: "biome",
        command: "biome",
        extensions: &[
            ".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts", ".html", ".htm", ".css",
            ".scss", ".sass", ".less", ".vue", ".svelte", ".json", ".jsonc", ".yaml", ".yml",
            ".toml", ".xml", ".md", ".mdx", ".graphql", ".gql",
        ],
    },
    FormatterInfo {
        name: "zig",
        command: "zig",
        extensions: &[".zig", ".zon"],
    },
    FormatterInfo {
        name: "clang-format",
        command: "clang-format",
        extensions: &[
            ".c", ".cc", ".cpp", ".cxx", ".c++", ".h", ".hh", ".hpp", ".hxx", ".h++", ".ino", ".C",
            ".H",
        ],
    },
    FormatterInfo {
        name: "ktlint",
        command: "ktlint",
        extensions: &[".kt", ".kts"],
    },
    FormatterInfo {
        name: "ruff",
        command: "ruff",
        extensions: &[".py", ".pyi"],
    },
    FormatterInfo {
        name: "air",
        command: "air",
        extensions: &[".R"],
    },
    FormatterInfo {
        name: "uv",
        command: "uv",
        extensions: &[".py", ".pyi"],
    },
    FormatterInfo {
        name: "rubocop",
        command: "rubocop",
        extensions: &[".rb", ".rake", ".gemspec", ".ru"],
    },
    FormatterInfo {
        name: "standardrb",
        command: "standardrb",
        extensions: &[".rb", ".rake", ".gemspec", ".ru"],
    },
    FormatterInfo {
        name: "htmlbeautifier",
        command: "htmlbeautifier",
        extensions: &[".erb", ".html.erb"],
    },
    FormatterInfo {
        name: "dart",
        command: "dart",
        extensions: &[".dart"],
    },
    FormatterInfo {
        name: "ocamlformat",
        command: "ocamlformat",
        extensions: &[".ml", ".mli"],
    },
    FormatterInfo {
        name: "terraform",
        command: "terraform",
        extensions: &[".tf", ".tfvars"],
    },
    FormatterInfo {
        name: "latexindent",
        command: "latexindent",
        extensions: &[".tex"],
    },
    FormatterInfo {
        name: "gleam",
        command: "gleam",
        extensions: &[".gleam"],
    },
    FormatterInfo {
        name: "shfmt",
        command: "shfmt",
        extensions: &[".sh", ".bash"],
    },
    FormatterInfo {
        name: "nixfmt",
        command: "nixfmt",
        extensions: &[".nix"],
    },
    FormatterInfo {
        name: "rustfmt",
        command: "rustfmt",
        extensions: &[".rs"],
    },
    FormatterInfo {
        name: "pint",
        command: "pint",
        extensions: &[".php"],
    },
    FormatterInfo {
        name: "ormolu",
        command: "ormolu",
        extensions: &[".hs"],
    },
    FormatterInfo {
        name: "cljfmt",
        command: "cljfmt",
        extensions: &[".clj", ".cljs", ".cljc", ".edn"],
    },
    FormatterInfo {
        name: "dfmt",
        command: "dfmt",
        extensions: &[".d"],
    },
];

pub async fn lsp_status(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "languages": ["rust", "typescript", "python"],
        "status": "available"
    }))
}

pub async fn tool_list(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let tools = [
        "bash",
        "read",
        "write",
        "edit",
        "glob",
        "grep",
        "task",
        "webfetch",
        "websearch",
        "lsp_diagnostics",
        "lsp_goto_definition",
        "lsp_find_references",
        "lsp_rename",
        "lsp_symbols",
    ];

    Json(json!({
        "tools": tools.iter().map(|t| json!({
            "name": t,
            "available": true
        })).collect::<Vec<_>>()
    }))
}

pub async fn command_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::command::CommandInfo>>, StatusCode> {
    let commands = crate::command::load_commands(&state.workspace_root, state.config.as_ref())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(commands))
}

pub async fn skill_list(State(_state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(json!({
        "skills": [
            {"name": "playwright", "description": "Browser automation via Playwright MCP"},
            {"name": "frontend-ui-ux", "description": "Designer-turned-developer UI/UX"},
            {"name": "git-master", "description": "Git operations"},
            {"name": "review-work", "description": "Post-implementation review orchestrator"}
        ]
    }))
}

pub async fn instance_dispose(State(_state): State<Arc<AppState>>) -> Json<bool> {
    Json(true)
}

pub async fn path_info(State(state): State<Arc<AppState>>) -> Json<PathInfo> {
    let worktree = state.workspace_root.to_string_lossy().to_string();
    Json(PathInfo {
        home: crate::global::home().to_string_lossy().to_string(),
        state: crate::global::state().to_string_lossy().to_string(),
        config: crate::global::config().to_string_lossy().to_string(),
        worktree: worktree.clone(),
        directory: worktree,
    })
}

pub async fn vcs_info(State(state): State<Arc<AppState>>) -> Json<VcsInfo> {
    let root = &state.workspace_root;
    if !is_git_worktree(root) {
        return Json(VcsInfo {
            branch: None,
            default_branch: None,
        });
    }

    Json(VcsInfo {
        branch: git_branch(root),
        default_branch: git_default_branch(root).map(|base| base.name),
    })
}

pub async fn vcs_status(State(state): State<Arc<AppState>>) -> Json<Vec<VcsFileStatus>> {
    Json(vcs_status_for_root(&state.workspace_root))
}

pub async fn vcs_diff(
    State(state): State<Arc<AppState>>,
    Query(query): Query<VcsDiffQuery>,
) -> Result<Json<Vec<VcsFileDiff>>, StatusCode> {
    let mode = query.mode.as_deref().unwrap_or("git");
    if mode != "git" && mode != "branch" {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(Json(vcs_diff_for_root(&state.workspace_root, mode)))
}

pub async fn vcs_diff_raw(State(state): State<Arc<AppState>>) -> Response {
    let raw = vcs_diff_raw_for_root(&state.workspace_root);
    ([(header::CONTENT_TYPE, "text/x-diff; charset=utf-8")], raw).into_response()
}

pub async fn vcs_apply(
    State(state): State<Arc<AppState>>,
    Json(input): Json<VcsApplyInput>,
) -> Response {
    if !is_git_worktree(&state.workspace_root) {
        return vcs_apply_error(
            "Patch can't be applied because the project is not git-based",
            "non-git",
        );
    }

    match git_apply_patch(&state.workspace_root, &input.patch) {
        Ok(output) if output.status.success() => Json(json!({ "applied": true })).into_response(),
        _ => vcs_apply_error("Patch can't be applied", "not-clean"),
    }
}

pub async fn formatter_status(State(state): State<Arc<AppState>>) -> Json<Vec<FormatterStatus>> {
    Json(formatter_status_for_state(&state))
}

fn vcs_apply_error(message: &str, reason: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "name": "VcsApplyError",
            "data": {
                "message": message,
                "reason": reason
            }
        })),
    )
        .into_response()
}

fn is_git_worktree(root: &Path) -> bool {
    git_output(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_branch(root: &Path) -> Option<String> {
    git_success_text(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn git_default_branch(root: &Path) -> Option<GitBase> {
    let remotes = git_lines(root, &["remote"]);
    let remote = if remotes.iter().any(|remote| remote == "origin") {
        Some("origin".to_string())
    } else if remotes.len() == 1 {
        remotes.first().cloned()
    } else if remotes.iter().any(|remote| remote == "upstream") {
        Some("upstream".to_string())
    } else {
        remotes.first().cloned()
    };

    if let Some(remote) = remote {
        if let Some(head) = git_success_text(
            root,
            &["symbolic-ref", &format!("refs/remotes/{remote}/HEAD")],
        ) {
            let reference = head
                .trim()
                .strip_prefix("refs/remotes/")
                .unwrap_or(head.trim())
                .to_string();
            if let Some(name) = reference.strip_prefix(&format!("{remote}/")) {
                if !name.is_empty() {
                    return Some(GitBase {
                        name: name.to_string(),
                        reference,
                    });
                }
            }
        }
    }

    let refs = git_lines(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    );
    if let Some(configured) = git_success_text(root, &["config", "init.defaultBranch"])
        .map(|value| value.trim().to_string())
        .filter(|value| refs.contains(value))
    {
        return Some(GitBase {
            name: configured.clone(),
            reference: configured,
        });
    }
    if refs.iter().any(|branch| branch == "main") {
        return Some(GitBase {
            name: "main".to_string(),
            reference: "main".to_string(),
        });
    }
    if refs.iter().any(|branch| branch == "master") {
        return Some(GitBase {
            name: "master".to_string(),
            reference: "master".to_string(),
        });
    }
    None
}

fn git_has_head(root: &Path) -> bool {
    git_output(root, &["rev-parse", "--verify", "HEAD"])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_merge_base(root: &Path, base: &str) -> Option<String> {
    git_success_text(root, &["merge-base", base, "HEAD"])
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn vcs_status_for_root(root: &Path) -> Vec<VcsFileStatus> {
    if !is_git_worktree(root) {
        return Vec::new();
    }

    let items = git_status_items(root);
    let stats = if git_has_head(root) {
        git_stats(root, "HEAD")
    } else {
        HashMap::new()
    };
    status_files(root, &items, &stats)
}

fn vcs_diff_for_root(root: &Path, mode: &str) -> Vec<VcsFileDiff> {
    if !is_git_worktree(root) {
        return Vec::new();
    }

    if mode == "branch" {
        let Some(default_branch) = git_default_branch(root) else {
            return Vec::new();
        };
        if git_branch(root).as_deref() == Some(default_branch.name.as_str()) {
            return Vec::new();
        }
        let Some(base) = git_merge_base(root, &default_branch.reference) else {
            return Vec::new();
        };
        return diff_against_ref(root, &base);
    }

    if git_has_head(root) {
        diff_against_ref(root, "HEAD")
    } else {
        diff_files(root, git_status_items(root), None, &HashMap::new())
    }
}

fn vcs_diff_raw_for_root(root: &Path) -> String {
    if !is_git_worktree(root) {
        return String::new();
    }

    let tracked = if git_has_head(root) {
        git_text_allow(root, &patch_all_args("HEAD"), &[0]).unwrap_or_default()
    } else {
        String::new()
    };
    let untracked = git_status_items(root)
        .into_iter()
        .filter(|item| item.code == "??")
        .filter_map(|item| patch_untracked(root, &item.file))
        .collect::<Vec<_>>();

    std::iter::once(tracked)
        .chain(untracked)
        .filter(|patch| !patch.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn diff_against_ref(root: &Path, reference: &str) -> Vec<VcsFileDiff> {
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for item in git_diff_items(root, reference).into_iter().chain(
        git_status_items(root)
            .into_iter()
            .filter(|item| item.code == "??"),
    ) {
        if seen.insert(item.file.clone()) {
            items.push(item);
        }
    }

    let stats = git_stats(root, reference);
    diff_files(root, items, Some(reference), &stats)
}

fn status_files(
    root: &Path,
    items: &[GitItem],
    stats: &HashMap<String, GitStat>,
) -> Vec<VcsFileStatus> {
    let mut items = items.to_vec();
    items.sort_by(|a, b| a.file.cmp(&b.file));
    items
        .into_iter()
        .map(|item| {
            let stat = stats
                .get(&item.file)
                .cloned()
                .or_else(|| {
                    (item.status == GitStatusKind::Added)
                        .then(|| stat_untracked(root, &item.file))
                        .flatten()
                })
                .unwrap_or_default();
            VcsFileStatus {
                file: item.file,
                additions: stat.additions,
                deletions: stat.deletions,
                status: item.status.as_str(),
            }
        })
        .collect()
}

fn diff_files(
    root: &Path,
    mut items: Vec<GitItem>,
    reference: Option<&str>,
    stats: &HashMap<String, GitStat>,
) -> Vec<VcsFileDiff> {
    items.sort_by(|a, b| a.file.cmp(&b.file));
    items
        .into_iter()
        .map(|item| {
            let stat = stats
                .get(&item.file)
                .cloned()
                .or_else(|| {
                    (item.status == GitStatusKind::Added)
                        .then(|| stat_untracked(root, &item.file))
                        .flatten()
                })
                .unwrap_or_default();
            let patch =
                patch_for_item(root, reference, &item).unwrap_or_else(|| empty_patch(&item.file));
            VcsFileDiff {
                file: item.file,
                patch,
                additions: stat.additions,
                deletions: stat.deletions,
                status: item.status.as_str(),
            }
        })
        .collect()
}

fn git_status_items(root: &Path) -> Vec<GitItem> {
    git_text_allow(
        root,
        &[
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--no-renames",
            "-z",
            "--",
            ".",
        ],
        &[0],
    )
    .unwrap_or_default()
    .split('\0')
    .filter(|item| !item.is_empty())
    .filter_map(|item| {
        let code = item.get(0..2)?.to_string();
        let file = item.get(3..)?.to_string();
        (!file.is_empty()).then(|| GitItem {
            file,
            status: GitStatusKind::from_code(&code),
            code,
        })
    })
    .collect()
}

fn git_diff_items(root: &Path, reference: &str) -> Vec<GitItem> {
    let args = [
        "diff",
        "--no-ext-diff",
        "--no-renames",
        "--name-status",
        "-z",
        reference,
        "--",
        ".",
    ];
    let parts = git_text_allow(root, &args, &[0])
        .unwrap_or_default()
        .split('\0')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    parts
        .chunks(2)
        .filter_map(|chunk| {
            let code = chunk.first()?.clone();
            let file = chunk.get(1)?.clone();
            Some(GitItem {
                file,
                status: GitStatusKind::from_code(&code),
                code,
            })
        })
        .collect()
}

fn git_stats(root: &Path, reference: &str) -> HashMap<String, GitStat> {
    let args = [
        "diff",
        "--no-ext-diff",
        "--no-renames",
        "--numstat",
        "-z",
        reference,
        "--",
        ".",
    ];
    git_text_allow(root, &args, &[0])
        .unwrap_or_default()
        .split('\0')
        .filter_map(parse_numstat)
        .collect()
}

fn stat_untracked(root: &Path, file: &str) -> Option<GitStat> {
    git_text_allow(
        root,
        &["diff", "--no-index", "--numstat", "--", "/dev/null", file],
        &[0, 1],
    )
    .as_deref()
    .and_then(|text| text.lines().next())
    .and_then(parse_numstat)
    .map(|(_, stat)| stat)
}

fn parse_numstat(line: &str) -> Option<(String, GitStat)> {
    let first = line.find('\t')?;
    let second = line[first + 1..].find('\t').map(|idx| first + 1 + idx)?;
    let additions = parse_git_count(&line[..first]);
    let deletions = parse_git_count(&line[first + 1..second]);
    let file = line[second + 1..].to_string();
    (!file.is_empty()).then_some((
        file,
        GitStat {
            additions,
            deletions,
        },
    ))
}

fn parse_git_count(value: &str) -> usize {
    if value == "-" {
        0
    } else {
        value.parse().unwrap_or(0)
    }
}

fn patch_for_item(root: &Path, reference: Option<&str>, item: &GitItem) -> Option<String> {
    if item.code == "??" || reference.is_none() {
        return patch_untracked(root, &item.file);
    }
    let reference = reference?;
    git_text_allow(root, &patch_args(reference, &item.file), &[0])
        .filter(|text| !text.trim().is_empty())
}

fn patch_args<'a>(reference: &'a str, file: &'a str) -> Vec<&'a str> {
    vec![
        "diff",
        "--patch",
        "--no-ext-diff",
        "--no-renames",
        MAX_PATCH_UNIFIED_ARG,
        reference,
        "--",
        file,
    ]
}

fn patch_all_args(reference: &str) -> Vec<&str> {
    vec![
        "diff",
        "--patch",
        "--no-ext-diff",
        "--no-renames",
        MAX_PATCH_UNIFIED_ARG,
        reference,
        "--",
        ".",
    ]
}

fn patch_untracked(root: &Path, file: &str) -> Option<String> {
    git_text_allow(
        root,
        &[
            "diff",
            "--no-index",
            "--patch",
            "--no-ext-diff",
            "--no-renames",
            MAX_PATCH_UNIFIED_ARG,
            "--",
            "/dev/null",
            file,
        ],
        &[0, 1],
    )
    .filter(|text| !text.trim().is_empty())
}

fn empty_patch(file: &str) -> String {
    format!("Index: {file}\n===================================================================\n")
}

fn git_lines(root: &Path, args: &[&str]) -> Vec<String> {
    git_success_text(root, args)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn git_success_text(root: &Path, args: &[&str]) -> Option<String> {
    git_text_allow(root, args, &[0])
}

fn git_text_allow(root: &Path, args: &[&str], success_codes: &[i32]) -> Option<String> {
    let output = git_output(root, args).ok()?;
    let code = output.status.code()?;
    if !success_codes.contains(&code) {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn git_output(root: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        .args(GIT_ARGS)
        .args(args)
        .current_dir(root)
        .output()
}

fn git_apply_patch(root: &Path, patch: &str) -> std::io::Result<Output> {
    let mut child = Command::new("git")
        .args(GIT_ARGS)
        .args(["apply", "-"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(patch.as_bytes())?;
    }
    child.wait_with_output()
}

fn formatter_status_for_state(state: &AppState) -> Vec<FormatterStatus> {
    let Some(config) = state.config.as_ref() else {
        return Vec::new();
    };
    let Some(formatter_config) = config.formatter.as_ref() else {
        return Vec::new();
    };

    let options = match formatter_config {
        crate::config::FormatterConfig::Disabled(false) => return Vec::new(),
        crate::config::FormatterConfig::Disabled(true) => None,
        crate::config::FormatterConfig::Enabled(options) => Some(options),
    };

    FORMATTERS
        .iter()
        .filter(|formatter| !formatter_disabled(formatter.name, options))
        .map(|formatter| FormatterStatus {
            name: formatter.name,
            extensions: formatter.extensions.to_vec(),
            enabled: formatter_enabled(formatter, &state.workspace_root),
        })
        .collect()
}

fn formatter_disabled(name: &str, options: Option<&crate::config::FormatterOptions>) -> bool {
    let Some(options) = options else {
        return false;
    };
    match name {
        "prettier" => options.prettier == Some(false),
        "rustfmt" => options.rustfmt == Some(false),
        _ => false,
    }
}

fn formatter_enabled(formatter: &FormatterInfo, root: &Path) -> bool {
    match formatter.name {
        "prettier" => command_exists(formatter.command) && package_dep_exists(root, "prettier"),
        "biome" => {
            command_exists(formatter.command)
                && (find_up(root, "biome.json").is_some() || find_up(root, "biome.jsonc").is_some())
        }
        "clang-format" => {
            command_exists(formatter.command) && find_up(root, ".clang-format").is_some()
        }
        "ruff" => command_exists(formatter.command) && ruff_configured(root),
        "uv" => command_exists(formatter.command) && !ruff_configured(root),
        "ocamlformat" => {
            command_exists(formatter.command) && find_up(root, ".ocamlformat").is_some()
        }
        "pint" => command_exists(formatter.command) && package_dep_exists(root, "laravel/pint"),
        _ => command_exists(formatter.command),
    }
}

fn command_exists(command: &str) -> bool {
    which::which(command).is_ok()
}

fn package_dep_exists(root: &Path, needle: &str) -> bool {
    find_up(root, "package.json")
        .or_else(|| find_up(root, "composer.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| content.contains(needle))
        .unwrap_or(false)
}

fn ruff_configured(root: &Path) -> bool {
    for config in ["ruff.toml", ".ruff.toml"] {
        if find_up(root, config).is_some() {
            return true;
        }
    }
    if find_up(root, "pyproject.toml")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| content.contains("[tool.ruff]") || content.contains("ruff"))
        .unwrap_or(false)
    {
        return true;
    }
    for dep in ["requirements.txt", "Pipfile"] {
        if find_up(root, dep)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|content| content.contains("ruff"))
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn find_up(start: &Path, name: &str) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        current = dir.parent();
    }
    None
}
