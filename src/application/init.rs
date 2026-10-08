use std::path::{Path, PathBuf};

use serde_json::json;
use thiserror::Error;

use super::ports::CleanupWarning;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitResult {
    pub config_path: PathBuf,
    pub skills_dir: PathBuf,
    pub agent_mapping_path: Option<PathBuf>,
    pub warnings: Vec<CleanupWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitAgentsResult {
    pub agent_mapping_path: PathBuf,
    pub warnings: Vec<CleanupWarning>,
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

pub trait InitStore {
    fn initialize(
        &self,
        config_path: &Path,
        skills_dir: &Path,
        config: &str,
        agent_mapping_path: Option<&Path>,
    ) -> Result<InitResult, InitError>;
    fn refresh_agents(&self, path: &Path) -> Result<InitAgentsResult, InitError>;
}

pub fn init_project(
    root: impl AsRef<Path>,
    store: &impl InitStore,
) -> Result<InitResult, InitError> {
    let root = root.as_ref();
    store.initialize(
        &root.join("sksync.config.json"),
        &root.join(".sksync/skills"),
        &project_config(),
        None,
    )
}

pub fn init_global(
    config_root: impl AsRef<Path>,
    store: &impl InitStore,
) -> Result<InitResult, InitError> {
    let config_root = config_root.as_ref();
    let skills_dir = config_root.join("skills");
    store.initialize(
        &config_root.join("config.json"),
        &skills_dir,
        &global_config(),
        Some(&config_root.join("agents.json")),
    )
}

pub fn init_agents(
    config_root: impl AsRef<Path>,
    store: &impl InitStore,
) -> Result<InitAgentsResult, InitError> {
    store.refresh_agents(&config_root.as_ref().join("agents.json"))
}

fn project_config() -> String {
    config_with_skill_dir("./.sksync/skills")
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
    use super::*;
    use std::cell::RefCell;

    type Initialization = (PathBuf, PathBuf, String, Option<PathBuf>);

    #[derive(Default)]
    struct RecordingInitStore {
        initialized: RefCell<Option<Initialization>>,
        refreshed: RefCell<Option<PathBuf>>,
    }

    impl InitStore for RecordingInitStore {
        fn initialize(
            &self,
            config_path: &Path,
            skills_dir: &Path,
            config: &str,
            agent_mapping_path: Option<&Path>,
        ) -> Result<InitResult, InitError> {
            self.initialized.replace(Some((
                config_path.into(),
                skills_dir.into(),
                config.into(),
                agent_mapping_path.map(Path::to_path_buf),
            )));
            Ok(InitResult {
                config_path: config_path.into(),
                skills_dir: skills_dir.into(),
                agent_mapping_path: agent_mapping_path.map(Path::to_path_buf),
                warnings: vec![CleanupWarning {
                    path: config_path.into(),
                    message: "retained temporary name".into(),
                }],
            })
        }
        fn refresh_agents(&self, path: &Path) -> Result<InitAgentsResult, InitError> {
            self.refreshed.replace(Some(path.into()));
            Ok(InitAgentsResult {
                agent_mapping_path: path.into(),
                warnings: Vec::new(),
            })
        }
    }

    #[test]
    fn init_coordinators_pass_logical_paths_config_and_warnings() {
        let root = tempfile::tempdir().unwrap();
        let store = RecordingInitStore::default();
        let result = init_project(root.path(), &store).unwrap();
        assert_eq!(result.warnings.len(), 1);
        let (config_path, skills_dir, config, mapping) = store.initialized.take().unwrap();
        assert_eq!(config_path, root.path().join("sksync.config.json"));
        assert_eq!(skills_dir, root.path().join(".sksync/skills"));
        assert_eq!(mapping, None);
        let config: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(config["skillDir"], "./.sksync/skills");
        assert_eq!(config["dependencies"], json!({}));
        init_global(root.path(), &store).unwrap();
        let (config_path, skills_dir, config, mapping) = store.initialized.take().unwrap();
        assert_eq!(config_path, root.path().join("config.json"));
        assert_eq!(skills_dir, root.path().join("skills"));
        assert_eq!(mapping, Some(root.path().join("agents.json")));
        let config: serde_json::Value = serde_json::from_str(&config).unwrap();
        assert_eq!(config["skillDir"], "~/.sksync/skills");
        init_agents(root.path(), &store).unwrap();
        assert_eq!(
            store.refreshed.take(),
            Some(root.path().join("agents.json"))
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
