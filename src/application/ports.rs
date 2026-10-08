use std::path::{Path, PathBuf};

use thiserror::Error;

use super::config::{ConfigResolveError, ResolvedConfig};
use crate::domain::agent::AgentKind;
use crate::domain::lockfile::{Digest, LockedFile, Lockfile};
use crate::domain::package_filter::PackageFilter;
use crate::domain::scope::Scope;
use crate::domain::skill::{SkillName, SourcePath};
use crate::domain::source::InstallSource;
use crate::domain::target::TargetPath;

/// A committed operation succeeded but left an owned path requiring cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupWarning {
    pub path: PathBuf,
    pub message: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddDependencyOptions {
    pub include: Option<PackageFilter>,
}

pub trait DependencyConfigStore {
    fn add_dependency(
        &self,
        skill_name: &str,
        source: &str,
        agents: &[String],
        options: AddDependencyOptions,
    ) -> Result<(), DependencyConfigStoreError>;

    fn add_dependency_agents(
        &self,
        skill_name: &str,
        agents: &[String],
    ) -> Result<Vec<String>, DependencyConfigStoreError>;

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
    #[error("source is missing at {path}: {source}")]
    SourceMissing {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("refusing to replace non-symlink target at {path}")]
    TargetNotSymlink { path: String },
    #[error("failed to remove existing symlink {path}: {source}")]
    RemoveSymlink {
        path: String,
        #[source]
        source: std::io::Error,
    },
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

    fn replace_symlink(
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
    #[error("{original}; rollback failed: {failure}")]
    RollbackFailed {
        original: Box<SkillInstallError>,
        failure: SkillRollbackFailure,
    },
    #[error("failed to prepare destination {path}: {message}")]
    Prepare { path: String, message: String },
    #[error("install source path does not exist: {path}")]
    MissingSourcePath { path: String },
    #[error("invalid skill package at {path}: {message}")]
    InvalidSkillPackage { path: String, message: String },
    #[error("invalid git source path {path}: {message}")]
    InvalidGitSubpath { path: String, message: String },
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
    pub warnings: Vec<CleanupWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallRequest {
    pub managed_root: PathBuf,
    pub source: InstallSource,
    pub include: Option<PackageFilter>,
}

pub trait SkillInstaller {
    fn install_skill(
        &self,
        request: &SkillInstallRequest,
        destination: &Path,
        skill_name: &str,
    ) -> Result<InstalledSkillSource, SkillInstallError>;
}

/// Content metadata prepared without changing the live managed body.
#[derive(Debug)]
pub struct PreparedSkill<R> {
    // P07 consumes this metadata when building the batch lockfile. Remove these
    // field-level allowances when that coordinator is implemented.
    #[allow(dead_code)]
    pub name: SkillName,
    #[allow(dead_code)]
    pub destination: PathBuf,
    pub installed: InstalledSkillSource,
    #[allow(dead_code)]
    pub hash: Digest,
    #[allow(dead_code)]
    pub files: Vec<LockedFile>,
    pub receipt: R,
}

#[derive(Debug)]
pub struct SkillRollbackFailure {
    pub skill: SkillName,
    pub retained_backup: Option<PathBuf>,
    pub message: String,
}

impl std::fmt::Display for SkillRollbackFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.skill, self.message)?;
        if let Some(path) = &self.retained_backup {
            write!(formatter, "; retained old body at {}", path.display())?;
        }
        Ok(())
    }
}

pub trait PreparedSkillInstaller {
    type Receipt;
    fn prepare_skill(
        &self,
        request: &SkillInstallRequest,
        destination: &Path,
        skill_name: &str,
    ) -> Result<PreparedSkill<Self::Receipt>, SkillInstallError>;
    fn publish_skill(&self, receipt: &mut Self::Receipt) -> Result<(), SkillInstallError>;
    fn rollback_skill(&self, receipt: &mut Self::Receipt) -> Result<(), SkillRollbackFailure>;
    fn finalize_skill(&self, receipt: &mut Self::Receipt) -> Vec<CleanupWarning>;
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

/// Serialize before publication; the prepared representation is adapter-owned.
pub trait PreparedLockfileStore: LockfileStore {
    type Prepared;
    fn prepare_lockfile(&self, value: &Lockfile) -> Result<Self::Prepared, LockfileStoreError>;
    fn publish_lockfile(
        &self,
        value: &Self::Prepared,
    ) -> Result<Vec<CleanupWarning>, LockfileStoreError>;
}

pub fn display_path(path: &Path) -> String {
    path.display().to_string()
}
