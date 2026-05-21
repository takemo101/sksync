use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitResult {
    pub config_path: PathBuf,
    pub skills_dir: PathBuf,
    pub agent_mapping_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitAgentsResult {
    pub agent_mapping_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("config already exists at {0}")]
    ConfigExists(String),
    #[error("failed to create skills directory {path}: {source}")]
    CreateSkillsDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write config {path}: {source}")]
    WriteConfig {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub fn init_project(root: impl AsRef<Path>) -> Result<InitResult, InitError> {
    let root = root.as_ref();
    init_with_config(
        root.join("sksync.config.json"),
        root.join(".sksync/skills"),
        project_config(),
        None,
    )
}

pub fn init_global(config_root: impl AsRef<Path>) -> Result<InitResult, InitError> {
    let config_root = config_root.as_ref();
    let skills_dir = config_root.join("skills");
    init_with_config(
        config_root.join("config.json"),
        skills_dir,
        global_config(),
        Some(config_root.join("agents.json")),
    )
}

pub fn init_agents(config_root: impl AsRef<Path>) -> Result<InitAgentsResult, InitError> {
    let path = config_root.as_ref().join("agents.json");
    write_agent_mapping(&path)?;
    Ok(InitAgentsResult {
        agent_mapping_path: path,
    })
}

fn init_with_config(
    config_path: PathBuf,
    skills_dir: PathBuf,
    config: String,
    agent_mapping_path: Option<PathBuf>,
) -> Result<InitResult, InitError> {
    if config_path.exists() {
        return Err(InitError::ConfigExists(config_path.display().to_string()));
    }

    fs::create_dir_all(&skills_dir).map_err(|source| InitError::CreateSkillsDir {
        path: skills_dir.display().to_string(),
        source,
    })?;
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|source| InitError::CreateSkillsDir {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(&config_path, config).map_err(|source| InitError::WriteConfig {
        path: config_path.display().to_string(),
        source,
    })?;

    let agent_mapping_path = match agent_mapping_path {
        Some(path) if write_agent_mapping_if_missing(&path)? => Some(path),
        _ => None,
    };

    Ok(InitResult {
        config_path,
        skills_dir,
        agent_mapping_path,
    })
}

fn write_agent_mapping_if_missing(path: &Path) -> Result<bool, InitError> {
    if path.exists() {
        return Ok(false);
    }
    write_agent_mapping(path)?;
    Ok(true)
}

fn write_agent_mapping(path: &Path) -> Result<(), InitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| InitError::CreateSkillsDir {
            path: parent.display().to_string(),
            source,
        })?;
    }
    fs::write(path, default_agent_mapping()).map_err(|source| InitError::WriteConfig {
        path: path.display().to_string(),
        source,
    })
}

fn project_config() -> String {
    config_with_skill_dir("./.sksync/skills")
}

fn default_agent_mapping() -> &'static str {
    include_str!("../../sksync.agents.example.json")
}

fn global_config() -> String {
    config_with_skill_dir("~/.sksync/skills")
}

fn config_with_skill_dir(skill_dir: &str) -> String {
    let config = json!({
        "$schema": "https://raw.githubusercontent.com/takemo101/sksync/main/schemas/sksync.schema.json",
        "skillDir": skill_dir,
        "dependencies": {}
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&config).expect("serialize global config")
    )
}

#[cfg(test)]
mod tests {
    use super::{init_global, init_project, InitError};

    #[test]
    fn init_creates_config_and_skills_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");

        let result = init_project(temp_dir.path()).expect("init succeeds");

        assert!(result.config_path.is_file());
        assert!(result.skills_dir.is_dir());
        assert_eq!(result.agent_mapping_path, None);
        let config = std::fs::read_to_string(result.config_path).expect("read config");
        assert!(config.contains("\"skillDir\": \"./.sksync/skills\""));
        assert!(config.contains("\"dependencies\": {}"));
        assert!(!config.contains("example-skill"));
        assert!(!config.contains("local-example"));
    }

    #[test]
    fn init_global_creates_config_agents_and_skills_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");

        let result = init_global(temp_dir.path()).expect("init global succeeds");

        let agent_mapping_path = temp_dir.path().join("agents.json");
        assert_eq!(result.config_path, temp_dir.path().join("config.json"));
        assert_eq!(result.skills_dir, temp_dir.path().join("skills"));
        assert_eq!(result.agent_mapping_path, Some(agent_mapping_path.clone()));
        assert!(result.config_path.is_file());
        assert!(result.skills_dir.is_dir());
        assert!(agent_mapping_path.is_file());
        let config = std::fs::read_to_string(result.config_path).expect("read config");
        assert!(config.contains("\"skillDir\": \"~/.sksync/skills\""));
        assert!(config.contains("\"dependencies\": {}"));
        let agents = std::fs::read_to_string(agent_mapping_path).expect("read agents");
        assert!(agents.contains("\"global\""));
        assert!(agents.contains("\"project\""));
        assert!(agents.contains("~/.pi/agent/skills"));
    }

    #[test]
    fn init_global_does_not_overwrite_existing_agent_mapping() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let agent_mapping_path = temp_dir.path().join("agents.json");
        std::fs::write(&agent_mapping_path, "custom").expect("write agents");

        let result = init_global(temp_dir.path()).expect("init global succeeds");

        assert_eq!(result.agent_mapping_path, None);
        assert_eq!(
            std::fs::read_to_string(agent_mapping_path).expect("read agents"),
            "custom"
        );
    }

    #[test]
    fn init_agents_overwrites_existing_agent_mapping_only() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let config_path = temp_dir.path().join("config.json");
        let agent_mapping_path = temp_dir.path().join("agents.json");
        std::fs::write(&config_path, "custom config").expect("write config");
        std::fs::write(&agent_mapping_path, "custom agents").expect("write agents");

        let result = super::init_agents(temp_dir.path()).expect("init agents succeeds");

        assert_eq!(result.agent_mapping_path, agent_mapping_path.clone());
        assert_eq!(
            std::fs::read_to_string(config_path).expect("read config"),
            "custom config"
        );
        let agents = std::fs::read_to_string(agent_mapping_path).expect("read agents");
        assert!(agents.contains("\"global\""));
        assert!(agents.contains("\"project\""));
        assert!(!agents.contains("custom agents"));
    }

    #[test]
    fn init_fails_when_config_exists() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp_dir.path().join("sksync.config.json"), "{}").expect("write config");

        let error = init_project(temp_dir.path()).expect_err("existing config fails");

        assert!(matches!(error, InitError::ConfigExists(_)));
    }

    #[test]
    fn init_global_fails_when_config_exists() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp_dir.path().join("config.json"), "{}").expect("write config");

        let error = init_global(temp_dir.path()).expect_err("existing config fails");

        assert!(matches!(error, InitError::ConfigExists(_)));
    }
}
