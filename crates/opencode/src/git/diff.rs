use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct DiffStat {
    pub file: String,
    pub additions: usize,
    pub deletions: usize,
}

pub fn git_diff(repo_path: &Path, ref_name: Option<&str>) -> anyhow::Result<Vec<DiffStat>> {
    let mut args = vec!["diff", "--numstat", "-z"];
    
    if let Some(ref_str) = ref_name {
        args.push(ref_str);
    }
    
    let output = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .output()?;
    
    if !output.status.success() {
        return Ok(Vec::new());
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stats: Vec<DiffStat> = stdout.split('\0')
        .filter(|s| !s.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 3 {
                let additions = parts[0].parse().unwrap_or(0);
                let deletions = parts[1].parse().unwrap_or(0);
                let file = parts[2].to_string();
                DiffStat { file, additions, deletions }
            } else {
                DiffStat { file: line.to_string(), additions: 0, deletions: 0 }
            }
        })
        .collect();
    
    Ok(stats)
}