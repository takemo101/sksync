use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::application::config::{GitInstallSource, InstallSource, RegistryInstallSource};
use crate::application::ports::{InstalledSkillSource, SkillInstallError, SkillInstaller};
use crate::application::registry::RegistryProviders;

#[derive(Debug, Clone, Default)]
pub struct FileSystemSkillInstaller;

impl SkillInstaller for FileSystemSkillInstaller {
    fn install_skill(
        &self,
        source: &InstallSource,
        destination: &Path,
        skill_name: &str,
    ) -> Result<InstalledSkillSource, SkillInstallError> {
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

        let result = install_to_staging(source, &staging)
            .and_then(|installed| replace_destination(&staging, destination).map(|()| installed));
        if result.is_err() && staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }
}

fn install_to_staging(
    source: &InstallSource,
    staging: &Path,
) -> Result<InstalledSkillSource, SkillInstallError> {
    match source {
        InstallSource::Local(path) => {
            if !path.exists() {
                return Err(SkillInstallError::MissingSourcePath {
                    path: path.display().to_string(),
                });
            }
            copy_dir_contents(path, staging)?;
            Ok(InstalledSkillSource {
                label: path.display().to_string(),
                resolved_source: source.clone(),
            })
        }
        InstallSource::Registry(registry_source) => {
            let git_source = RegistryProviders::default().resolve_git_source(registry_source)?;
            let installed = install_git_to_staging(&git_source, staging)?;
            let resolved_reference = match installed.resolved_source {
                InstallSource::Git(git) => git.reference,
                _ => registry_source.reference.clone(),
            };
            Ok(InstalledSkillSource {
                label: installed.label,
                resolved_source: InstallSource::Registry(RegistryInstallSource {
                    registry: registry_source.registry.clone(),
                    package: registry_source.package.clone(),
                    reference: resolved_reference,
                }),
            })
        }
        InstallSource::Git(git_source) => install_git_to_staging(git_source, staging),
    }
}

fn install_git_to_staging(
    git_source: &GitInstallSource,
    staging: &Path,
) -> Result<InstalledSkillSource, SkillInstallError> {
    let clone_dir = staging.join(".repo");
    clone_git_source(git_source, &clone_dir)?;
    let source_path = clone_dir.join(&git_source.path);
    if !source_path.exists() {
        return Err(SkillInstallError::MissingSourcePath {
            path: source_path.display().to_string(),
        });
    }
    copy_dir_contents(&source_path, staging)?;
    let rev = resolve_git_head(&clone_dir, &git_source.url)?;
    remove_dir(&clone_dir)?;
    let resolved_source = InstallSource::Git(GitInstallSource {
        url: git_source.url.clone(),
        reference: Some(rev.clone()),
        path: git_source.path.clone(),
    });
    Ok(InstalledSkillSource {
        label: format!("{}#{}:{}", git_source.url, rev, git_source.path.display()),
        resolved_source,
    })
}

fn clone_git_source(
    git_source: &GitInstallSource,
    clone_dir: &Path,
) -> Result<(), SkillInstallError> {
    let output = Command::new("git")
        .arg("clone")
        .arg("--filter=blob:none")
        .arg("--no-checkout")
        .arg(&git_source.url)
        .arg(clone_dir)
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

    checkout_git_reference(
        clone_dir,
        &git_source.url,
        git_source.reference.as_deref().unwrap_or("HEAD"),
    )
}

fn checkout_git_reference(
    clone_dir: &Path,
    repo: &str,
    reference: &str,
) -> Result<(), SkillInstallError> {
    if run_git(clone_dir, repo, &["checkout", "--detach", reference]).is_ok() {
        return Ok(());
    }

    run_git(
        clone_dir,
        repo,
        &["fetch", "--depth", "1", "origin", reference],
    )?;
    run_git(clone_dir, repo, &["checkout", "--detach", "FETCH_HEAD"])
}

fn run_git(clone_dir: &Path, repo: &str, args: &[&str]) -> Result<(), SkillInstallError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(clone_dir)
        .args(args)
        .output()
        .map_err(|error| SkillInstallError::Git {
            repo: repo.to_owned(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SkillInstallError::Git {
            repo: repo.to_owned(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

fn resolve_git_head(clone_dir: &Path, repo: &str) -> Result<String, SkillInstallError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(clone_dir)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .map_err(|error| SkillInstallError::Git {
            repo: repo.to_owned(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(SkillInstallError::Git {
            repo: repo.to_owned(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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
    use crate::application::config::{GitInstallSource, InstallSource};
    use crate::application::ports::{SkillInstallError, SkillInstaller};
    use std::path::Path;
    use std::process::Command;

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

    #[test]
    fn git_dependency_can_install_exact_commit_reference() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        create_git_skill_repo(&remote, "# Review v1");
        let rev = git_output(&remote, &["rev-parse", "HEAD"]);
        std::fs::write(remote.join("skills/review/SKILL.md"), "# Review v2").unwrap();
        git(&remote, &["add", "."]);
        git(&remote, &["commit", "-m", "update review"]);
        let destination = temp.path().join("skills/review");

        FileSystemSkillInstaller
            .install_skill(
                &InstallSource::Git(GitInstallSource {
                    url: remote.display().to_string(),
                    reference: Some(rev.clone()),
                    path: "skills/review".into(),
                }),
                &destination,
                "review",
            )
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "# Review v1"
        );
    }

    #[test]
    fn staging_directory_is_removed_when_install_fails() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("skills/review");
        let error = FileSystemSkillInstaller
            .install_skill(
                &InstallSource::Local(temp.path().join("missing")),
                &destination,
                "review",
            )
            .expect_err("missing source should fail");

        assert!(matches!(error, SkillInstallError::MissingSourcePath { .. }));
        assert!(!temp
            .path()
            .join(format!(
                "skills/.sksync-update-review-{}",
                std::process::id()
            ))
            .exists());
    }

    fn create_git_skill_repo(path: &Path, skill_content: &str) {
        std::fs::create_dir_all(path.join("skills/review")).unwrap();
        git(path, &["init"]);
        git(path, &["config", "user.email", "test@example.com"]);
        git(path, &["config", "user.name", "Test User"]);
        std::fs::write(path.join("skills/review/SKILL.md"), skill_content).unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "add review"]);
    }

    fn git(path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}
