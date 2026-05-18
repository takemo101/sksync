use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TargetPathError {
    #[error("target path must not be empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetPath(PathBuf);

impl TargetPath {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, TargetPathError> {
        let path = path.into();

        if path.components().next().is_none() || path == Path::new("") {
            return Err(TargetPathError::Empty);
        }

        Ok(Self(path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{TargetPath, TargetPathError};
    use std::path::Path;

    #[test]
    fn target_path_rejects_empty_path() {
        assert_eq!(TargetPath::new(""), Err(TargetPathError::Empty));
    }

    #[test]
    fn target_path_accepts_resolved_path() {
        let path = TargetPath::new(".claude/skills/review").expect("valid target path");
        assert_eq!(path.as_path(), Path::new(".claude/skills/review"));
    }
}
