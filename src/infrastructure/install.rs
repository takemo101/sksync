use crate::application::config::{GitInstallSource, InstallSource};
use crate::application::ports::{InstalledSkillSource, SkillInstallError, SkillInstaller};
use crate::domain::skill_manifest::parse_skill_manifest;
use crate::infrastructure::git::{GitClient, GitCommandError};
use std::fs;
use std::path::{Component, Path, PathBuf};

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

        let result = install_to_staging(source, &staging).and_then(|installed| {
            validate_skill_package(&staging)?;
            replace_destination(&staging, destination).map(|()| installed)
        });
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
        InstallSource::Git(git_source) => install_git_to_staging(git_source, staging),
    }
}

fn install_git_to_staging(
    git_source: &GitInstallSource,
    staging: &Path,
) -> Result<InstalledSkillSource, SkillInstallError> {
    validate_git_subpath(&git_source.path)?;
    let clone_dir = staging.join(".repo");
    let git = GitClient;
    git.clone_checkout(git_source, &clone_dir)
        .map_err(skill_install_git_error)?;
    let source_path = safe_git_source_path(&clone_dir, &git_source.path)?;
    copy_dir_contents(&source_path, staging)?;
    let rev = git
        .resolve_head(&clone_dir, &git_source.url)
        .map_err(skill_install_git_error)?;
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

fn validate_git_subpath(path: &Path) -> Result<(), SkillInstallError> {
    let is_safe = !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::CurDir | Component::Normal(_)));
    if is_safe {
        Ok(())
    } else {
        Err(SkillInstallError::InvalidGitSubpath {
            path: path.display().to_string(),
            message: "path must be relative and must not contain '..'".to_owned(),
        })
    }
}

fn safe_git_source_path(clone_dir: &Path, subpath: &Path) -> Result<PathBuf, SkillInstallError> {
    let source_path = clone_dir.join(subpath);
    if !source_path.exists() {
        return Err(SkillInstallError::MissingSourcePath {
            path: source_path.display().to_string(),
        });
    }

    let canonical_clone = clone_dir
        .canonicalize()
        .map_err(|error| SkillInstallError::Prepare {
            path: clone_dir.display().to_string(),
            message: error.to_string(),
        })?;
    let canonical_source =
        source_path
            .canonicalize()
            .map_err(|error| SkillInstallError::Prepare {
                path: source_path.display().to_string(),
                message: error.to_string(),
            })?;
    if !canonical_source.starts_with(&canonical_clone) {
        return Err(SkillInstallError::InvalidGitSubpath {
            path: subpath.display().to_string(),
            message: "resolved path escapes cloned repository".to_owned(),
        });
    }

    Ok(canonical_source)
}

fn skill_install_git_error(error: GitCommandError) -> SkillInstallError {
    SkillInstallError::Git {
        repo: error.repo,
        message: error.message,
    }
}

fn validate_skill_package(path: &Path) -> Result<(), SkillInstallError> {
    let skill_md = path.join("SKILL.md");
    if !skill_md.exists() {
        return Err(SkillInstallError::InvalidSkillPackage {
            path: path.display().to_string(),
            message: "SKILL.md is missing".to_owned(),
        });
    }
    if !skill_md.is_file() {
        return Err(SkillInstallError::InvalidSkillPackage {
            path: skill_md.display().to_string(),
            message: "SKILL.md must be a file".to_owned(),
        });
    }

    let content = fs::read_to_string(&skill_md).map_err(|error| SkillInstallError::Prepare {
        path: skill_md.display().to_string(),
        message: error.to_string(),
    })?;
    parse_skill_manifest(&content).map_err(|error| SkillInstallError::InvalidSkillPackage {
        path: skill_md.display().to_string(),
        message: error.to_string(),
    })?;
    Ok(())
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
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        let destination = temp.path().join("skills/review");

        FileSystemSkillInstaller
            .install_skill(&InstallSource::Local(remote), &destination, "review")
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            skill_md("review", "Review helper")
        );
    }

    #[test]
    fn git_dependency_can_install_exact_commit_reference() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        create_git_skill_repo(&remote, &skill_md("review", "Review v1"));
        let rev = git_output(&remote, &["rev-parse", "HEAD"]);
        std::fs::write(
            remote.join("skills/review/SKILL.md"),
            skill_md("review", "Review v2"),
        )
        .unwrap();
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
            skill_md("review", "Review v1")
        );
    }

    #[test]
    fn git_install_rejects_parent_directory_subpath() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        create_git_skill_repo(&remote, &skill_md("review", "Review helper"));
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(
                &InstallSource::Git(GitInstallSource {
                    url: remote.display().to_string(),
                    reference: None,
                    path: "../review".into(),
                }),
                &destination,
                "review",
            )
            .expect_err("parent directory git subpath should fail");

        assert!(matches!(error, SkillInstallError::InvalidGitSubpath { .. }));
        assert!(!destination.exists());
    }

    #[test]
    fn git_install_rejects_symlink_escape_subpath() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            outside.join("SKILL.md"),
            skill_md("outside", "Outside skill"),
        )
        .unwrap();
        create_git_skill_repo(&remote, &skill_md("review", "Review helper"));
        std::os::unix::fs::symlink(&outside, remote.join("skills/escape")).unwrap();
        git(&remote, &["add", "."]);
        git(&remote, &["commit", "-m", "add escape symlink"]);
        let destination = temp.path().join("skills/escape");

        let error = FileSystemSkillInstaller
            .install_skill(
                &InstallSource::Git(GitInstallSource {
                    url: remote.display().to_string(),
                    reference: None,
                    path: "skills/escape".into(),
                }),
                &destination,
                "escape",
            )
            .expect_err("symlink escape git subpath should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidGitSubpath { message, .. }
                if message == "resolved path escapes cloned repository"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_skill_md_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(&InstallSource::Local(remote), &destination, "review")
            .expect_err("missing SKILL.md should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md is missing"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_frontmatter_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), "# Review\n").unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(&InstallSource::Local(remote), &destination, "review")
            .expect_err("missing frontmatter should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md YAML frontmatter is missing"
        ));
        assert!(!destination.exists());
        assert!(!temp
            .path()
            .join(format!(
                "skills/.sksync-update-review-{}",
                std::process::id()
            ))
            .exists());
    }

    #[test]
    fn install_fails_when_required_frontmatter_field_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(
            remote.join("SKILL.md"),
            "---\ndescription: Review helper\n---\n# Review\n",
        )
        .unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(&InstallSource::Local(remote), &destination, "review")
            .expect_err("missing name should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md frontmatter field 'name' is required"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_required_frontmatter_field_is_empty() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(
            remote.join("SKILL.md"),
            "---\nname: review\ndescription: '   '\n---\n# Review\n",
        )
        .unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(&InstallSource::Local(remote), &destination, "review")
            .expect_err("empty description should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md frontmatter field 'description' must not be empty"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_fails_when_required_frontmatter_field_is_not_a_string() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(
            remote.join("SKILL.md"),
            "---\nname: review\ndescription: 123\n---\n# Review\n",
        )
        .unwrap();
        let destination = temp.path().join("skills/review");

        let error = FileSystemSkillInstaller
            .install_skill(&InstallSource::Local(remote), &destination, "review")
            .expect_err("non-string description should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md frontmatter field 'description' must be a string"
        ));
        assert!(!destination.exists());
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

    fn skill_md(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n")
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
