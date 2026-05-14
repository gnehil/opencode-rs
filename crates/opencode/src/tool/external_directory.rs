use std::path::{Component, Path, PathBuf};

use super::context::ToolContext;

/// Whether an external `target` refers to a file or a directory. For a file we
/// ask permission for its parent directory; for a directory we ask for the
/// directory itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalKind {
    File,
    Directory,
}

/// Lexically normalize a path (resolve `.` and `..` without touching the
/// filesystem), mirroring Node's `path.relative` semantics used by the
/// TypeScript `AppFileSystem.contains` check.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when `child` is `base` or lives somewhere beneath it.
fn contains_path(base: &Path, child: &Path) -> bool {
    normalize(child).starts_with(normalize(base))
}

/// Guard used by file-touching tools (`read`, `write`, `edit`, `glob`,
/// `grep`, `apply_patch`, `lsp`) before they operate on a `target` that may
/// fall outside the session working directory. Paths inside the workspace
/// pass through; anything outside requires an `external_directory` permission
/// keyed on the parent directory glob, matching the TypeScript
/// `assertExternalDirectory` behavior.
pub async fn assert_external_directory(
    ctx: &ToolContext,
    target: &Path,
    kind: ExternalKind,
    bypass: bool,
) -> anyhow::Result<()> {
    if bypass {
        return Ok(());
    }

    let full = if target.is_absolute() {
        normalize(target)
    } else {
        normalize(&ctx.working_dir.join(target))
    };

    if contains_path(&ctx.working_dir, &full) {
        return Ok(());
    }

    let dir = match kind {
        ExternalKind::Directory => full.clone(),
        ExternalKind::File => full.parent().map(Path::to_path_buf).unwrap_or(full),
    };

    let glob = format!("{}/*", dir.to_string_lossy().replace('\\', "/"));
    ctx.check_permission("external_directory", &glob).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_path_matches_workspace_and_descendants() {
        let base = Path::new("/home/user/proj");
        assert!(contains_path(base, Path::new("/home/user/proj")));
        assert!(contains_path(base, Path::new("/home/user/proj/src/main.rs")));
        assert!(contains_path(base, Path::new("/home/user/proj/./src/../lib.rs")));
        assert!(!contains_path(base, Path::new("/home/user/proj2/x")));
        assert!(!contains_path(base, Path::new("/home/user")));
        assert!(!contains_path(base, Path::new("/home/user/proj/../escape")));
    }
}
