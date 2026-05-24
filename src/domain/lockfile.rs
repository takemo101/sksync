use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

use super::agent::AgentKind;
use super::scope::Scope;
use super::skill::{SkillName, SourcePath};
use super::source::InstallSource;
use super::target::TargetPath;

pub const SUPPORTED_LOCKFILE_VERSION: u32 = 4;
pub const LEGACY_LOCKFILE_VERSION: u32 = 3;
pub const LEGACY_LOCKFILE_VERSION_WITH_TARGETS: u32 = 2;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DigestError {
    #[error("digest must not be empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Digest(String);

impl Digest {
    pub fn new(value: impl Into<String>) -> Result<Self, DigestError> {
        let value = value.into();
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(DigestError::Empty);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkType {
    Symlink,
    Copy,
}

impl LinkType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Symlink => "symlink",
            Self::Copy => "copy",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinkTypeError {
    #[error("link type must be either 'symlink' or 'copy'")]
    Invalid,
}

impl std::str::FromStr for LinkType {
    type Err = LinkTypeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "symlink" => Ok(Self::Symlink),
            "copy" => Ok(Self::Copy),
            _ => Err(LinkTypeError::Invalid),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lockfile {
    pub generated_by: String,
    pub generated_at: String,
    pub root: PathBuf,
    pub skills: BTreeMap<SkillName, LockedSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedSkill {
    pub source: SourcePath,
    pub install_source: Option<InstallSource>,
    pub hash: Digest,
    pub files: Vec<LockedFile>,
    pub targets: Vec<LockedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedFile {
    pub path: PathBuf,
    pub hash: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedTarget {
    pub agent: AgentKind,
    pub scope: Scope,
    pub path: TargetPath,
    pub link_type: LinkType,
}

#[cfg(test)]
mod tests {
    use super::{Digest, DigestError, LinkType, LinkTypeError};
    use std::str::FromStr;

    #[test]
    fn digest_rejects_empty_value() {
        assert_eq!(Digest::new(" "), Err(DigestError::Empty));
    }

    #[test]
    fn link_type_parses_supported_values() {
        assert_eq!(LinkType::from_str("symlink"), Ok(LinkType::Symlink));
        assert_eq!(LinkType::from_str("COPY"), Ok(LinkType::Copy));
    }

    #[test]
    fn link_type_rejects_unknown_values() {
        assert_eq!(LinkType::from_str("hardlink"), Err(LinkTypeError::Invalid));
    }
}
