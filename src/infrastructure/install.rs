use crate::application::ports::{
    InstalledSkillSource, SkillInstallError, SkillInstallRequest, SkillInstaller,
};
use crate::domain::package_filter::PackageFilter;
use crate::domain::skill_manifest::parse_skill_manifest;
use crate::domain::source::{GitInstallSource, InstallSource};
use crate::infrastructure::git::{GitClient, GitCommandError};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct FileSystemSkillInstaller;

impl SkillInstaller for FileSystemSkillInstaller {
    fn install_skill(
        &self,
        request: &SkillInstallRequest,
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

        let result = install_to_staging(request, &staging).and_then(|installed| {
            validate_skill_package(&staging)?;
            replace_destination(&staging, destination).map(|()| installed)
        });
        if result.is_err() && staging.exists() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }
}

// P05 will consume this check once install requests carry the configured managed root.
#[allow(dead_code)]
pub(crate) fn validate_managed_destination(
    skill_dir: &Path,
    destination: &Path,
) -> Result<(), SkillInstallError> {
    let prepare_error = |path: &Path, message: String| SkillInstallError::Prepare {
        path: path.display().to_string(),
        message,
    };
    let root = skill_dir
        .canonicalize()
        .map_err(|error| prepare_error(skill_dir, error.to_string()))?;
    if !root.is_dir() {
        return Err(prepare_error(
            skill_dir,
            "managed skill store must be a directory".into(),
        ));
    }
    // Strip trailing separators/dots so metadata cannot follow a body symlink via `link/`.
    let destination = std::path::absolute(destination)
        .map_err(|error| prepare_error(destination, error.to_string()))?
        .components()
        .collect::<PathBuf>();
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(prepare_error(
                &destination,
                "managed body must be a directory, not a symlink or other file".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(prepare_error(&destination, error.to_string())),
    }

    let resolved = resolve_managed_directory(&destination)?;
    if resolved == root || !resolved.starts_with(&root) {
        return Err(prepare_error(
            &destination,
            "managed body must be strictly inside the configured skill store".into(),
        ));
    }
    let parent = destination.parent().ok_or_else(|| {
        prepare_error(
            &destination,
            "managed body must have a parent directory".into(),
        )
    })?;
    if !resolve_managed_directory(parent)?.starts_with(&root) {
        return Err(prepare_error(
            parent,
            "managed body parent escapes the configured skill store".into(),
        ));
    }
    Ok(())
}

/// Resolve through the nearest existing directory without creating missing namespaces.
/// symlink_metadata distinguishes a dangling alias from a genuinely missing directory.
fn resolve_managed_directory(path: &Path) -> Result<PathBuf, SkillInstallError> {
    let prepare_error = |message: String| SkillInstallError::Prepare {
        path: path.display().to_string(),
        message,
    };
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(existing) {
            Ok(_) => {
                let mut resolved = existing
                    .canonicalize()
                    .map_err(|error| prepare_error(error.to_string()))?;
                if !resolved.is_dir() {
                    return Err(prepare_error(
                        "managed body parent must be a directory".into(),
                    ));
                }
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = existing.file_name().ok_or_else(|| {
                    prepare_error("missing managed path must not contain parent traversal".into())
                })?;
                missing.push(component);
                existing = existing
                    .parent()
                    .ok_or_else(|| prepare_error("managed path has no existing parent".into()))?;
            }
            Err(error) => return Err(prepare_error(error.to_string())),
        }
    }
}

fn install_to_staging(
    request: &SkillInstallRequest,
    staging: &Path,
) -> Result<InstalledSkillSource, SkillInstallError> {
    match &request.source {
        InstallSource::Local(path) => {
            if !path.exists() {
                return Err(SkillInstallError::MissingSourcePath {
                    path: path.display().to_string(),
                });
            }
            copy_package_contents(path, staging, request.include.as_ref())?;
            Ok(InstalledSkillSource {
                label: path.display().to_string(),
                resolved_source: request.source.clone(),
            })
        }
        InstallSource::Git(git_source) => {
            install_git_to_staging(git_source, staging, request.include.as_ref())
        }
    }
}

