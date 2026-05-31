use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::application::ports::{
    display_path, LinkApplier, LinkApplyError, LinkStore, LinkStoreError, SourceStore,
    SourceStoreError, TargetState,
};
use crate::domain::skill::SourcePath;
use crate::domain::target::TargetPath;

#[derive(Debug, Default, Clone, Copy)]
pub struct FileSystemLinkStore;

impl LinkStore for FileSystemLinkStore {
    fn inspect_target(
        &self,
        target: &TargetPath,
        expected_source: &SourcePath,
    ) -> Result<TargetState, LinkStoreError> {
        inspect_target_path(target.as_path(), expected_source.as_path())
    }
}

impl SourceStore for FileSystemLinkStore {
    fn source_exists(&self, source: &SourcePath) -> Result<bool, SourceStoreError> {
        match fs::metadata(source.as_path()) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source_error) => Err(SourceStoreError::Inspect {
                path: display_path(source.as_path()),
                source: source_error,
            }),
        }
    }
}

impl LinkApplier for FileSystemLinkStore {
    fn create_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError> {
        if fs::symlink_metadata(target.as_path()).is_ok() {
            return Err(LinkApplyError::TargetExists {
                path: display_path(target.as_path()),
            });
        }

        create_symlink_at(source.as_path(), target.as_path())
    }

    fn replace_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError> {
        let link_source = canonicalize_link_source(source.as_path())?;
        let metadata = fs::symlink_metadata(target.as_path()).map_err(|source| {
            LinkApplyError::RemoveSymlink {
                path: display_path(target.as_path()),
                source,
            }
        })?;

        if !metadata.file_type().is_symlink() {
            return Err(LinkApplyError::TargetNotSymlink {
                path: display_path(target.as_path()),
            });
        }

        fs::remove_file(target.as_path()).map_err(|source| LinkApplyError::RemoveSymlink {
            path: display_path(target.as_path()),
            source,
        })?;

        create_symlink_at_resolved(&link_source, target.as_path())
    }
}

fn create_symlink_at(source: &Path, target: &Path) -> Result<(), LinkApplyError> {
    let link_source = canonicalize_link_source(source)?;
    create_symlink_at_resolved(&link_source, target)
}

fn canonicalize_link_source(source: &Path) -> Result<PathBuf, LinkApplyError> {
    source
        .canonicalize()
        .map_err(|error| LinkApplyError::SourceMissing {
            path: display_path(source),
            source: error,
        })
}

fn create_symlink_at_resolved(link_source: &Path, target: &Path) -> Result<(), LinkApplyError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|source| LinkApplyError::CreateParent {
            path: display_path(parent),
            source,
        })?;
    }

    std::os::unix::fs::symlink(link_source, target).map_err(|error| LinkApplyError::CreateSymlink {
        source: display_path(link_source),
        target: display_path(target),
        error,
    })
}

fn inspect_target_path(
    target: &Path,
    expected_source: &Path,
) -> Result<TargetState, LinkStoreError> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(TargetState::Missing),
        Err(source) => {
            return Err(LinkStoreError::Inspect {
                path: display_path(target),
                source,
            });
        }
    };
    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        return inspect_symlink(target, expected_source);
    }

    if file_type.is_dir() {
        return Ok(TargetState::DirectoryConflict);
    }

    Ok(TargetState::RegularFileConflict)
}

fn inspect_symlink(target: &Path, expected_source: &Path) -> Result<TargetState, LinkStoreError> {
    let actual_source = fs::read_link(target).map_err(|source| LinkStoreError::ReadLink {
        path: display_path(target),
        source,
    })?;

    let resolved_actual_source = resolve_link_destination(target, &actual_source);

    if !resolved_actual_source.exists() {
        return Ok(TargetState::BrokenSymlink { actual_source });
    }

    if paths_equivalent(&resolved_actual_source, expected_source) {
        return Ok(TargetState::SymlinkToExpectedSource);
    }

    Ok(TargetState::SymlinkToUnexpectedSource { actual_source })
}

