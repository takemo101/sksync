use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::application::config::InstallSource;
use crate::application::ports::{SkillInstallError, SkillInstaller};

#[derive(Debug, Clone, Default)]
pub struct FileSystemSkillInstaller;

impl SkillInstaller for FileSystemSkillInstaller {
    fn install_skill(
        &self,
        source: &InstallSource,
        destination: &Path,
        skill_name: &str,
    ) -> Result<String, SkillInstallError> {
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|error| SkillInstallError::Prepare {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;

        let staging = staging_dir(parent, skill_name);
        if staging.exists() {
            remove_dir(&staging)?;
        }
        fs::create_dir_all(&staging).map_err(|error| SkillInstallError::Prepare {
            path: staging.display().to_string(),
            message: error.to_string(),
        })?;

        let source_label = install_to_staging(source, &staging)?;
        replace_destination(&staging, destination)?;
        Ok(source_label)
    }
}

fn install_to_staging(source: &InstallSource, staging: &Path) -> Result<String, SkillInstallError> {
    match source {
        InstallSource::Local(path) => {
            if !path.exists() {
                return Err(SkillInstallError::MissingSourcePath {
                    path: path.display().to_string(),
                });
            }
            copy_dir_contents(path, staging)?;
            Ok(path.display().to_string())
        }
        InstallSource::Registry(registry_source) => Err(SkillInstallError::UnsupportedRegistry {
            registry: registry_source.registry.clone(),
            package: registry_source.package.clone(),
        }),
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
                .map_err(|error| SkillInstallError::Git {
                    repo: git_source.url.clone(),
                    message: error.to_string(),
                })?;
            if !output.status.success() {
                return Err(SkillInstallError::Git {
                    repo: git_source.url.clone(),
                    message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                });
            }
            let source_path = clone_dir.join(&git_source.path);
            if !source_path.exists() {
                return Err(SkillInstallError::MissingSourcePath {
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

fn replace_destination(staging: &Path, destination: &Path) -> Result<(), SkillInstallError> {
    if destination.exists() {
        remove_dir(destination)?;
    }
    fs::rename(staging, destination).map_err(|error| SkillInstallError::Prepare {
        path: destination.display().to_string(),
        message: error.to_string(),
    })
}

fn copy_dir_contents(from: &Path, to: &Path) -> Result<(), SkillInstallError> {
    for entry in fs::read_dir(from).map_err(|error| SkillInstallError::Copy {
        from: from.display().to_string(),
        to: to.display().to_string(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| SkillInstallError::Copy {
            from: from.display().to_string(),
            to: to.display().to_string(),
            message: error.to_string(),
        })?;
        let source_path = entry.path();
        let target_path = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| SkillInstallError::Copy {
            from: source_path.display().to_string(),
            to: target_path.display().to_string(),
            message: error.to_string(),
        })?;
        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| SkillInstallError::Prepare {
                path: target_path.display().to_string(),
                message: error.to_string(),
            })?;
            copy_dir_contents(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path).map_err(|error| SkillInstallError::Copy {
                from: source_path.display().to_string(),
                to: target_path.display().to_string(),
                message: error.to_string(),
            })?;
        }
    }
    Ok(())
}

fn remove_dir(path: &Path) -> Result<(), SkillInstallError> {
    fs::remove_dir_all(path).map_err(|error| SkillInstallError::Prepare {
        path: path.display().to_string(),
        message: error.to_string(),
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
    use super::FileSystemSkillInstaller;
    use crate::application::config::InstallSource;
    use crate::application::ports::SkillInstaller;

    #[test]
    fn local_dependency_is_copied_into_destination() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), "# Review").unwrap();
        let destination = temp.path().join("skills/review");

        FileSystemSkillInstaller
            .install_skill(&InstallSource::Local(remote), &destination, "review")
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "# Review"
        );
    }
}
