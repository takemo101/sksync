use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::application::ports::{
    display_path, LinkStore, LinkStoreError, SourceStore, SourceStoreError, TargetState,
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
    use crate::application::ports::{LinkStore, TargetState};
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
}
