use std::path::{Path, PathBuf};

use thiserror::Error;

use super::config::{ConfigResolveError, InstallSource, ResolvedConfig};
use crate::domain::agent::AgentKind;
use crate::domain::lockfile::{Digest, Lockfile};
use crate::domain::scope::Scope;
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

#[derive(Debug, Error)]
pub enum DependencyConfigStoreError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create config directory {path}: {source}")]
    CreateDir {
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
    #[error("invalid config field: {0}")]
    InvalidField(String),
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to write config at {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub trait DependencyConfigStore {
    fn add_dependency(
        &self,
        skill_name: &str,
        source: &str,
        agents: &[String],
    ) -> Result<(), DependencyConfigStoreError>;

    fn remove_dependency(&self, skill_name: &str) -> Result<(), DependencyConfigStoreError>;

    fn remove_dependency_agents(
        &self,
        skill_name: &str,
        agents: &[String],
    ) -> Result<Vec<String>, DependencyConfigStoreError>;
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

#[derive(Debug, Error)]
pub enum LinkApplyError {
    #[error("failed to create parent directory {path}: {source}")]
    CreateParent {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("target already exists at {path}")]
    TargetExists { path: String },
    #[error("failed to create symlink {target} -> {source}: {error}")]
    CreateSymlink {
        source: String,
        target: String,
        #[source]
        error: std::io::Error,
    },
}

pub trait LinkApplier {
    fn create_symlink(
        &self,
        source: &SourcePath,
        target: &TargetPath,
    ) -> Result<(), LinkApplyError>;
}

#[derive(Debug, Error)]
pub enum SourceStoreError {
    #[error("failed to inspect source {path}: {source}")]
    Inspect {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub trait SourceStore {
    fn source_exists(&self, source: &SourcePath) -> Result<bool, SourceStoreError>;
}

#[derive(Debug, Error)]
pub enum SkillInstallError {
    #[error("failed to prepare destination {path}: {message}")]
    Prepare { path: String, message: String },
    #[error("install source path does not exist: {path}")]
    MissingSourcePath { path: String },
    #[error("git command failed for {repo}: {message}")]
    Git { repo: String, message: String },
    #[error("failed to copy {from} to {to}: {message}")]
    Copy {
        from: String,
        to: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledSkillSource {
    pub label: String,
    pub resolved_source: InstallSource,
}

pub trait SkillInstaller {
    fn install_skill(
        &self,
        source: &InstallSource,
        destination: &Path,
        skill_name: &str,
    ) -> Result<InstalledSkillSource, SkillInstallError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceHash {
    pub hash: Digest,
}

#[derive(Debug, Error)]
pub enum SourceHashStoreError {
    #[error("failed to hash source {path}: {message}")]
    Hash { path: String, message: String },
}

pub trait SourceHashStore {
    fn hash_source(&self, source: &SourcePath) -> Result<SourceHash, SourceHashStoreError>;
}

#[derive(Debug, Error)]
pub enum TargetResolverError {
    #[error("failed to resolve target for agent '{agent}' and scope '{scope}': {message}")]
    Resolve {
        agent: String,
        scope: Scope,
        message: String,
    },
}

pub trait TargetResolver {
    fn resolve_agent_target(
        &self,
        agent: &AgentKind,
        scope: Scope,
        target_dir_override: Option<&Path>,
    ) -> Result<TargetPath, TargetResolverError>;
}

#[derive(Debug, Error)]
pub enum LockfileStoreError {
    #[error("failed to write lockfile: {0}")]
    Write(String),
}

pub trait LockfileStore {
    fn write(&self, lockfile: &Lockfile) -> Result<(), LockfileStoreError>;
}

pub fn display_path(path: &Path) -> String {
    path.display().to_string()
}
