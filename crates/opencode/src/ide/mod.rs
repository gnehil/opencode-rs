//! IDE detection and extension installation.
//!
//! Mirrors the TypeScript `Ide` module: detect the editor opencode is running
//! inside, check whether opencode was launched by the VSCode extension, and
//! install the opencode extension into a supported VSCode-family editor.

use std::process::Command;

/// Editors that ship a `--install-extension` compatible CLI, as
/// `(display name, cli command)`. Order matters: [`detect`] returns the first
/// match, and "Visual Studio Code - Insiders" must be checked before "Visual
/// Studio Code" since the latter is a substring of the former.
pub const SUPPORTED_IDES: &[(&str, &str)] = &[
    ("Windsurf", "windsurf"),
    ("Visual Studio Code - Insiders", "code-insiders"),
    ("Visual Studio Code", "code"),
    ("Cursor", "cursor"),
    ("VSCodium", "codium"),
];

const EXTENSION_ID: &str = "sst-dev.opencode";

#[derive(Debug, thiserror::Error)]
pub enum IdeError {
    #[error("Unknown IDE: {0}")]
    Unknown(String),
    #[error("opencode extension is already installed")]
    AlreadyInstalled,
    #[error("failed to install opencode extension: {stderr}")]
    InstallFailed { stderr: String },
}

/// Detect the IDE opencode is running inside, or `"unknown"`. Matches the TS
/// heuristic: only when `TERM_PROGRAM=vscode`, inspect `GIT_ASKPASS` for a
/// known editor name.
pub fn detect() -> &'static str {
    detect_from(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("GIT_ASKPASS").ok().as_deref(),
    )
}

fn detect_from(term_program: Option<&str>, git_askpass: Option<&str>) -> &'static str {
    if term_program == Some("vscode") {
        if let Some(askpass) = git_askpass {
            for (name, _) in SUPPORTED_IDES {
                if askpass.contains(name) {
                    return name;
                }
            }
        }
    }
    "unknown"
}

/// True when opencode was launched by the VSCode extension itself, so there is
/// nothing to install.
pub fn already_installed() -> bool {
    already_installed_from(std::env::var("OPENCODE_CALLER").ok().as_deref())
}

fn already_installed_from(caller: Option<&str>) -> bool {
    matches!(caller, Some("vscode") | Some("vscode-insiders"))
}

/// Install the opencode extension into `ide` (a [`SUPPORTED_IDES`] display
/// name) by shelling out to that editor's `--install-extension` CLI.
pub fn install(ide: &str) -> Result<(), IdeError> {
    let cmd = SUPPORTED_IDES
        .iter()
        .find(|(name, _)| *name == ide)
        .map(|(_, cmd)| *cmd)
        .ok_or_else(|| IdeError::Unknown(ide.to_string()))?;

    let output = Command::new(cmd)
        .args(["--install-extension", EXTENSION_ID])
        .output()
        .map_err(|e| IdeError::InstallFailed {
            stderr: e.to_string(),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    tracing::info!(ide, %stdout, %stderr, "ide extension install");

    if !output.status.success() {
        return Err(IdeError::InstallFailed {
            stderr: stderr.into_owned(),
        });
    }
    if stdout.contains("already installed") {
        return Err(IdeError::AlreadyInstalled);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_visual_studio_code() {
        assert_eq!(
            detect_from(
                Some("vscode"),
                Some("/path/to/Visual Studio Code.app/Contents/Resources/app/extensions/git/dist/askpass.sh")
            ),
            "Visual Studio Code"
        );
    }

    #[test]
    fn detects_visual_studio_code_insiders() {
        assert_eq!(
            detect_from(
                Some("vscode"),
                Some("/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/extensions/git/dist/askpass.sh")
            ),
            "Visual Studio Code - Insiders"
        );
    }

    #[test]
    fn detects_cursor() {
        assert_eq!(
            detect_from(
                Some("vscode"),
                Some("/path/to/Cursor.app/Contents/Resources/app/extensions/git/dist/askpass.sh")
            ),
            "Cursor"
        );
    }

    #[test]
    fn detects_vscodium() {
        assert_eq!(
            detect_from(
                Some("vscode"),
                Some("/path/to/VSCodium.app/Contents/Resources/app/extensions/git/dist/askpass.sh")
            ),
            "VSCodium"
        );
    }

    #[test]
    fn detects_windsurf() {
        assert_eq!(
            detect_from(
                Some("vscode"),
                Some("/path/to/Windsurf.app/Contents/Resources/app/extensions/git/dist/askpass.sh")
            ),
            "Windsurf"
        );
    }

    #[test]
    fn unknown_when_term_program_is_not_vscode() {
        assert_eq!(
            detect_from(
                Some("iTerm2"),
                Some("/Applications/Visual Studio Code - Insiders.app/Contents/Resources/app/extensions/git/dist/askpass.sh")
            ),
            "unknown"
        );
    }

    #[test]
    fn unknown_when_git_askpass_has_no_ide_name() {
        assert_eq!(
            detect_from(Some("vscode"), Some("/path/to/unknown/askpass.sh")),
            "unknown"
        );
    }

    #[test]
    fn already_installed_recognizes_vscode_callers() {
        assert!(already_installed_from(Some("vscode")));
        assert!(already_installed_from(Some("vscode-insiders")));
        assert!(!already_installed_from(Some("unknown")));
        assert!(!already_installed_from(None));
    }

    #[test]
    fn install_rejects_unknown_ide() {
        assert!(matches!(install("Notepad"), Err(IdeError::Unknown(_))));
    }
}
