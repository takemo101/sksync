use std::path::{Path, PathBuf};

use thiserror::Error;

use super::config::{ConfigResolveError, ResolvedConfig};
use crate::domain::skill::SourcePath;
use crate::domain::target::TargetPath;

#[derive(Debug, Error)]
pub enum ConfigStoreError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Resolve(#[from] ConfigResolveError),
}

pub trait ConfigStore {
    fn load(&self) -> Result<ResolvedConfig, ConfigStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetState {
    Missing,
    SymlinkToExpectedSource,
    SymlinkToUnexpectedSource { actual_source: PathBuf },
    RegularFileConflict,
    DirectoryConflict,
    BrokenSymlink { actual_source: PathBuf },
}

#[derive(Debug, Error)]
pub enum LinkStoreError {
    #[error("failed to inspect target {path}: {source}")]
    Inspect {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read symlink target {path}: {source}")]
    ReadLink {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub trait LinkStore {
    fn inspect_target(
        &self,
        target: &TargetPath,
        expected_source: &SourcePath,
    ) -> Result<TargetState, LinkStoreError>;
}

pub fn display_path(path: &Path) -> String {
    path.display().to_string()
}
