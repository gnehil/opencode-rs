use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

const SKILL_FILE_NAME: &str = "SKILL.md";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: Option<String>,
    pub location: PathBuf,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SkillFrontmatter {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

pub struct SkillService {
    skills: Arc<RwLock<HashMap<String, SkillInfo>>>,
    skill_dirs: Arc<RwLock<Vec<PathBuf>>>,
}

impl SkillService {
    pub fn new() -> Self {
        Self {
            skills: Arc::new(RwLock::new(HashMap::new())),
            skill_dirs: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn discover(&self, project_path: &PathBuf) -> anyhow::Result<()> {
        let mut skills = HashMap::new();

        self.add_builtin_skills(&mut skills);

        let global_skill_dirs = self.get_global_skill_dirs();
        for dir in global_skill_dirs {
            self.scan_skill_dir(&dir, &mut skills).await?;
        }

        self.scan_skill_dir(&project_path.join(".opencode"), &mut skills)
            .await?;

        if let Some(config) = self.load_project_config(project_path) {
            if let Some(paths) = config.skills.paths {
                for path in paths {
                    let skill_path = if path.is_absolute() {
                        path.clone()
                    } else {
                        project_path.join(path)
                    };
                    self.scan_skill_dir(&skill_path, &mut skills).await?;
                }
            }
        }

        let mut skills_lock = self.skills.write().await;
        skills_lock.extend(skills);

        Ok(())
    }

    fn add_builtin_skills(&self, skills: &mut HashMap<String, SkillInfo>) {
        let customize_skill = SkillInfo {
            name: "customize-opencode".to_string(),
            description: Some(
                "Learn how to customize opencode configuration for your project".to_string(),
            ),
            location: PathBuf::from("builtin://customize-opencode"),
            content: include_str!("customize-opencode.md").to_string(),
        };
        skills.insert(customize_skill.name.clone(), customize_skill);
    }

    fn get_global_skill_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        if let Some(home) = dirs::home_dir() {
            let claude_skills = home.join(".claude").join("skills");
            if claude_skills.exists() {
                dirs.push(claude_skills);
            }

            let agents_skills = home.join(".agents").join("skills");
            if agents_skills.exists() {
                dirs.push(agents_skills);
            }
        }

        dirs
    }

    async fn scan_skill_dir(
        &self,
        dir: &PathBuf,
        skills: &mut HashMap<String, SkillInfo>,
    ) -> anyhow::Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        let mut skill_dirs = self.skill_dirs.write().await;
        skill_dirs.push(dir.clone());

        let pattern = dir.join("**").join(SKILL_FILE_NAME);
        let pattern_str = pattern.to_string_lossy();

        let walker = walkdir::WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok());

        for entry in walker {
            let path = entry.path();
            if path
                .file_name()
                .map(|n| n == SKILL_FILE_NAME)
                .unwrap_or(false)
            {
                if let Some(skill) = self.parse_skill_file(&path.to_path_buf()) {
                    skills.insert(skill.name.clone(), skill);
                }
            }
        }

        Ok(())
    }

    fn parse_skill_file(&self, path: &PathBuf) -> Option<SkillInfo> {
        let content = std::fs::read_to_string(path).ok()?;

        let (frontmatter, body) = self.extract_frontmatter(&content)?;

        Some(SkillInfo {
            name: frontmatter.name,
            description: frontmatter.description,
            location: path.clone(),
            content: body,
        })
    }

    fn extract_frontmatter(&self, content: &str) -> Option<(SkillFrontmatter, String)> {
        let content = content.trim();

        if !content.starts_with("---") {
            return None;
        }

        let end_marker_idx = content[3..].find("---")?;
        let frontmatter_str = &content[3..end_marker_idx + 3].trim();
        let body = content[end_marker_idx + 6..].trim().to_string();

        let frontmatter: SkillFrontmatter = serde_yaml::from_str(frontmatter_str).ok()?;

        Some((frontmatter, body))
    }

    fn load_project_config(&self, project_path: &PathBuf) -> Option<ProjectConfig> {
        let config_path = project_path.join("opencode.json");
        if !config_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&config_path).ok()?;
        serde_json::from_str(&content).ok()
    }

    pub async fn get(&self, name: &str) -> Option<SkillInfo> {
        self.skills.read().await.get(name).cloned()
    }

    pub async fn all(&self) -> Vec<SkillInfo> {
        self.skills.read().await.values().cloned().collect()
    }

    pub async fn dirs(&self) -> Vec<PathBuf> {
        self.skill_dirs.read().await.clone()
    }

    pub fn format_skills_list(&self, skills: &[SkillInfo], verbose: bool) -> String {
        if verbose {
            let mut output = String::from("<available_skills>\n");
            for skill in skills {
                output.push_str(&format!(
                    "<skill>\n<name>{}</name>\n<description>{}</description>\n<location>{}</location>\n</skill>\n",
                    skill.name,
                    skill.description.as_deref().unwrap_or(""),
                    skill.location.display()
                ));
            }
            output.push_str("</available_skills>");
            output
        } else {
            skills
                .iter()
                .map(|s| format!("**{}**: {}", s.name, s.description.as_deref().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

impl Default for SkillService {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct ProjectConfig {
    #[serde(default)]
    skills: SkillsConfig,
}

#[derive(Debug, Deserialize, Default)]
struct SkillsConfig {
    #[serde(default)]
    paths: Option<Vec<PathBuf>>,
    #[serde(default)]
    urls: Option<Vec<String>>,
}
