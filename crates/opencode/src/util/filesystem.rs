use std::path::{Path, PathBuf};

use thiserror::Error;
use glob_match::glob_match;
use tokio::fs::{self, DirEntry};
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum FsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Path is not a directory: {}", path.display())]
    NotADir { path: PathBuf },
    #[error("Glob pattern error: {0}")]
    GlobPattern(String),
}

pub type FsResult<T> = Result<T, FsError>;

pub async fn exists(path: impl AsRef<Path>) -> bool {
    fs::metadata(path).await.is_ok()
}

pub async fn read_file(path: impl AsRef<Path>) -> FsResult<String> {
    let content = fs::read_to_string(path).await?;
    Ok(content)
}

pub async fn read_file_bytes(path: impl AsRef<Path>) -> FsResult<Vec<u8>> {
    let content = fs::read(path).await?;
    Ok(content)
}

pub async fn write_file(path: impl AsRef<Path>, content: impl AsRef<str>) -> FsResult<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).await?;
        }
    }
    fs::write(path, content.as_ref()).await?;
    Ok(())
}

pub async fn write_file_bytes(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> FsResult<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).await?;
        }
    }
    fs::write(path, bytes).await?;
    Ok(())
}

pub async fn is_dir(path: impl AsRef<Path>) -> bool {
    match fs::metadata(path).await {
        Ok(meta) => meta.is_dir(),
        Err(_) => false,
    }
}

pub async fn is_file(path: impl AsRef<Path>) -> bool {
    match fs::metadata(path).await {
        Ok(meta) => meta.is_file(),
        Err(_) => false,
    }
}

pub async fn list_dir(path: impl AsRef<Path>) -> FsResult<Vec<DirEntry>> {
    let mut entries = Vec::new();
    let mut dir = fs::read_dir(path).await?;
    while let Some(entry) = dir.next_entry().await? {
        entries.push(entry);
    }
    Ok(entries)
}

pub async fn create_dir(path: impl AsRef<Path>) -> FsResult<()> {
    fs::create_dir_all(path).await?;
    Ok(())
}

pub async fn remove_file(path: impl AsRef<Path>) -> FsResult<()> {
    fs::remove_file(path).await?;
    Ok(())
}

pub async fn remove_dir(path: impl AsRef<Path>) -> FsResult<()> {
    fs::remove_dir_all(path).await?;
    Ok(())
}

pub async fn copy_file(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> FsResult<u64> {
    let bytes = fs::copy(src, dst).await?;
    Ok(bytes)
}

/// Recursively walks `path` and returns entries matching `pattern`.
pub async fn glob(pattern: &str, path: impl AsRef<Path>) -> FsResult<Vec<PathBuf>> {
    let base = path.as_ref().to_path_buf();
    let pattern = pattern.to_owned();

    let entries: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
        WalkDir::new(&base)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let entry_path = entry.path();
                let relative = entry_path.strip_prefix(&base).unwrap_or(entry_path);
                let rel_str = relative.to_string_lossy();
                if glob_match(&pattern, &rel_str) {
                    Some(entry_path.to_path_buf())
                } else {
                    let forward = rel_str.replace('\\', "/");
                    if glob_match(&pattern, &forward) {
                        Some(entry_path.to_path_buf())
                    } else {
                        entry_path
                            .file_name()
                            .filter(|name| glob_match(&pattern, &name.to_string_lossy()))
                            .map(|_| entry_path.to_path_buf())
                    }
                }
            })
            .collect()
    })
    .await
    .map_err(|join_err| FsError::GlobPattern(join_err.to_string()))?;

    Ok(entries)
}
