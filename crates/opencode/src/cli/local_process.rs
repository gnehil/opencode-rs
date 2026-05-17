#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Curl,
    Npm,
    Pnpm,
    Bun,
    Brew,
    Choco,
    Scoop,
    SelfUpdate,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeAction {
    Command(CommandSpec),
    SelfUpdate { target: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradePlan {
    pub method: InstallMethod,
    pub target: String,
    pub action: UpgradeAction,
}

pub fn pr_checkout_command(number: u64) -> CommandSpec {
    let number = number.to_string();
    CommandSpec::new(
        "gh",
        vec![
            "pr".to_string(),
            "checkout".to_string(),
            number.clone(),
            "--branch".to_string(),
            format!("pr/{number}"),
            "--force".to_string(),
        ],
    )
}

pub fn pr_view_command(number: u64) -> CommandSpec {
    CommandSpec::new(
        "gh",
        vec![
            "pr".to_string(),
            "view".to_string(),
            number.to_string(),
            "--json".to_string(),
            "headRepository,headRepositoryOwner,isCrossRepository,headRefName,body".to_string(),
        ],
    )
}

pub fn opencode_start_command(session_id: Option<&str>) -> CommandSpec {
    let args = session_id
        .map(|session_id| vec!["-s".to_string(), session_id.to_string()])
        .unwrap_or_default();
    CommandSpec::new("opencode", args)
}

pub fn cross_repo_pr_commands(
    fork_owner: &str,
    fork_name: &str,
    head_ref_name: &str,
    existing_remotes: &[String],
    local_branch: &str,
) -> Vec<CommandSpec> {
    let mut commands = Vec::new();
    if !existing_remotes.iter().any(|remote| remote == fork_owner) {
        commands.push(CommandSpec::new(
            "git",
            vec![
                "remote".to_string(),
                "add".to_string(),
                fork_owner.to_string(),
                format!("https://github.com/{fork_owner}/{fork_name}.git"),
            ],
        ));
    }
    commands.push(CommandSpec::new(
        "git",
        vec![
            "branch".to_string(),
            format!("--set-upstream-to={fork_owner}/{head_ref_name}"),
            local_branch.to_string(),
        ],
    ));
    commands
}

pub fn parse_opencode_session_url(body: &str) -> Option<String> {
    const PREFIX: &str = "https://opncd.ai/s/";
    let start = body.find(PREFIX)?;
    let rest = &body[start..];
    let end = rest
        .find(|c: char| {
            !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '/' && c != ':' && c != '.'
        })
        .unwrap_or(rest.len());
    let candidate = &rest[..end];
    (candidate.len() > PREFIX.len()).then(|| candidate.to_string())
}

pub fn parse_imported_session_id(output: &str) -> Option<String> {
    const PREFIX: &str = "Imported session: ";
    let start = output.find(PREFIX)? + PREFIX.len();
    let id = output[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();
    (!id.is_empty()).then_some(id)
}

pub fn resolve_upgrade_method(
    explicit: Option<InstallMethod>,
    detected: InstallMethod,
) -> InstallMethod {
    explicit.unwrap_or(match detected {
        InstallMethod::Unknown => InstallMethod::SelfUpdate,
        method => method,
    })
}

pub fn normalize_upgrade_target(target: Option<&str>, latest: &str) -> String {
    target.unwrap_or(latest).trim_start_matches('v').to_string()
}

pub fn upgrade_plan(
    target: Option<&str>,
    latest: &str,
    explicit: Option<InstallMethod>,
    detected: InstallMethod,
) -> UpgradePlan {
    let target = normalize_upgrade_target(target, latest);
    let method = resolve_upgrade_method(explicit, detected);
    UpgradePlan {
        method,
        target: target.clone(),
        action: upgrade_action(method, &target),
    }
}

pub fn github_workflow_file() -> &'static str {
    ".github/workflows/opencode.yml"
}

pub fn github_workflow_contents(model: &str, env: &[&str]) -> String {
    let env_block = if env.is_empty() {
        String::new()
    } else {
        let vars = env
            .iter()
            .map(|name| format!("\n          {name}: ${{{{ secrets.{name} }}}}"))
            .collect::<String>();
        format!("\n        env:{vars}")
    };

    format!(
        r#"name: opencode

on:
  issue_comment:
    types: [created]
  pull_request_review_comment:
    types: [created]

jobs:
  opencode:
    if: |
      contains(github.event.comment.body, ' /oc') ||
      startsWith(github.event.comment.body, '/oc') ||
      contains(github.event.comment.body, ' /opencode') ||
      startsWith(github.event.comment.body, '/opencode')
    runs-on: ubuntu-latest
    permissions:
      id-token: write
      contents: read
      pull-requests: read
      issues: read
    steps:
      - name: Checkout repository
        uses: actions/checkout@v6
        with:
          persist-credentials: false

      - name: Run opencode
        uses: anomalyco/opencode/github@latest{env_block}
        with:
          model: {model}
"#
    )
}

pub fn attach_run_command(
    base_url: &str,
    dir: Option<&str>,
    continue_last: bool,
    session_id: Option<&str>,
    fork: bool,
    password: Option<&str>,
    username: Option<&str>,
) -> CommandSpec {
    let mut args = vec![
        "run".to_string(),
        "--interactive".to_string(),
        "--attach".to_string(),
        base_url.trim_end_matches('/').to_string(),
    ];
    if let Some(dir) = dir.filter(|dir| !dir.is_empty()) {
        args.extend(["--dir".to_string(), dir.to_string()]);
    }
    if continue_last {
        args.push("--continue".to_string());
    }
    if let Some(session_id) = session_id.filter(|session_id| !session_id.is_empty()) {
        args.extend(["--session".to_string(), session_id.to_string()]);
    }
    if fork {
        args.push("--fork".to_string());
    }
    if let Some(password) = password.filter(|password| !password.is_empty()) {
        args.extend(["--password".to_string(), password.to_string()]);
    }
    if let Some(username) = username.filter(|username| !username.is_empty()) {
        args.extend(["--username".to_string(), username.to_string()]);
    }
    CommandSpec::new("opencode", args)
}

pub fn join_run_message(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains(' ') {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn resolve_run_input(value: &str, piped: Option<&str>) -> Option<String> {
    if value.is_empty() {
        return piped.filter(|input| !input.is_empty()).map(str::to_string);
    }

    match piped.filter(|input| !input.is_empty()) {
        Some(piped) => Some(format!("{value}\n{piped}")),
        None => Some(value.to_string()),
    }
}

fn upgrade_action(method: InstallMethod, target: &str) -> UpgradeAction {
    match method {
        InstallMethod::Curl | InstallMethod::SelfUpdate | InstallMethod::Unknown => {
            UpgradeAction::SelfUpdate {
                target: target.to_string(),
            }
        }
        InstallMethod::Npm => UpgradeAction::Command(CommandSpec::new(
            "npm",
            vec![
                "install".to_string(),
                "-g".to_string(),
                format!("opencode-ai@{target}"),
            ],
        )),
        InstallMethod::Pnpm => UpgradeAction::Command(CommandSpec::new(
            "pnpm",
            vec![
                "install".to_string(),
                "-g".to_string(),
                format!("opencode-ai@{target}"),
            ],
        )),
        InstallMethod::Bun => UpgradeAction::Command(CommandSpec::new(
            "bun",
            vec![
                "install".to_string(),
                "-g".to_string(),
                format!("opencode-ai@{target}"),
            ],
        )),
        InstallMethod::Brew => {
            UpgradeAction::Command(CommandSpec::new("brew", ["upgrade", "opencode"]))
        }
        InstallMethod::Choco => UpgradeAction::Command(CommandSpec::new(
            "choco",
            vec![
                "upgrade".to_string(),
                "opencode".to_string(),
                format!("--version={target}"),
                "-y".to_string(),
            ],
        )),
        InstallMethod::Scoop => UpgradeAction::Command(CommandSpec::new(
            "scoop",
            vec!["install".to_string(), format!("opencode@{target}")],
        )),
    }
}