fn install_git_to_staging(
    git_source: &GitInstallSource,
    staging: &Path,
    include: Option<&PackageFilter>,
) -> Result<InstalledSkillSource, SkillInstallError> {
    validate_git_subpath(&git_source.path)?;
    let clone_dir = staging.join(".repo");
    let git = GitClient;
    git.clone_checkout(git_source, &clone_dir)
        .map_err(skill_install_git_error)?;
    let source_path = safe_git_source_path(&clone_dir, &git_source.path)?;
    copy_package_contents(&source_path, staging, include)?;
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

const PROTECTED_DIRS: &[&str] = &[".git", ".sksync", "node_modules"];

fn copy_package_contents(
    from: &Path,
    to: &Path,
    include: Option<&PackageFilter>,
) -> Result<(), SkillInstallError> {
    match include {
        None => copy_dir_contents(from, to),
        Some(filter) => copy_filtered_contents(from, to, filter),
    }
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

fn copy_filtered_contents(
    from: &Path,
    to: &Path,
    filter: &PackageFilter,
) -> Result<(), SkillInstallError> {
    let canonical_root = from
        .canonicalize()
        .map_err(|error| SkillInstallError::Prepare {
            path: from.display().to_string(),
            message: error.to_string(),
        })?;
    for pattern in filter.patterns() {
        let matches = include_matches(from, pattern)?;
        if matches.is_empty() {
            return Err(SkillInstallError::InvalidSkillPackage {
                path: from.display().to_string(),
                message: format!("include pattern matched no files: {pattern}"),
            });
        }
        for source_path in matches {
            copy_filtered_path(&canonical_root, &source_path, to)?;
        }
    }
    Ok(())
}

fn include_matches(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, SkillInstallError> {
    if !contains_glob(pattern) {
        let path = root.join(pattern);
        if path.exists() && !has_protected_component(path.strip_prefix(root).unwrap_or(&path)) {
            return Ok(vec![path]);
        }
        return Ok(Vec::new());
    }

    let mut matches = Vec::new();
    collect_matching_files(root, root, pattern, &mut matches)?;
    Ok(matches)
}

fn contains_glob(pattern: &str) -> bool {
    pattern.contains('*')
}

fn collect_matching_files(
    root: &Path,
    current: &Path,
    pattern: &str,
    matches: &mut Vec<PathBuf>,
) -> Result<(), SkillInstallError> {
    for entry in fs::read_dir(current).map_err(|error| SkillInstallError::Copy {
        from: current.display().to_string(),
        to: root.display().to_string(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| SkillInstallError::Copy {
            from: current.display().to_string(),
            to: root.display().to_string(),
            message: error.to_string(),
        })?;
        let source_path = entry.path();
        let relative = source_path.strip_prefix(root).unwrap_or(&source_path);
        if has_protected_component(relative) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| SkillInstallError::Copy {
            from: source_path.display().to_string(),
            to: root.display().to_string(),
            message: error.to_string(),
        })?;
        if file_type.is_dir() {
            collect_matching_files(root, &source_path, pattern, matches)?;
        } else if file_type.is_file() && glob_matches(relative, pattern) {
            matches.push(source_path);
        }
    }
    Ok(())
}

fn glob_matches(relative: &Path, pattern: &str) -> bool {
    if pattern == "**" {
        return true;
    }
    let relative = relative.to_string_lossy().replace('\\', "/");
    if let Some(prefix) = pattern.strip_suffix("/**") {
        let prefix = prefix.trim_end_matches('/');
        return relative == prefix || relative.starts_with(&format!("{prefix}/"));
    }
    let path_parts = relative.split('/').collect::<Vec<_>>();
    let pattern_parts = pattern.split('/').collect::<Vec<_>>();
    path_parts.len() == pattern_parts.len()
        && path_parts
            .iter()
            .zip(pattern_parts.iter())
            .all(|(path, pattern)| segment_matches(path, pattern))
}

fn segment_matches(value: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return value == pattern;
    };
    value.starts_with(prefix) && value.ends_with(suffix)
}

fn copy_filtered_path(
    root: &Path,
    source_path: &Path,
    target_root: &Path,
) -> Result<(), SkillInstallError> {
    let canonical_source =
        source_path
            .canonicalize()
            .map_err(|error| SkillInstallError::Prepare {
                path: source_path.display().to_string(),
                message: error.to_string(),
            })?;
    if !canonical_source.starts_with(root) {
        return Err(SkillInstallError::Copy {
            from: source_path.display().to_string(),
            to: target_root.display().to_string(),
            message: "include path escapes package root".to_owned(),
        });
    }
    let relative =
        canonical_source
            .strip_prefix(root)
            .map_err(|error| SkillInstallError::Copy {
                from: canonical_source.display().to_string(),
                to: target_root.display().to_string(),
                message: error.to_string(),
            })?;
    if has_protected_component(relative) {
        return Ok(());
    }
    let target_path = target_root.join(relative);
    if canonical_source.is_dir() {
        fs::create_dir_all(&target_path).map_err(|error| SkillInstallError::Prepare {
            path: target_path.display().to_string(),
            message: error.to_string(),
        })?;
        copy_filtered_directory(root, &canonical_source, target_root)?;
    } else if canonical_source.is_file() {
        copy_file(&canonical_source, &target_path)?;
    }
    Ok(())
}

fn copy_filtered_directory(
    root: &Path,
    source: &Path,
    target_root: &Path,
) -> Result<(), SkillInstallError> {
    for entry in fs::read_dir(source).map_err(|error| SkillInstallError::Copy {
        from: source.display().to_string(),
        to: target_root.display().to_string(),
        message: error.to_string(),
    })? {
        let entry = entry.map_err(|error| SkillInstallError::Copy {
            from: source.display().to_string(),
            to: target_root.display().to_string(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if has_protected_component(relative) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| SkillInstallError::Copy {
            from: path.display().to_string(),
            to: target_root.display().to_string(),
            message: error.to_string(),
        })?;
        let target = target_root.join(relative);
        if file_type.is_dir() {
            fs::create_dir_all(&target).map_err(|error| SkillInstallError::Prepare {
                path: target.display().to_string(),
                message: error.to_string(),
            })?;
            copy_filtered_directory(root, &path, target_root)?;
        } else if file_type.is_file() {
            copy_file(&path, &target)?;
        }
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> Result<(), SkillInstallError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| SkillInstallError::Prepare {
            path: parent.display().to_string(),
            message: error.to_string(),
        })?;
    }
    fs::copy(source, target).map_err(|error| SkillInstallError::Copy {
        from: source.display().to_string(),
        to: target.display().to_string(),
        message: error.to_string(),
    })?;
    Ok(())
}

fn has_protected_component(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|part| PROTECTED_DIRS.contains(&part))
    })
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
    use super::{validate_managed_destination, FileSystemSkillInstaller};
    use crate::application::ports::{SkillInstallError, SkillInstallRequest, SkillInstaller};
    use crate::domain::package_filter::PackageFilter;
    use crate::domain::source::{GitInstallSource, InstallSource};
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn managed_destination_accepts_flat_and_missing_namespaced_bodies() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir(&root).unwrap();
        let flat = root.join("review");
        std::fs::create_dir(&flat).unwrap();
        std::fs::write(flat.join("SKILL.md"), b"old").unwrap();
        validate_managed_destination(&root, &flat).unwrap();
        let namespaced = root.join("local/source/.review");
        validate_managed_destination(&root, &namespaced).unwrap();
        assert!(!root.join("local").exists());
        assert_eq!(std::fs::read(flat.join("SKILL.md")).unwrap(), b"old");
    }

    #[test]
    fn managed_destination_rejects_store_root_and_outside_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("namespace")).unwrap();
        std::fs::create_dir(temp.path().join("skills-other")).unwrap();
        for destination in [
            root.clone(),
            root.join("."),
            root.join("namespace/.."),
            root.join("../review"),
            temp.path().join("skills-other/review"),
            root.join("missing/../../review"),
        ] {
            assert!(
                validate_managed_destination(&root, &destination).is_err(),
                "accepted {}",
                destination.display()
            );
        }
    }

    #[test]
    fn managed_destination_rejects_missing_or_non_directory_store_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        assert!(validate_managed_destination(&root, &root.join("review")).is_err());
        std::fs::write(&root, b"store file").unwrap();
        assert!(validate_managed_destination(&root, &root.join("review")).is_err());
        assert_eq!(std::fs::read(&root).unwrap(), b"store file");
    }

    #[test]
    fn managed_destination_rejects_regular_files_and_non_directory_parents() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let file = root.join("review");
        std::fs::write(&file, b"unmanaged").unwrap();
        assert!(validate_managed_destination(root, &file).is_err());
        assert!(validate_managed_destination(root, &file.join("child")).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"unmanaged");
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_existing_and_dangling_body_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        std::fs::create_dir_all(root.join("real")).unwrap();
        for (name, target) in [
            ("linked", root.join("real")),
            ("dangling", root.join("absent")),
        ] {
            let destination = root.join(name);
            symlink(&target, &destination).unwrap();
            for body in [
                destination.clone(),
                destination.join(""),
                destination.join("."),
            ] {
                assert!(
                    validate_managed_destination(&root, &body).is_err(),
                    "accepted body alias {}",
                    body.display()
                );
            }
            assert_eq!(std::fs::read_link(&destination).unwrap(), target);
        }
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_escaping_parent_aliases() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("skills");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir_all(outside.join("existing")).unwrap();
        symlink(&outside, root.join("alias")).unwrap();
        for destination in [
            root.join("alias/existing"),
            root.join("alias/new/namespace/review"),
        ] {
            assert!(validate_managed_destination(&root, &destination).is_err());
        }
        assert!(!outside.join("new").exists());
        assert_eq!(std::fs::read_link(root.join("alias")).unwrap(), outside);
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_dangling_parent_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let alias = root.join("alias");
        let target = root.join("absent");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        assert!(validate_managed_destination(root, &alias.join("namespace/review")).is_err());
        assert_eq!(std::fs::read_link(alias).unwrap(), target);
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_accepts_authorized_store_symlink_and_internal_parent_alias() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let physical = temp.path().join("physical");
        let root = temp.path().join("store");
        std::fs::create_dir_all(physical.join("namespace/existing")).unwrap();
        symlink(&physical, &root).unwrap();
        symlink(physical.join("namespace"), physical.join("alias")).unwrap();
        validate_managed_destination(&root, &root.join("namespace/existing")).unwrap();
        validate_managed_destination(&root, &root.join("alias/missing/review")).unwrap();
        assert!(validate_managed_destination(&root, &root).is_err());
        assert!(!physical.join("namespace/missing").exists());
        assert_eq!(std::fs::read_link(&root).unwrap(), physical);
    }

    #[cfg(unix)]
    #[test]
    fn managed_destination_rejects_special_files() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("socket");
        let _socket = std::os::unix::net::UnixListener::bind(&destination).unwrap();
        assert!(validate_managed_destination(temp.path(), &destination).is_err());
        assert!(std::fs::symlink_metadata(&destination).is_ok());
    }

    #[test]
    fn local_dependency_is_copied_into_destination() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote/review");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        let destination = temp.path().join("skills/review");

        FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(remote)),
                &destination,
                "review",
            )
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
                &request(InstallSource::Git(GitInstallSource {
                    url: remote.display().to_string(),
                    reference: Some(rev.clone()),
                    path: "skills/review".into(),
                })),
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
                &request(InstallSource::Git(GitInstallSource {
                    url: remote.display().to_string(),
                    reference: None,
                    path: "../review".into(),
                })),
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
                &request(InstallSource::Git(GitInstallSource {
                    url: remote.display().to_string(),
                    reference: None,
                    path: "skills/escape".into(),
                })),
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
            .install_skill(
                &request(InstallSource::Local(remote)),
                &destination,
                "review",
            )
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
            .install_skill(
                &request(InstallSource::Local(remote)),
                &destination,
                "review",
            )
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
            .install_skill(
                &request(InstallSource::Local(remote)),
                &destination,
                "review",
            )
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
            .install_skill(
                &request(InstallSource::Local(remote)),
                &destination,
                "review",
            )
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
            .install_skill(
                &request(InstallSource::Local(remote)),
                &destination,
                "review",
            )
            .expect_err("non-string description should fail");

        assert!(matches!(
            error,
            SkillInstallError::InvalidSkillPackage { message, .. }
                if message == "SKILL.md frontmatter field 'description' must be a string"
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn install_with_manifest_only_copies_only_skill_md() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join("src")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("herdr", "Herdr helper")).unwrap();
        std::fs::write(remote.join("Cargo.toml"), "[package]\nname = \"herdr\"\n").unwrap();
        std::fs::write(remote.join("src/main.rs"), "fn main() {}\n").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(InstallSource::Local(remote), PackageFilter::manifest_only()),
                &destination,
                "herdr",
            )
            .unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(!destination.join("Cargo.toml").exists());
        assert!(!destination.join("src").exists());
    }

    #[test]
    fn install_with_directory_include_recursively_copies_references() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join("references/nested")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        std::fs::write(remote.join("references/nested/guide.md"), "guide").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["SKILL.md".into(), "references".into()]).unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(destination.join("references/nested/guide.md").is_file());
    }

    #[test]
    fn install_with_glob_include_copies_matching_files() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join("assets")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        std::fs::write(remote.join("assets/logo.png"), "png").unwrap();
        std::fs::write(remote.join("assets/readme.txt"), "txt").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["SKILL.md".into(), "assets/*.png".into()]).unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();

        assert!(destination.join("assets/logo.png").is_file());
        assert!(!destination.join("assets/readme.txt").exists());
    }

    #[test]
    fn include_pattern_must_match() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();

        let error = FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["missing".into()]).unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("include pattern matched no files"));
    }

    #[test]
    fn protected_dirs_are_not_copied_by_filtered_installs() {
        let temp = tempfile::tempdir().unwrap();
        let remote = temp.path().join("remote");
        let destination = temp.path().join("installed");
        std::fs::create_dir_all(remote.join(".git/objects")).unwrap();
        std::fs::write(remote.join("SKILL.md"), skill_md("review", "Review helper")).unwrap();
        std::fs::write(remote.join(".git/config"), "secret").unwrap();

        FileSystemSkillInstaller
            .install_skill(
                &filtered_request(
                    InstallSource::Local(remote),
                    PackageFilter::new(vec!["SKILL.md".into(), "**".into()]).unwrap(),
                ),
                &destination,
                "review",
            )
            .unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(!destination.join(".git/config").exists());
    }

    #[test]
    fn staging_directory_is_removed_when_install_fails() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("skills/review");
        let error = FileSystemSkillInstaller
            .install_skill(
                &request(InstallSource::Local(temp.path().join("missing"))),
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

    fn request(source: InstallSource) -> SkillInstallRequest {
        SkillInstallRequest {
            source,
            include: None,
        }
    }

    fn filtered_request(source: InstallSource, include: PackageFilter) -> SkillInstallRequest {
        SkillInstallRequest {
            source,
            include: Some(include),
        }
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

    #[test]
    fn git_fixture_command_sets_temporary_home_environment_without_spawning() {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("remote");
        std::fs::create_dir(&repository).unwrap();
        let command = git_command(&repository, &["rev-parse", "HEAD"]);
        let home = temp.path().join("git-test-home");
        let config = home.join(".config");

        for (key, expected) in [
            ("HOME", &home),
            ("USERPROFILE", &home),
            ("XDG_CONFIG_HOME", &config),
        ] {
            let actual = command
                .get_envs()
                .find(|(name, _)| *name == std::ffi::OsStr::new(key))
                .and_then(|(_, value)| value);
            assert_eq!(
                actual,
                Some(expected.as_os_str()),
                "missing temporary {key}"
            );
            assert!(expected.starts_with(temp.path()));
            assert!(!expected.starts_with(&repository));
            assert!(expected.is_dir());
        }
    }

    fn git_command(path: &Path, args: &[&str]) -> Command {
        // Keep home configuration outside the repository's committed fixture contents.
        let home = path
            .parent()
            .expect("temporary fixture parent")
            .join("git-test-home");
        let config = home.join(".config");
        std::fs::create_dir_all(&config).unwrap();
        let mut command = Command::new("git");
        command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &config)
            .arg("-C")
            .arg(path)
            .args(args);
        command
    }

    fn git(path: &Path, args: &[&str]) {
        let output = git_command(path, args).output().unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(path: &Path, args: &[&str]) -> String {
        let output = git_command(path, args).output().unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}
