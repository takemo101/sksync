use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

use super::config::{InstallSource, ResolvedConfig, ResolvedSkill};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub updated: Vec<UpdatedSkill>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatedSkill {
    pub name: String,
    pub source: String,
    pub destination: PathBuf,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to remove directory {path}: {source}")]
    RemoveDir {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to copy {from} to {to}: {source}")]
    Copy {
        from: String,
        to: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to rename {from} to {to}: {source}")]
    Rename {
        from: String,
        to: String,
        #[source]
        source: std::io::Error,
    },
    #[error("git command failed for {repo}: {message}")]
    Git { repo: String, message: String },
    #[error("install source path does not exist for {skill}: {path}")]
    MissingSourcePath { skill: String, path: String },
}

pub fn update_dependencies(config: &ResolvedConfig) -> Result<UpdateReport, UpdateError> {
    fs::create_dir_all(config.skill_dir.as_path()).map_err(|source| UpdateError::CreateDir {
        path: config.skill_dir.as_path().display().to_string(),
        source,
    })?;

    let mut report = UpdateReport {
        updated: Vec::new(),
        skipped: Vec::new(),
    };

    for skill in &config.skills {
        let Some(install_source) = &skill.install_source else {
            report.skipped.push(skill.name.as_str().to_owned());
            continue;
        };
        let destination = skill.source.as_path().to_path_buf();
        let staging = staging_dir(config.skill_dir.as_path(), skill.name.as_str());
        if staging.exists() {
            remove_dir(&staging)?;
        }
        fs::create_dir_all(&staging).map_err(|source| UpdateError::CreateDir {
            path: staging.display().to_string(),
            source,
        })?;

        let source_label = install_to_staging(skill, install_source, &staging)?;
        replace_destination(&staging, &destination)?;
        report.updated.push(UpdatedSkill {
            name: skill.name.as_str().to_owned(),
            source: source_label,
            destination,
        });
    }

    Ok(report)
}

fn install_to_staging(
    skill: &ResolvedSkill,
    source: &InstallSource,
    staging: &Path,
) -> Result<String, UpdateError> {
    match source {
        InstallSource::Local(path) => {
            if !path.exists() {
                return Err(UpdateError::MissingSourcePath {
                    skill: skill.name.as_str().to_owned(),
                    path: path.display().to_string(),
                });
            }
            copy_dir_contents(path, staging)?;
            Ok(path.display().to_string())
        }
        InstallSource::Git(git_source) => {
            let clone_dir = staging.join(".repo");
            let mut command = Command::new("git");
            command
                .arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--filter=blob:none");
            if let Some(reference) = &git_source.reference {
                command.arg("--branch").arg(reference);
            }
            let output = command
                .arg(&git_source.url)
                .arg(&clone_dir)
                .output()
                .map_err(|source| UpdateError::Git {
                    repo: git_source.url.clone(),
                    message: source.to_string(),
                })?;
            if !output.status.success() {
                return Err(UpdateError::Git {
                    repo: git_source.url.clone(),
                    message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                });
            }
            let source_path = clone_dir.join(&git_source.path);
            if !source_path.exists() {
                return Err(UpdateError::MissingSourcePath {
                    skill: skill.name.as_str().to_owned(),
                    path: source_path.display().to_string(),
                });
            }
            copy_dir_contents(&source_path, staging)?;
            remove_dir(&clone_dir)?;
            Ok(format!(
                "{}{}:{}",
                git_source.url,
                git_source
                    .reference
                    .as_ref()
                    .map(|reference| format!("#{reference}"))
                    .unwrap_or_default(),
                git_source.path.display()
            ))
        }
    }
}

fn replace_destination(staging: &Path, destination: &Path) -> Result<(), UpdateError> {
    if destination.exists() {
        remove_dir(destination)?;
    }
    fs::rename(staging, destination).map_err(|source| UpdateError::Rename {
        from: staging.display().to_string(),
        to: destination.display().to_string(),
        source,
    })
}

fn copy_dir_contents(from: &Path, to: &Path) -> Result<(), UpdateError> {
    for entry in fs::read_dir(from).map_err(|source| UpdateError::Copy {
        from: from.display().to_string(),
        to: to.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| UpdateError::Copy {
            from: from.display().to_string(),
            to: to.display().to_string(),
            source,
        })?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|source| UpdateError::Copy {
            from: source_path.display().to_string(),
            to: target_path.display().to_string(),
            source,
        })?;
        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|source| UpdateError::CreateDir {
                path: target_path.display().to_string(),
                source,
            })?;
            copy_dir_contents(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).map_err(|source| UpdateError::Copy {
                from: source_path.display().to_string(),
                to: target_path.display().to_string(),
                source,
            })?;
        }
    }
    Ok(())
}

fn remove_dir(path: &Path) -> Result<(), UpdateError> {
    fs::remove_dir_all(path).map_err(|source| UpdateError::RemoveDir {
        path: path.display().to_string(),
        source,
    })
}

fn staging_dir(skill_dir: &Path, skill_name: &str) -> PathBuf {
    skill_dir.join(format!(
        ".sksync-update-{skill_name}-{}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::update_dependencies;
    use crate::application::config::{InstallSource, ResolvedAgent, ResolvedConfig, ResolvedSkill};
    use crate::domain::agent::AgentKind;
    use crate::domain::scope::Scope;
    use crate::domain::skill::{SkillName, SourcePath};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn local_dependency_is_copied_into_skill_dir() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), "# Review").unwrap();
        let skill_dir = temp.path().join("skills");
        let config = config(skill_dir.clone(), InstallSource::Local(remote));

        let report = update_dependencies(&config).unwrap();

        assert_eq!(report.updated.len(), 1);
        assert_eq!(report.updated[0].name, "review");
        assert_eq!(
            std::fs::read_to_string(skill_dir.join("review/SKILL.md")).unwrap(),
            "# Review"
        );
    }

    fn config(skill_dir: PathBuf, install_source: InstallSource) -> ResolvedConfig {
        let mut agents = BTreeMap::new();
        agents.insert(
            "pi".to_owned(),
            ResolvedAgent {
                kind: AgentKind::Pi,
                enabled: true,
                scope: Scope::User,
                target_dir: None,
            },
        );
        ResolvedConfig {
            skill_dir: SourcePath::new(skill_dir.clone()).unwrap(),
            agents,
            skills: vec![ResolvedSkill {
                name: SkillName::new("review").unwrap(),
                source: SourcePath::new(skill_dir.join("review")).unwrap(),
                install_source: Some(install_source),
                agents: vec![AgentKind::Pi],
            }],
        }
    }
}
