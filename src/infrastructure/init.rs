use std::fs;
use std::path::Path;

use super::atomic_file::{write_atomic, WriteMode};
use crate::application::init::{InitAgentsResult, InitError, InitResult, InitStore};

pub struct FileInitStore;

impl InitStore for FileInitStore {
    fn initialize(
        &self,
        config_path: &Path,
        skills_dir: &Path,
        config: &str,
        agent_mapping_path: Option<&Path>,
    ) -> Result<InitResult, InitError> {
        match fs::symlink_metadata(config_path) {
            Ok(_) => return Err(InitError::ConfigExists(config_path.display().to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(write_error(config_path, source)),
        }
        create_directory(skills_dir)?;
        if let Some(parent) = config_path.parent() {
            create_directory(parent)?;
        }
        let outcome = write_atomic(config_path, config.as_bytes(), WriteMode::CreateNew).map_err(
            |source| {
                if source.kind() == std::io::ErrorKind::AlreadyExists {
                    InitError::ConfigExists(config_path.display().to_string())
                } else {
                    write_error(config_path, source)
                }
            },
        )?;
        let mut warnings = outcome.warnings;
        let agent_mapping_path = match agent_mapping_path {
            Some(path) => {
                match fs::symlink_metadata(path) {
                    Ok(_) => None,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        if let Some(parent) = path.parent() {
                            create_directory(parent)?;
                        }
                        match write_atomic(
                            path,
                            default_agent_mapping().as_bytes(),
                            WriteMode::CreateNew,
                        ) {
                            Ok(outcome) => {
                                warnings.extend(outcome.warnings);
                                Some(path.to_path_buf())
                            }
                            // Another creator won publication; keep its mapping untouched.
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                            Err(source) => return Err(write_error(path, source)),
                        }
                    }
                    Err(source) => return Err(write_error(path, source)),
                }
            }
            None => None,
        };
        Ok(InitResult {
            config_path: config_path.to_path_buf(),
            skills_dir: skills_dir.to_path_buf(),
            agent_mapping_path,
            warnings,
        })
    }

    fn refresh_agents(&self, path: &Path) -> Result<InitAgentsResult, InitError> {
        if let Some(parent) = path.parent() {
            create_directory(parent)?;
        }
        let outcome = write_atomic(path, default_agent_mapping().as_bytes(), WriteMode::Replace)
            .map_err(|source| write_error(path, source))?;
        Ok(InitAgentsResult {
            agent_mapping_path: path.to_path_buf(),
            warnings: outcome.warnings,
        })
    }
}

fn create_directory(path: &Path) -> Result<(), InitError> {
    fs::create_dir_all(path).map_err(|source| InitError::CreateSkillsDir {
        path: path.display().to_string(),
        source,
    })
}

fn write_error(path: &Path, source: std::io::Error) -> InitError {
    InitError::WriteConfig {
        path: path.display().to_string(),
        source,
    }
}

fn default_agent_mapping() -> &'static str {
    include_str!("../../sksync.agents.example.json")
}

#[cfg(test)]
mod tests {
    use super::FileInitStore;
    use crate::application::init::{init_global, init_project, InitError};

