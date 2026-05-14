//! On-demand npm-package plugin installation.
//!
//! opencode plugins published to npm are installed into a per-package cache
//! directory and their entrypoint resolved, mirroring the TypeScript
//! `Npm.add` + `createPluginEntry` flow. Installation shells out to a package
//! manager (npm or bun) rather than embedding npm internals.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::spec::parse_plugin_specifier;

const INDEX_FILES: &[&str] = &["index.js", "index.mjs", "index.cjs", "index.ts", "index.tsx"];

/// Turn a plugin spec into a single flat directory name safe on every
/// platform. Unlike the TypeScript `sanitize` (which only rewrites Windows
/// reserved characters and leaves `/` to nest scoped packages), this keeps
/// the cache layout flat so a scoped spec maps to one directory.
pub fn sanitize_package_dir(spec: &str) -> String {
    spec.chars()
        .map(|ch| {
            if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                || (ch as u32) < 0x20
            {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

/// Install an npm-package plugin on demand and return its `file://` entry.
///
/// The package is installed under the opencode cache directory; a spec whose
/// package is already present is reused without re-installing.
pub async fn install_npm_plugin(spec: &str) -> anyhow::Result<String> {
    install_npm_plugin_in(spec, crate::global::cache().join("packages")).await
}

async fn install_npm_plugin_in(spec: &str, packages_root: PathBuf) -> anyhow::Result<String> {
    let name = parse_plugin_specifier(spec).pkg;
    let dir = packages_root.join(sanitize_package_dir(spec));
    let pkg_dir = dir.join("node_modules").join(&name);

    if !pkg_dir.exists() {
        std::fs::create_dir_all(&dir)?;
        run_install(spec, &dir).await?;
    }
    if !pkg_dir.exists() {
        anyhow::bail!(
            "npm install of '{spec}' did not produce {}",
            pkg_dir.display()
        );
    }
    resolve_npm_entry(&pkg_dir)
}

/// Shell out to a package manager to install `spec` into `dir`. Prefers npm,
/// falling back to bun. Install scripts are disabled for safety.
async fn run_install(spec: &str, dir: &Path) -> anyhow::Result<()> {
    use tokio::process::Command;

    if which::which("npm").is_ok() {
        let status = Command::new("npm")
            .args([
                "install",
                spec,
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
                "--silent",
            ])
            .arg("--prefix")
            .arg(dir)
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("npm install of '{spec}' exited with {status}");
        }
        return Ok(());
    }

    if which::which("bun").is_ok() {
        let status = Command::new("bun")
            .arg("add")
            .arg(spec)
            .arg("--cwd")
            .arg(dir)
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("bun add of '{spec}' exited with {status}");
        }
        return Ok(());
    }

    anyhow::bail!("no package manager (npm or bun) found to install '{spec}'")
}

/// Resolve the importable entry of an installed package directory: prefer the
/// plugin `exports["./server"]` entry, then `main`, then a directory index
/// file. Mirrors the TypeScript `resolvePackageEntrypoint`.
fn resolve_npm_entry(pkg_dir: &Path) -> anyhow::Result<String> {
    let manifest_path = pkg_dir.join("package.json");
    let manifest: Value = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("invalid {}: {e}", manifest_path.display()))?,
        Err(_) => Value::Null,
    };

    if let Some(raw) = manifest
        .get("exports")
        .and_then(|exports| exports.get("./server"))
        .and_then(extract_export_value)
    {
        return resolve_within(pkg_dir, &raw);
    }

    if let Some(main) = manifest
        .get("main")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        return resolve_within(pkg_dir, main);
    }

    for name in INDEX_FILES {
        let candidate = pkg_dir.join(name);
        if candidate.exists() {
            return Ok(path_to_file_url(&candidate));
        }
    }

    anyhow::bail!(
        "package {} has no server export, main, or index file",
        pkg_dir.display()
    )
}

/// Extract a path string from an `exports` entry, which may be a bare string
/// or a conditions object with `import`/`default`.
fn extract_export_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    for key in ["import", "default"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

/// Resolve `raw` relative to `pkg_dir` and reject anything that escapes the
/// package directory.
fn resolve_within(pkg_dir: &Path, raw: &str) -> anyhow::Result<String> {
    let joined = pkg_dir.join(raw);
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    if !normalized.starts_with(pkg_dir) {
        anyhow::bail!(
            "package entry '{raw}' resolves outside {}",
            pkg_dir.display()
        );
    }
    Ok(path_to_file_url(&normalized))
}

fn path_to_file_url(path: &Path) -> String {
    format!("file://{}", path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_scoped_specs_to_a_flat_dir_name() {
        assert_eq!(
            sanitize_package_dir("@opencode-ai/foo@1.2.3"),
            "@opencode-ai_foo@1.2.3"
        );
        assert_eq!(sanitize_package_dir("opencode-foo"), "opencode-foo");
    }

    fn write_pkg(dir: &Path, manifest: &str) {
        std::fs::write(dir.join("package.json"), manifest).unwrap();
    }

    #[test]
    fn resolves_server_export_string() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(
            dir.path(),
            r#"{ "exports": { "./server": "./dist/server.js" } }"#,
        );
        std::fs::create_dir_all(dir.path().join("dist")).unwrap();
        std::fs::write(dir.path().join("dist/server.js"), "").unwrap();

        let entry = resolve_npm_entry(dir.path()).unwrap();
        assert_eq!(
            entry,
            format!("file://{}", dir.path().join("dist/server.js").to_string_lossy())
        );
    }

    #[test]
    fn resolves_server_export_conditions_object() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(
            dir.path(),
            r#"{ "exports": { "./server": { "import": "./server.mjs" } } }"#,
        );
        let entry = resolve_npm_entry(dir.path()).unwrap();
        assert_eq!(
            entry,
            format!("file://{}", dir.path().join("server.mjs").to_string_lossy())
        );
    }

    #[test]
    fn falls_back_to_main_then_index() {
        let main_dir = tempfile::tempdir().unwrap();
        write_pkg(main_dir.path(), r#"{ "main": "lib/entry.js" }"#);
        assert_eq!(
            resolve_npm_entry(main_dir.path()).unwrap(),
            format!(
                "file://{}",
                main_dir.path().join("lib/entry.js").to_string_lossy()
            )
        );

        let index_dir = tempfile::tempdir().unwrap();
        write_pkg(index_dir.path(), "{}");
        std::fs::write(index_dir.path().join("index.mjs"), "").unwrap();
        assert_eq!(
            resolve_npm_entry(index_dir.path()).unwrap(),
            format!(
                "file://{}",
                index_dir.path().join("index.mjs").to_string_lossy()
            )
        );
    }

    #[test]
    fn rejects_entry_escaping_the_package_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(dir.path(), r#"{ "main": "../../../etc/passwd" }"#);
        assert!(resolve_npm_entry(dir.path()).is_err());
    }

    #[test]
    fn errors_when_no_entry_can_be_found() {
        let dir = tempfile::tempdir().unwrap();
        write_pkg(dir.path(), "{}");
        assert!(resolve_npm_entry(dir.path()).is_err());
    }
}
