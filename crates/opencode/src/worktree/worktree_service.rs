use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

const WORKTREE_PREFIX: &str = "wt";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub id: String,
    pub name: String,
    pub branch: String,
    pub directory: PathBuf,
    pub project_id: String,
    pub time_created: i64,
}

pub struct WorktreeService {
    worktrees: Arc<RwLock<Vec<WorktreeInfo>>>,
}

impl WorktreeService {
    pub fn new() -> Self {
        Self {
            worktrees: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn create(
        &self,
        project_path: &PathBuf,
        name: &str,
        branch: &str,
    ) -> Result<WorktreeInfo> {
        let id = format!("{}_{}", WORKTREE_PREFIX, ulid::Ulid::new().to_string());

        let worktree_dir = project_path.join(".opencode").join("worktrees").join(&id);

        std::fs::create_dir_all(&worktree_dir)?;

        let worktree_path = worktree_dir.join("workspace");

        let branch_arg = if branch.starts_with("refs/") {
            branch.to_string()
        } else {
            format!("refs/heads/{}", branch)
        };

        let output = std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                &branch,
                worktree_path.to_string_lossy().as_ref(),
            ])
            .current_dir(project_path)
            .output();

        match output {
            Ok(result) if result.status.success() => {
                let info = WorktreeInfo {
                    id,
                    name: name.to_string(),
                    branch: branch.to_string(),
                    directory: worktree_path,
                    project_id: self.get_project_id(project_path),
                    time_created: chrono::Utc::now().timestamp_millis(),
                };

                self.worktrees.write().await.push(info.clone());
                Ok(info)
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Err(anyhow::anyhow!("Git worktree add failed: {}", stderr))
            }
            Err(e) => Err(anyhow::anyhow!("Failed to run git: {}", e)),
        }
    }

    pub async fn list(&self, project_path: &PathBuf) -> Vec<WorktreeInfo> {
        let worktrees = self.worktrees.read().await;
        let project_id = self.get_project_id(project_path);
        worktrees
            .iter()
            .filter(|w| w.project_id == project_id)
            .cloned()
            .collect()
    }

    pub async fn get(&self, id: &str) -> Option<WorktreeInfo> {
        self.worktrees
            .read()
            .await
            .iter()
            .find(|w| w.id == id)
            .cloned()
    }

    pub async fn remove(&self, id: &str, project_path: &PathBuf) -> Result<Option<WorktreeInfo>> {
        let worktrees = self.worktrees.read().await;
        let worktree = worktrees.iter().find(|w| w.id == id).cloned();

        if let Some(wt) = worktree {
            let output = std::process::Command::new("git")
                .args([
                    "worktree",
                    "remove",
                    "--force",
                    wt.directory.to_string_lossy().as_ref(),
                ])
                .current_dir(project_path)
                .output();

            match output {
                Ok(result) if result.status.success() => {
                    self.worktrees.write().await.retain(|w| w.id != id);
                    Ok(Some(wt))
                }
                Ok(result) => {
                    let stderr = String::from_utf8_lossy(&result.stderr);
                    Err(anyhow::anyhow!("Git worktree remove failed: {}", stderr))
                }
                Err(e) => Err(anyhow::anyhow!("Failed to run git: {}", e)),
            }
        } else {
            Ok(None)
        }
    }

    pub async fn prune(&self, project_path: &PathBuf) -> Result<()> {
        let output = std::process::Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(project_path)
            .output();

        match output {
            Ok(result) if result.status.success() => Ok(()),
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                Err(anyhow::anyhow!("Git worktree prune failed: {}", stderr))
            }
            Err(e) => Err(anyhow::anyhow!("Failed to run git: {}", e)),
        }
    }

    fn get_project_id(&self, project_path: &PathBuf) -> String {
        project_path.to_string_lossy().to_string()
    }

    pub async fn discover_from_git(&self, project_path: &PathBuf) -> Result<Vec<WorktreeInfo>> {
        let output = std::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(project_path)
            .output();

        match output {
            Ok(result) if result.status.success() => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                self.parse_worktree_list(&stdout, project_path)
            }
            Ok(_) => Ok(Vec::new()),
            Err(_) => Ok(Vec::new()),
        }
    }

    fn parse_worktree_list(
        &self,
        output: &str,
        project_path: &PathBuf,
    ) -> Result<Vec<WorktreeInfo>> {
        let mut worktrees = Vec::new();
        let mut current_dir: Option<PathBuf> = None;
        let mut current_branch: Option<String> = None;

        for line in output.lines() {
            if line.starts_with("worktree ") {
                current_dir = Some(PathBuf::from(line[9..].trim()));
            } else if line.starts_with("branch ") {
                current_branch = Some(line[7..].trim().to_string());
            } else if line.is_empty() {
                if let (Some(dir), Some(branch)) = (current_dir.take(), current_branch.take()) {
                    let name = dir
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");

                    let branch_name = branch.strip_prefix("refs/heads/").unwrap_or(&branch);

                    worktrees.push(WorktreeInfo {
                        id: format!("{}_{}", WORKTREE_PREFIX, name),
                        name: name.to_string(),
                        branch: branch_name.to_string(),
                        directory: dir,
                        project_id: self.get_project_id(project_path),
                        time_created: chrono::Utc::now().timestamp_millis(),
                    });
                }
            }
        }

        Ok(worktrees)
    }
}

impl Default for WorktreeService {
    fn default() -> Self {
        Self::new()
    }
}
