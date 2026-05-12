use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct StatusItem {
    pub file: String,
    pub code: String,
    pub kind: StatusKind,
}

#[derive(Debug, Clone, Copy)]
pub enum StatusKind {
    Added,
    Deleted,
    Modified,
}

pub fn git_status(repo_path: &Path) -> anyhow::Result<Vec<StatusItem>> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "-z"])
        .current_dir(repo_path)
        .output()?;
    
    if !output.status.success() {
        return Err(anyhow::anyhow!("git status failed"));
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<StatusItem> = stdout.split('\0')
        .filter(|s| !s.is_empty())
        .map(|entry| {
            let code = entry.chars().take(2).collect::<String>();
            let file = entry.chars().skip(3).collect::<String>();
            
            let kind = if code.contains('?') {
                StatusKind::Added
            } else if code.contains('D') {
                StatusKind::Deleted
            } else {
                StatusKind::Modified
            };
            
            StatusItem {
                file,
                code,
                kind,
            }
        })
        .collect();
    
    Ok(items)
}