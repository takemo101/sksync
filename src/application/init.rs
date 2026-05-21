use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitResult {
    pub config_path: PathBuf,
    pub skills_dir: PathBuf,
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
        default_config().to_owned(),
    )
}

pub fn init_global(config_root: impl AsRef<Path>) -> Result<InitResult, InitError> {
    let config_root = config_root.as_ref();
    let skills_dir = config_root.join("skills");
    init_with_config(
        config_root.join("config.json"),
        skills_dir.clone(),
        global_config(&skills_dir),
    )
}

fn init_with_config(
    config_path: PathBuf,
    skills_dir: PathBuf,
    config: String,
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

    Ok(InitResult {
        config_path,
        skills_dir,
    })
}

fn default_config() -> &'static str {
    include_str!("../../sksync.config.example.json")
}

fn global_config(skills_dir: &Path) -> String {
    let config = json!({
        "$schema": "https://example.com/sksync.schema.json",
        "skillDir": skills_dir,
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
        let config = std::fs::read_to_string(result.config_path).expect("read config");
        assert!(config.contains("\"skillDir\": \"./.sksync/skills\""));
    }

    #[test]
    fn init_global_creates_config_and_skills_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");

        let result = init_global(temp_dir.path()).expect("init global succeeds");

        assert_eq!(result.config_path, temp_dir.path().join("config.json"));
        assert_eq!(result.skills_dir, temp_dir.path().join("skills"));
        assert!(result.config_path.is_file());
        assert!(result.skills_dir.is_dir());
        let config = std::fs::read_to_string(result.config_path).expect("read config");
        assert!(config.contains("\"skillDir\""));
        assert!(config.contains("skills"));
        assert!(config.contains("\"dependencies\": {}"));
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