    #[test]
    fn init_create_only_keeps_competing_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sksync.config.json");
        let result = crate::infrastructure::atomic_file::with_test_competing_create(
            path.clone(),
            b"competing config".to_vec(),
            || init_project(dir.path(), &FileInitStore),
        );
        assert!(matches!(result, Err(InitError::ConfigExists(_))));
        assert_eq!(std::fs::read(path).unwrap(), b"competing config");
    }

    #[test]
    fn init_create_only_keeps_competing_agent_mapping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let result = crate::infrastructure::atomic_file::with_test_competing_create(
            path.clone(),
            b"competing agents".to_vec(),
            || init_global(dir.path(), &FileInitStore),
        )
        .unwrap();
        assert_eq!(result.agent_mapping_path, None);
        assert_eq!(std::fs::read(path).unwrap(), b"competing agents");
        assert!(result.config_path.is_file());
    }

    #[test]
    fn init_create_only_returns_cleanup_warnings_after_commit() {
        let dir = tempfile::tempdir().unwrap();
        let result = crate::infrastructure::atomic_file::with_test_cleanup_failure(|| {
            init_global(dir.path(), &FileInitStore)
        })
        .unwrap();
        assert_eq!(result.warnings.len(), 2);
        assert!(result.config_path.is_file());
        assert!(result.agent_mapping_path.unwrap().is_file());
        for warning in result.warnings {
            assert!(warning.path.is_file());
            assert!(warning.message.contains("injected"));
        }
    }

    #[test]
    fn init_agents_refresh_preserves_symlink_permissions_and_old_inode() {
        use std::io::Read;
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("dotfiles-agents.json");
        let path = dir.path().join("agents.json");
        std::fs::write(&referent, b"custom agents").unwrap();
        std::fs::set_permissions(&referent, std::fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&referent, &path).unwrap();
        let mut previous = std::fs::File::open(&path).unwrap();
        let failed = crate::infrastructure::atomic_file::with_test_publication_failure(|| {
            crate::application::init::init_agents(dir.path(), &FileInitStore)
        });
        assert!(failed.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"custom agents");
        assert_eq!(std::fs::read_link(&path).unwrap(), referent);
        let result = crate::application::init::init_agents(dir.path(), &FileInitStore).unwrap();
        assert!(result.warnings.is_empty());
        assert_eq!(std::fs::read_link(&path).unwrap(), referent);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let mut bytes = Vec::new();
        previous.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"custom agents");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            super::default_agent_mapping()
        );
    }

    #[test]
    fn init_failed_publication_preserves_absence() {
        let dir = tempfile::tempdir().unwrap();
        let result = crate::infrastructure::atomic_file::with_test_publication_failure(|| {
            init_project(dir.path(), &FileInitStore)
        });
        assert!(result.is_err());
        assert!(!dir.path().join("sksync.config.json").exists());
    }

    #[test]
    fn init_agents_failed_publication_preserves_raw_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents.json");
        let original = b"custom agents raw bytes\n";
        std::fs::write(&path, original).unwrap();
        let result = crate::infrastructure::atomic_file::with_test_publication_failure(|| {
            crate::application::init::init_agents(dir.path(), &FileInitStore)
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn init_rejects_dangling_config_without_creating_referent() {
        let dir = tempfile::tempdir().unwrap();
        let referent = dir.path().join("missing.json");
        std::os::unix::fs::symlink(&referent, dir.path().join("sksync.config.json")).unwrap();
        assert!(init_project(dir.path(), &FileInitStore).is_err());
        assert!(!referent.exists());
    }

    #[test]
    fn init_creates_config_and_skills_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");

        let result = init_project(temp_dir.path(), &FileInitStore).expect("init succeeds");

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

        let result = init_global(temp_dir.path(), &FileInitStore).expect("init global succeeds");

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
    fn init_and_refresh_include_additional_agent_mappings() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        init_global(temp_dir.path(), &FileInitStore).expect("init global succeeds");
        let mapping_path = temp_dir.path().join("agents.json");
        let content = std::fs::read_to_string(&mapping_path).expect("read agents");
        let agents: serde_json::Value = serde_json::from_str(&content).expect("parse agents");

        for (agent, global, project) in [
            ("oh-my-pi", "~/.omp/agent/skills", Some(".omp/skills")),
            ("empryo", "~/.empryo/skills", Some(".empryo/skills")),
            ("phi", "~/.phi/skills", None),
            ("pig", "~/.pig/agent/skills", Some(".pig/skills")),
            ("vtcode", "~/.agents/skills", Some(".agents/skills")),
            ("fx", "~/.fx/skills", Some(".agents/skills")),
        ] {
            assert_eq!(agents["global"][agent]["targetDir"], global, "{agent}");
            if let Some(project) = project {
                assert_eq!(agents["project"][agent]["targetDir"], project, "{agent}");
            } else {
                assert!(agents["project"].get(agent).is_none(), "{agent}");
            }
        }

        std::fs::write(&mapping_path, "custom agents").expect("write agents");
        crate::application::init::init_agents(temp_dir.path(), &FileInitStore)
            .expect("refresh agents succeeds");
        assert_eq!(
            std::fs::read_to_string(mapping_path).expect("read agents"),
            content
        );
    }

    #[test]
    fn init_global_does_not_overwrite_existing_agent_mapping() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let agent_mapping_path = temp_dir.path().join("agents.json");
        std::fs::write(&agent_mapping_path, "custom").expect("write agents");

        let result = init_global(temp_dir.path(), &FileInitStore).expect("init global succeeds");

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

        let result = crate::application::init::init_agents(temp_dir.path(), &FileInitStore)
            .expect("init agents succeeds");

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

        let error =
            init_project(temp_dir.path(), &FileInitStore).expect_err("existing config fails");

        assert!(matches!(error, InitError::ConfigExists(_)));
    }

    #[test]
    fn init_global_fails_when_config_exists() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(temp_dir.path().join("config.json"), "{}").expect("write config");

        let error =
            init_global(temp_dir.path(), &FileInitStore).expect_err("existing config fails");

        assert!(matches!(error, InitError::ConfigExists(_)));
    }
}
