use std::env;
use std::path::PathBuf;

use directories::BaseDirs;
use once_cell::sync::Lazy;

pub const NAME: &str = "opencode";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn test_home() -> Option<String> {
    env::var("OPENCODE_TEST_HOME").ok()
}

fn config_dir_override() -> Option<String> {
    env::var("OPENCODE_CONFIG_DIR").ok()
}

fn state_dir() -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| BaseDirs::new().map(|bd| bd.home_dir().join(".local").join("state")))
    } else if cfg!(target_os = "macos") {
        BaseDirs::new().and_then(|bd| {
            bd.data_dir()
                .parent()
                .map(|p| p.join("State"))
                .or_else(|| Some(bd.data_dir().join("State")))
        })
    } else if cfg!(target_os = "windows") {
        BaseDirs::new().map(|bd| bd.data_dir().join("State"))
    } else {
        BaseDirs::new().map(|bd| bd.data_dir().join("state"))
    }
}

fn tmp_dir() -> PathBuf {
    env::temp_dir().join(NAME)
}

pub struct Paths {
    pub home: PathBuf,
    pub data: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub tmp: PathBuf,
    pub bin: PathBuf,
    pub log: PathBuf,
    pub repos: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        let base = BaseDirs::new().expect("could not determine home directory");

        let home = test_home()
            .map(PathBuf::from)
            .unwrap_or_else(|| base.home_dir().to_path_buf());

        let data = base.data_dir().join(NAME);
        let config = config_dir_override()
            .map(PathBuf::from)
            .unwrap_or_else(|| base.config_dir().join(NAME));
        let cache = base.cache_dir().join(NAME);
        let state = state_dir()
            .unwrap_or_else(|| base.data_dir().join("state"))
            .join(NAME);
        let tmp = tmp_dir();

        let bin = cache.join("bin");
        let log = data.join("log");
        let repos = data.join("repos");

        Self {
            home,
            data,
            config,
            cache,
            state,
            tmp,
            bin,
            log,
            repos,
        }
    }

    pub fn init(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data)?;
        std::fs::create_dir_all(&self.config)?;
        std::fs::create_dir_all(&self.cache)?;
        std::fs::create_dir_all(&self.state)?;
        std::fs::create_dir_all(&self.tmp)?;
        std::fs::create_dir_all(&self.log)?;
        std::fs::create_dir_all(&self.bin)?;
        std::fs::create_dir_all(&self.repos)?;
        Ok(())
    }
}

impl Default for Paths {
    fn default() -> Self {
        Self::new()
    }
}

pub static PATH: Lazy<Paths> = Lazy::new(Paths::new);

pub fn home() -> &'static PathBuf {
    &PATH.home
}

pub fn data() -> &'static PathBuf {
    &PATH.data
}

pub fn config() -> &'static PathBuf {
    &PATH.config
}

pub fn cache() -> &'static PathBuf {
    &PATH.cache
}

pub fn state() -> &'static PathBuf {
    &PATH.state
}

pub fn tmp() -> &'static PathBuf {
    &PATH.tmp
}

pub fn bin() -> &'static PathBuf {
    &PATH.bin
}

pub fn log() -> &'static PathBuf {
    &PATH.log
}

pub fn repos() -> &'static PathBuf {
    &PATH.repos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_opencode() {
        assert_eq!(NAME, "opencode");
    }

    #[test]
    fn version_is_not_empty() {
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn paths_contain_app_name() {
        let p = Paths::new();
        assert!(p.data.to_string_lossy().contains(NAME));
    }

    #[test]
    fn test_home_override() {
        std::env::set_var("OPENCODE_TEST_HOME", "/tmp/test-home");
        let p = Paths::new();
        assert_eq!(p.home, PathBuf::from("/tmp/test-home"));
        std::env::remove_var("OPENCODE_TEST_HOME");
    }

    #[test]
    fn config_dir_override() {
        std::env::set_var("OPENCODE_CONFIG_DIR", "/tmp/test-config");
        let p = Paths::new();
        assert_eq!(p.config, PathBuf::from("/tmp/test-config"));
        std::env::remove_var("OPENCODE_CONFIG_DIR");
    }
}
