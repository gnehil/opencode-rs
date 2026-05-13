use glob_match;
use std::collections::HashSet;
use std::path::PathBuf;

pub struct IgnoreMatcher {
    patterns: Vec<IgnorePattern>,
    root: PathBuf,
}

#[derive(Debug, Clone)]
struct IgnorePattern {
    pattern: String,
    is_negation: bool,
    is_dir_only: bool,
}

impl IgnoreMatcher {
    pub fn new(root: PathBuf) -> Self {
        Self {
            patterns: Vec::new(),
            root,
        }
    }

    pub fn from_gitignore(root: PathBuf) -> anyhow::Result<Self> {
        let gitignore_path = root.join(".gitignore");
        let mut matcher = Self::new(root);

        if gitignore_path.exists() {
            let content = std::fs::read_to_string(&gitignore_path)?;
            matcher.parse_gitignore(&content);
        }

        matcher.add_default_ignores();
        Ok(matcher)
    }

    pub fn add_pattern(&mut self, pattern: &str) {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern.starts_with('#') {
            return;
        }

        let is_negation = pattern.starts_with('!');
        let actual_pattern = if is_negation { &pattern[1..] } else { pattern };
        let is_dir_only = actual_pattern.ends_with('/');

        self.patterns.push(IgnorePattern {
            pattern: actual_pattern.trim_end_matches('/').to_string(),
            is_negation,
            is_dir_only,
        });
    }

    fn parse_gitignore(&mut self, content: &str) {
        for line in content.lines() {
            self.add_pattern(line);
        }
    }

    fn add_default_ignores(&mut self) {
        let defaults = [
            ".git",
            ".gitignore",
            ".opencode",
            "node_modules",
            "target",
            "dist",
            "build",
            "*.lock",
            "*.log",
            ".env",
            ".env.local",
            ".DS_Store",
            "Thumbs.db",
            "*.swp",
            "*.swo",
            "*~",
        ];

        for pattern in defaults {
            self.add_pattern(pattern);
        }
    }

    pub fn is_ignored(&self, path: &PathBuf) -> bool {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        let relative_str = relative.to_string_lossy();
        let is_dir = path.is_dir();

        let mut ignored = false;

        for pattern in &self.patterns {
            if pattern.is_dir_only && !is_dir {
                continue;
            }

            let matches = self.pattern_matches(&pattern.pattern, &relative_str, is_dir);

            if pattern.is_negation {
                if matches {
                    ignored = false;
                }
            } else {
                if matches {
                    ignored = true;
                }
            }
        }

        ignored
    }

    fn pattern_matches(&self, pattern: &str, path: &str, is_dir: bool) -> bool {
        let pattern_with_dir = if is_dir && !pattern.ends_with('/') {
            format!("{}/**", pattern)
        } else if !pattern.contains('/') {
            format!("**/{}", pattern)
        } else {
            pattern.to_string()
        };

        glob_match::glob_match(&pattern_with_dir, path)
    }

    pub fn filter_files(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
        paths
            .iter()
            .filter(|p| !self.is_ignored(p))
            .cloned()
            .collect()
    }
}
