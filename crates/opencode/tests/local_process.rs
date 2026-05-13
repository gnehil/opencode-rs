#[path = "../src/cli/local_process.rs"]
mod local_process;

use local_process::{
    attach_select_session_endpoint, cross_repo_pr_commands, github_workflow_contents,
    github_workflow_file, normalize_upgrade_target, opencode_start_command,
    parse_imported_session_id, parse_opencode_session_url, pr_checkout_command, pr_view_command,
    resolve_upgrade_method, upgrade_plan, CommandSpec, InstallMethod, UpgradeAction,
};

#[test]
fn pr_checkout_uses_forced_pr_branch() {
    assert_eq!(
        pr_checkout_command(42),
        CommandSpec {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "checkout".to_string(),
                "42".to_string(),
                "--branch".to_string(),
                "pr/42".to_string(),
                "--force".to_string(),
            ],
        }
    );
}

#[test]
fn pr_view_fetches_cross_repo_and_body_fields() {
    assert_eq!(
        pr_view_command(7),
        CommandSpec {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "view".to_string(),
                "7".to_string(),
                "--json".to_string(),
                "headRepository,headRepositoryOwner,isCrossRepository,headRefName,body".to_string(),
            ],
        }
    );
}

#[test]
fn session_url_parser_extracts_opencode_share_link() {
    let body = "Review notes\n\nSession: https://opncd.ai/s/ses_abc-123_XyZ\nThanks";

    assert_eq!(
        parse_opencode_session_url(body),
        Some("https://opncd.ai/s/ses_abc-123_XyZ".to_string())
    );
}

#[test]
fn imported_session_parser_extracts_cli_session_id() {
    assert_eq!(
        parse_imported_session_id("Imported session: ses_123-ABC_xyz\n"),
        Some("ses_123-ABC_xyz".to_string())
    );
}

#[test]
fn opencode_start_uses_imported_session_when_available() {
    assert_eq!(
        opencode_start_command(Some("ses_123")),
        CommandSpec {
            program: "opencode".to_string(),
            args: vec!["-s".to_string(), "ses_123".to_string()],
        }
    );
    assert_eq!(
        opencode_start_command(None),
        CommandSpec {
            program: "opencode".to_string(),
            args: Vec::new(),
        }
    );
}

#[test]
fn upgrade_resolution_prefers_explicit_then_detected_then_self_update() {
    assert_eq!(
        resolve_upgrade_method(Some(InstallMethod::Npm), InstallMethod::Brew),
        InstallMethod::Npm
    );
    assert_eq!(
        resolve_upgrade_method(None, InstallMethod::Brew),
        InstallMethod::Brew
    );
    assert_eq!(
        resolve_upgrade_method(None, InstallMethod::Unknown),
        InstallMethod::SelfUpdate
    );
}

#[test]
fn upgrade_plan_normalizes_target_and_builds_package_command() {
    assert_eq!(
        normalize_upgrade_target(Some("v0.1.48"), "0.1.99"),
        "0.1.48"
    );

    let plan = upgrade_plan(
        None,
        "0.1.99",
        Some(InstallMethod::Pnpm),
        InstallMethod::Unknown,
    );
    assert_eq!(plan.method, InstallMethod::Pnpm);
    assert_eq!(plan.target, "0.1.99");
    assert_eq!(
        plan.action,
        UpgradeAction::Command(CommandSpec {
            program: "pnpm".to_string(),
            args: vec![
                "install".to_string(),
                "-g".to_string(),
                "opencode-ai@0.1.99".to_string(),
            ],
        })
    );
}

#[test]
fn cross_repo_pr_adds_remote_when_missing_and_sets_upstream() {
    let commands =
        cross_repo_pr_commands("alice", "demo", "feature", &["origin".to_string()], "pr/12");

    assert_eq!(
        commands,
        vec![
            CommandSpec {
                program: "git".to_string(),
                args: vec![
                    "remote".to_string(),
                    "add".to_string(),
                    "alice".to_string(),
                    "https://github.com/alice/demo.git".to_string(),
                ],
            },
            CommandSpec {
                program: "git".to_string(),
                args: vec![
                    "branch".to_string(),
                    "--set-upstream-to=alice/feature".to_string(),
                    "pr/12".to_string(),
                ],
            },
        ]
    );
}

#[test]
fn github_workflow_install_content_uses_bundled_action_and_model() {
    assert_eq!(github_workflow_file(), ".github/workflows/opencode.yml");
    let workflow = github_workflow_contents("openai/gpt-4.1", &["OPENAI_API_KEY"]);

    assert!(workflow.contains("uses: anomalyco/opencode/github@latest"));
    assert!(workflow.contains("model: openai/gpt-4.1"));
    assert!(workflow.contains("OPENAI_API_KEY: ${{ secrets.OPENAI_API_KEY }}"));
    assert!(workflow.contains("startsWith(github.event.comment.body, '/oc')"));
}

#[test]
fn attach_select_session_endpoint_normalizes_base_url() {
    assert_eq!(
        attach_select_session_endpoint("http://127.0.0.1:4096/", "ses_123"),
        "http://127.0.0.1:4096/tui/select-session"
    );
}
