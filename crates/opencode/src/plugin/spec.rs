//! External plugin spec parsing.
//!
//! Mirrors the TypeScript `config/plugin.ts` + `plugin/shared.ts` parsing
//! helpers: classify a configured plugin spec as a local file or an npm
//! package, split an npm spec into package name + version, and resolve a
//! path-like spec into a concrete `file://` target on disk.

use std::path::{Path, PathBuf};

/// Where a plugin is loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSource {
    /// A local file or directory (`./plugin.ts`, an absolute path, `file://`).
    File,
    /// An npm package, installed on demand.
    Npm,
}

/// Old npm package names for plugins that are now built in; configured specs
/// referencing these are silently ignored.
pub const DEPRECATED_PLUGIN_PACKAGES: &[&str] =
    &["opencode-openai-codex-auth", "opencode-copilot-auth"];

/// An npm spec split into package name and version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSpecifier {
    pub pkg: String,
    pub version: String,
}

/// True when `spec` points at a Windows drive-letter absolute path.
fn is_windows_abs(spec: &str) -> bool {
    let bytes = spec.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
}

/// True when `spec` is a path-like spec rather than an npm package name.
pub fn is_path_plugin_spec(spec: &str) -> bool {
    spec.starts_with("file://")
        || spec.starts_with('.')
        || Path::new(spec).is_absolute()
        || is_windows_abs(spec)
}

/// Classify a spec as a local file or an npm package.
pub fn plugin_source(spec: &str) -> PluginSource {
    if is_path_plugin_spec(spec) {
        PluginSource::File
    } else {
        PluginSource::Npm
    }
}

/// True when `spec` references a plugin that is now built in.
pub fn is_deprecated_plugin(spec: &str) -> bool {
    DEPRECATED_PLUGIN_PACKAGES
        .iter()
        .any(|pkg| spec.contains(pkg))
}

/// Split an npm spec into package name and version. A spec with no explicit
/// `@version` resolves to `latest`. Path-like specs are returned verbatim
/// with an empty version, matching the TypeScript `parsePluginSpecifier`.
pub fn parse_plugin_specifier(spec: &str) -> PackageSpecifier {
    if is_path_plugin_spec(spec) {
        return PackageSpecifier {
            pkg: spec.to_string(),
            version: String::new(),
        };
    }

    // For a scoped package the version-separating `@` is the one that comes
    // after the scope's `/`; the leading `@` is part of the scope name.
    let version_at = if let Some(scoped) = spec.strip_prefix('@') {
        scoped
            .find('/')
            .and_then(|slash| spec[1 + slash + 1..].find('@').map(|at| 1 + slash + 1 + at))
    } else {
        spec.find('@')
    };

    match version_at {
        Some(at) => {
            let version = spec[at + 1..].trim();
            PackageSpecifier {
                pkg: spec[..at].to_string(),
                version: if version.is_empty() || version == "*" {
                    "latest".to_string()
                } else {
                    version.to_string()
                },
            }
        }
        None => PackageSpecifier {
            pkg: spec.to_string(),
            version: "latest".to_string(),
        },
    }
}

const INDEX_FILES: &[&str] = &["index.ts", "index.tsx", "index.js", "index.mjs", "index.cjs"];

/// Resolve a path-like spec into a concrete `file://` target.
///
/// A file spec resolves to itself (as a `file://` URL). A directory spec
/// resolves to the directory when it contains a `package.json`, otherwise to
/// the first index file found inside it. Mirrors the TypeScript
/// `resolvePathPluginTarget`.
pub fn resolve_path_plugin_target(spec: &str) -> anyhow::Result<String> {
    let raw = if let Some(rest) = spec.strip_prefix("file://") {
        rest.to_string()
    } else {
        spec.to_string()
    };

    let path = if Path::new(&raw).is_absolute() || is_windows_abs(&raw) {
        PathBuf::from(&raw)
    } else {
        std::env::current_dir()?.join(&raw)
    };

    let metadata = std::fs::metadata(&path).ok();
    let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);

    if !is_dir {
        // A file spec (existing or not) resolves to itself. A `file://` spec
        // is returned verbatim; a plain path becomes a `file://` URL.
        if spec.starts_with("file://") {
            return Ok(spec.to_string());
        }
        return Ok(path_to_file_url(&path));
    }

    if path.join("package.json").exists() {
        return Ok(path_to_file_url(&path));
    }

    for name in INDEX_FILES {
        let candidate = path.join(name);
        if candidate.exists() {
            return Ok(path_to_file_url(&candidate));
        }
    }

    anyhow::bail!(
        "Plugin directory {} is missing package.json or index file",
        path.display()
    )
}

fn path_to_file_url(path: &Path) -> String {
    format!("file://{}", path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_file_and_npm_specs() {
        assert_eq!(plugin_source("./plugin.ts"), PluginSource::File);
        assert_eq!(plugin_source("/abs/plugin.ts"), PluginSource::File);
        assert_eq!(plugin_source("file:///abs/plugin.ts"), PluginSource::File);
        assert_eq!(plugin_source("@opencode-ai/some-plugin"), PluginSource::Npm);
        assert_eq!(plugin_source("opencode-some-plugin"), PluginSource::Npm);
    }

    #[test]
    fn parses_unscoped_npm_specs() {
        assert_eq!(
            parse_plugin_specifier("opencode-foo"),
            PackageSpecifier {
                pkg: "opencode-foo".to_string(),
                version: "latest".to_string(),
            }
        );
        assert_eq!(
            parse_plugin_specifier("opencode-foo@1.2.3"),
            PackageSpecifier {
                pkg: "opencode-foo".to_string(),
                version: "1.2.3".to_string(),
            }
        );
    }

    #[test]
    fn parses_scoped_npm_specs() {
        assert_eq!(
            parse_plugin_specifier("@opencode-ai/foo"),
            PackageSpecifier {
                pkg: "@opencode-ai/foo".to_string(),
                version: "latest".to_string(),
            }
        );
        assert_eq!(
            parse_plugin_specifier("@opencode-ai/foo@2.0.0"),
            PackageSpecifier {
                pkg: "@opencode-ai/foo".to_string(),
                version: "2.0.0".to_string(),
            }
        );
    }

    #[test]
    fn parses_path_specs_verbatim() {
        assert_eq!(
            parse_plugin_specifier("./plugin.ts"),
            PackageSpecifier {
                pkg: "./plugin.ts".to_string(),
                version: String::new(),
            }
        );
    }

    #[test]
    fn detects_deprecated_plugins() {
        assert!(is_deprecated_plugin("opencode-openai-codex-auth"));
        assert!(is_deprecated_plugin("opencode-copilot-auth@1.0.0"));
        assert!(!is_deprecated_plugin("@opencode-ai/some-plugin"));
    }

    #[test]
    fn resolves_directory_with_package_json_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        let resolved = resolve_path_plugin_target(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(resolved, format!("file://{}", dir.path().to_string_lossy()));
    }

    #[test]
    fn resolves_directory_without_package_json_to_index_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.ts"), "").unwrap();
        let resolved = resolve_path_plugin_target(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(
            resolved,
            format!("file://{}", dir.path().join("index.ts").to_string_lossy())
        );
    }

    #[test]
    fn directory_missing_entrypoint_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_path_plugin_target(dir.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn resolves_file_spec_to_file_url() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plugin.ts");
        std::fs::write(&file, "").unwrap();
        let resolved = resolve_path_plugin_target(file.to_str().unwrap()).unwrap();
        assert_eq!(resolved, format!("file://{}", file.to_string_lossy()));
    }
}
