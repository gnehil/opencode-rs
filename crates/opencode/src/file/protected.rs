use std::path::PathBuf;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ProtectedFiles {
    protected: Arc<RwLock<HashSet<PathBuf>>>,
    patterns: Vec<String>,
}

impl ProtectedFiles {
    pub fn new() -> Self {
        let mut patterns = Vec::new();
        patterns.extend(Self::default_protected_patterns());
        
        Self {
            protected: Arc::new(RwLock::new(HashSet::new())),
            patterns,
        }
    }

    fn default_protected_patterns() -> Vec<String> {
        vec![
            ".env",
            ".env.local",
            ".env.*",
            "*.pem",
            "*.key",
            "*.secret",
            "credentials",
            "secrets",
            "*.credentials",
            ".git",
            ".ssh",
            "id_rsa",
            "id_ed25519",
            "*.pub",
            ".npmrc",
            ".yarnrc",
            "netrc",
            "_netrc",
            ".netrc",
            ".pgpass",
            ".htpasswd",
            "htpasswd",
            "*.asc",
            "*.gpg",
            "gnupg",
            ".gnupg",
        ]
    }

    pub fn add_protected(&self, path: PathBuf) {
        self.protected.write().await.insert(path);
    }

    pub fn remove_protected(&self, path: &PathBuf) {
        self.protected.write().await.remove(path);
    }

    pub fn add_pattern(&mut self, pattern: String) {
        self.patterns.push(pattern);
    }

    pub fn is_protected(&self, path: &PathBuf) -> bool {
        let protected = self.protected.read().await;
        if protected.contains(path) {
            return true;
        }

        let path_str = path.to_string_lossy();
        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        for pattern in &self.patterns {
            if glob_match::glob_match(pattern, filename) || glob_match::glob_match(pattern, &path_str) {
                return true;
            }
        }

        false
    }

    pub fn check_write_allowed(&self, path: &PathBuf) -> anyhow::Result<()> {
        if self.is_protected(path) {
            anyhow::bail!("File {:?} is protected and cannot be modified", path);
        }
        Ok(())
    }

    pub fn check_read_allowed(&self, path: &PathBuf) -> anyhow::Result<()> {
        let protected = self.protected.read().await;
        Ok(())
    }
}

impl Default for ProtectedFiles {
    fn default() -> Self {
        Self::new()
    }
}