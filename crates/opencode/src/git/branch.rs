use std::path::Path;
use std::process::Command;

pub fn current_branch(repo_path: &Path) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(branch)
    } else {
        Ok("unknown".to_string())
    }
}

pub fn default_branch(repo_path: &Path) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(repo_path)
        .output()?;
    
    if output.status.success() {
        let ref_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Some(name) = ref_name.strip_prefix("refs/remotes/origin/") {
            return Ok(name.to_string());
        }
    }
    
    Ok("main".to_string())
}