fn paths_equivalent(actual: &Path, expected: &Path) -> bool {
    actual == expected || canonicalize_lossy(actual) == canonicalize_lossy(expected)
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_link_destination(link_path: &Path, destination: &Path) -> PathBuf {
    if destination.is_absolute() {
        destination.to_path_buf()
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(destination)
    }
}

#[cfg(test)]
mod tests {
    use super::FileSystemLinkStore;
    use crate::application::ports::{LinkApplier, LinkApplyError, LinkStore, TargetState};
    use crate::domain::skill::SourcePath;
    use crate::domain::target::TargetPath;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn source_path(path: impl Into<std::path::PathBuf>) -> SourcePath {
        SourcePath::new(path).expect("valid source path")
    }

    fn target_path(path: impl Into<std::path::PathBuf>) -> TargetPath {
        TargetPath::new(path).expect("valid target path")
    }

    #[test]
    fn detects_missing_target() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let target = target_path(temp_dir.path().join("missing"));
        let source = source_path(temp_dir.path().join("source"));

        let state = FileSystemLinkStore
            .inspect_target(&target, &source)
            .expect("inspect target");

        assert_eq!(state, TargetState::Missing);
    }

    #[test]
    fn detects_symlink_to_expected_source() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let source_dir = temp_dir.path().join("source");
        let target_link = temp_dir.path().join("target");
        fs::create_dir(&source_dir).expect("create source");
        symlink(&source_dir, &target_link).expect("create symlink");

        let state = FileSystemLinkStore
            .inspect_target(&target_path(target_link), &source_path(source_dir))
            .expect("inspect target");

        assert_eq!(state, TargetState::SymlinkToExpectedSource);
    }

    #[test]
    fn detects_relative_symlink_to_expected_source() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let source_dir = temp_dir.path().join("source");
        let links_dir = temp_dir.path().join("links");
        let target_link = links_dir.join("target");
        fs::create_dir(&source_dir).expect("create source");
        fs::create_dir(&links_dir).expect("create links dir");
        symlink("../source", &target_link).expect("create relative symlink");

        let state = FileSystemLinkStore
            .inspect_target(&target_path(target_link), &source_path(source_dir))
            .expect("inspect target");

        assert_eq!(state, TargetState::SymlinkToExpectedSource);
    }

    #[test]
    fn detects_symlink_to_unexpected_source() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let expected = temp_dir.path().join("expected");
        let actual = temp_dir.path().join("actual");
        let target_link = temp_dir.path().join("target");
        fs::create_dir(&expected).expect("create expected");
        fs::create_dir(&actual).expect("create actual");
        symlink(&actual, &target_link).expect("create symlink");

        let state = FileSystemLinkStore
            .inspect_target(&target_path(target_link), &source_path(expected))
            .expect("inspect target");

        assert_eq!(
            state,
            TargetState::SymlinkToUnexpectedSource {
                actual_source: actual
            }
        );
    }

    #[test]
    fn detects_regular_file_conflict_without_treating_it_as_symlink() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let target = temp_dir.path().join("target");
        let source = temp_dir.path().join("source");
        fs::write(&target, "not a symlink").expect("write target file");
        fs::create_dir(&source).expect("create source");

        let state = FileSystemLinkStore
            .inspect_target(&target_path(target), &source_path(source))
            .expect("inspect target");

        assert_eq!(state, TargetState::RegularFileConflict);
    }

    #[test]
    fn detects_directory_conflict() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let target = temp_dir.path().join("target");
        let source = temp_dir.path().join("source");
        fs::create_dir(&target).expect("create target dir");
        fs::create_dir(&source).expect("create source dir");

        let state = FileSystemLinkStore
            .inspect_target(&target_path(target), &source_path(source))
            .expect("inspect target");

        assert_eq!(state, TargetState::DirectoryConflict);
    }

    #[test]
    fn detects_broken_symlink() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let missing_source = temp_dir.path().join("missing-source");
        let target_link = temp_dir.path().join("target");
        symlink(&missing_source, &target_link).expect("create broken symlink");

        let state = FileSystemLinkStore
            .inspect_target(
                &target_path(target_link),
                &source_path(temp_dir.path().join("expected")),
            )
            .expect("inspect target");

        assert_eq!(
            state,
            TargetState::BrokenSymlink {
                actual_source: missing_source
            }
        );
    }

    #[test]
    fn replace_symlink_replaces_unexpected_symlink() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let expected = temp_dir.path().join("expected");
        let actual = temp_dir.path().join("actual");
        let target_link = temp_dir.path().join("target");
        fs::create_dir(&expected).expect("create expected");
        fs::create_dir(&actual).expect("create actual");
        symlink(&actual, &target_link).expect("create existing symlink");

        FileSystemLinkStore
            .replace_symlink(&source_path(&expected), &target_path(&target_link))
            .expect("replace symlink");

        let replaced = fs::read_link(&target_link).expect("read replaced link");
        assert_eq!(
            replaced,
            expected.canonicalize().expect("canonical expected")
        );
    }

    #[test]
    fn replace_symlink_replaces_broken_symlink() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let expected = temp_dir.path().join("expected");
        let target_link = temp_dir.path().join("target");
        fs::create_dir(&expected).expect("create expected");
        symlink(temp_dir.path().join("missing"), &target_link).expect("create broken symlink");

        FileSystemLinkStore
            .replace_symlink(&source_path(&expected), &target_path(&target_link))
            .expect("replace broken symlink");

        let replaced = fs::read_link(&target_link).expect("read replaced link");
        assert_eq!(
            replaced,
            expected.canonicalize().expect("canonical expected")
        );
    }

    #[test]
    fn replace_symlink_refuses_missing_source_without_removing_existing_link() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let missing = temp_dir.path().join("missing");
        let actual = temp_dir.path().join("actual");
        let target_link = temp_dir.path().join("target");
        fs::create_dir(&actual).expect("create actual");
        symlink(&actual, &target_link).expect("create existing symlink");

        FileSystemLinkStore
            .replace_symlink(&source_path(missing), &target_path(&target_link))
            .expect_err("missing source is not linked");

        assert_eq!(
            fs::read_link(&target_link).expect("read preserved link"),
            actual
        );
    }

    #[test]
    fn replace_symlink_refuses_regular_file() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let expected = temp_dir.path().join("expected");
        let target = temp_dir.path().join("target");
        fs::create_dir(&expected).expect("create expected");
        fs::write(&target, "manual file").expect("write file");

        let error = FileSystemLinkStore
            .replace_symlink(&source_path(expected), &target_path(&target))
            .expect_err("regular file is not replaced");

        assert!(matches!(error, LinkApplyError::TargetNotSymlink { .. }));
        assert_eq!(
            fs::read_to_string(target).expect("read file"),
            "manual file"
        );
    }

    #[test]
    fn replace_symlink_refuses_directory() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let expected = temp_dir.path().join("expected");
        let target = temp_dir.path().join("target");
        fs::create_dir(&expected).expect("create expected");
        fs::create_dir(&target).expect("create target directory");

        let error = FileSystemLinkStore
            .replace_symlink(&source_path(expected), &target_path(target))
            .expect_err("directory is not replaced");

        assert!(matches!(error, LinkApplyError::TargetNotSymlink { .. }));
    }
}
