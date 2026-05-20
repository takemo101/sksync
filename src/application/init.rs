use std::fs;
use std::path::{Path, PathBuf};

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
    let config_path = root.join("sksync.config.json");
    let skills_dir = root.join(".sksync/skills");

    if config_path.exists() {
        return Err(InitError::ConfigExists(config_path.display().to_string()));
    }

    fs::create_dir_all(&skills_dir).map_err(|source| InitError::CreateSkillsDir {
        path: skills_dir.display().to_string(),
        source,
    })?;
    fs::write(&config_path, default_config()).map_err(|source| InitError::WriteConfig {
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

#[cfg(test)]
mod tests {
    use super::{init_project, InitError};

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
    fn init_fails_when_config_exists() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp_dir.path().join("sksync.config.json"), "{}").expect("write config");

        let error = init_project(temp_dir.path()).expect_err("existing config fails");

        assert!(matches!(error, InitError::ConfigExists(_)));
    }
}
