//! Registry mapping source-file language → LSP server invocation.
//!
//! We don't ship a hard-coded server binary; instead we describe what
//! to spawn and let the surrounding process discover the binary on
//! PATH. If the binary is missing, the client surfaces an error and
//! falls back to the legacy shell-out diagnostic path.
//!
//! Adding a new language is: append a row to `BUILTIN`. The first
//! entry whose `extensions` slice contains the file's extension wins.

use std::path::Path;

#[derive(Debug, Clone)]
pub struct ServerSpec {
    /// Identifier passed to the LSP server as `languageId` (e.g.
    /// `"rust"`, `"typescript"`). Different from the file extension.
    pub language_id: &'static str,
    /// File extensions this server claims (lowercase, no leading dot).
    pub extensions: &'static [&'static str],
    /// Process to spawn. The first element is the program; the rest
    /// are arguments. We don't pre-shell-split because the user may
    /// have a server with args containing spaces.
    pub command: &'static [&'static str],
}

/// Built-in language → server mapping. Order matters: the first
/// matching entry wins. Servers are spawned lazily, so listing one
/// the user doesn't have installed costs nothing until they touch a
/// file in that language.
pub const BUILTIN: &[ServerSpec] = &[
    ServerSpec {
        language_id: "rust",
        extensions: &["rs"],
        // rust-analyzer reads its config from rustfmt.toml / Cargo.toml,
        // so no extra args needed for diagnostics.
        command: &["rust-analyzer"],
    },
    ServerSpec {
        language_id: "typescript",
        extensions: &["ts", "tsx"],
        // typescript-language-server is the de facto standalone server.
        command: &["typescript-language-server", "--stdio"],
    },
    ServerSpec {
        language_id: "javascript",
        extensions: &["js", "jsx", "mjs", "cjs"],
        command: &["typescript-language-server", "--stdio"],
    },
    ServerSpec {
        language_id: "python",
        extensions: &["py"],
        command: &["pyright-langserver", "--stdio"],
    },
    ServerSpec {
        language_id: "go",
        extensions: &["go"],
        command: &["gopls"],
    },
];

/// Find the server spec for a file path, by extension lookup.
pub fn for_path(path: &Path) -> Option<&'static ServerSpec> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())?;
    BUILTIN
        .iter()
        .find(|s| s.extensions.iter().any(|e| *e == ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_files_map_to_rust_analyzer() {
        let spec = for_path(Path::new("src/main.rs")).unwrap();
        assert_eq!(spec.language_id, "rust");
        assert_eq!(spec.command[0], "rust-analyzer");
    }

    #[test]
    fn tsx_uses_typescript_server() {
        let spec = for_path(Path::new("App.tsx")).unwrap();
        assert_eq!(spec.language_id, "typescript");
        assert_eq!(spec.command[0], "typescript-language-server");
    }

    #[test]
    fn case_insensitive_extension_match() {
        let spec = for_path(Path::new("Foo.RS")).unwrap();
        assert_eq!(spec.language_id, "rust");
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert!(for_path(Path::new("file.lol")).is_none());
        assert!(for_path(Path::new("no_extension")).is_none());
    }
}
