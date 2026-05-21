use std::fmt;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SkillNameError {
    #[error("skill name must not be empty")]
    Empty,
    #[error("skill name must not contain path separators")]
    ContainsPathSeparator,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SkillName(String);

impl SkillName {
    pub fn new(value: impl Into<String>) -> Result<Self, SkillNameError> {
        let value = value.into();
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(SkillNameError::Empty);
        }

        if trimmed.contains(std::path::MAIN_SEPARATOR)
            || trimmed.contains('/')
            || trimmed.contains('\\')
        {
            return Err(SkillNameError::ContainsPathSeparator);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SkillName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourcePathError {
    #[error("source path must not be empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourcePath(PathBuf);

impl SourcePath {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, SourcePathError> {
        let path = path.into();

        if path.components().next().is_none() || path == Path::new("") {
            return Err(SourcePathError::Empty);
        }

        Ok(Self(expand_tilde(path)))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let Some(path_string) = path.to_str() else {
        return path;
    };

    if path_string == "~" || path_string.starts_with("~/") {
        PathBuf::from(shellexpand::tilde(path_string).into_owned())
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::{SkillName, SkillNameError, SourcePath, SourcePathError};
    use std::path::Path;

    #[test]
    fn skill_name_accepts_simple_name() {
        let name = SkillName::new("rust-format").expect("valid skill name");
        assert_eq!(name.as_str(), "rust-format");
    }

    #[test]
    fn skill_name_trims_outer_whitespace() {
        let name = SkillName::new("  review  ").expect("valid skill name");
        assert_eq!(name.as_str(), "review");
    }

    #[test]
    fn skill_name_rejects_empty_name() {
        assert_eq!(SkillName::new("  "), Err(SkillNameError::Empty));
    }

    #[test]
    fn skill_name_rejects_path_separators() {
        assert_eq!(
            SkillName::new("nested/skill"),
            Err(SkillNameError::ContainsPathSeparator)
        );
        assert_eq!(
            SkillName::new("nested\\skill"),
            Err(SkillNameError::ContainsPathSeparator)
        );
    }

    #[test]
    fn source_path_rejects_empty_path() {
        assert_eq!(SourcePath::new(""), Err(SourcePathError::Empty));
    }

    #[test]
    fn source_path_accepts_relative_path() {
        let path = SourcePath::new("skills/review").expect("valid source path");
        assert_eq!(path.as_path(), Path::new("skills/review"));
    }
}
