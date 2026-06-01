use std::path::{Component, Path};

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PackagePatternError {
    #[error("include must contain at least one pattern")]
    NoPatterns,
    #[error("include pattern must not be empty")]
    Empty,
    #[error("include pattern must be relative")]
    Absolute,
    #[error("include pattern must not contain '..'")]
    ParentComponent,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageFilter {
    patterns: Vec<String>,
}

impl PackageFilter {
    pub fn new(patterns: Vec<String>) -> Result<Self, PackagePatternError> {
        if patterns.is_empty() {
            return Err(PackagePatternError::NoPatterns);
        }

        let mut normalized = Vec::new();
        for pattern in patterns {
            let trimmed = pattern.trim();
            if trimmed.is_empty() {
                return Err(PackagePatternError::Empty);
            }
            let normalized_pattern = trimmed.replace('\\', "/");
            let path = Path::new(&normalized_pattern);
            if path.is_absolute()
                || normalized_pattern
                    .split_once(':')
                    .is_some_and(|(prefix, rest)| prefix.len() == 1 && rest.starts_with('/'))
            {
                return Err(PackagePatternError::Absolute);
            }
            if path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
                || normalized_pattern.split('/').any(|component| component == "..")
            {
                return Err(PackagePatternError::ParentComponent);
            }
            normalized.push(normalized_pattern);
        }
        normalized.sort();
        normalized.dedup();
        Ok(Self { patterns: normalized })
    }

    pub fn manifest_only() -> Self {
        Self {
            patterns: vec!["SKILL.md".to_owned()],
        }
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

#[cfg(test)]
mod tests {
    use super::{PackageFilter, PackagePatternError};

    #[test]
    fn include_patterns_are_normalized_and_sorted() {
        let filter = PackageFilter::new(vec!["references".into(), "SKILL.md".into()]).unwrap();
        assert_eq!(filter.patterns(), &["SKILL.md", "references"]);
    }

    #[test]
    fn include_rejects_empty_pattern_list() {
        let error = PackageFilter::new(Vec::new()).unwrap_err();
        assert_eq!(error, PackagePatternError::NoPatterns);
    }

    #[test]
    fn include_rejects_empty_patterns() {
        let error = PackageFilter::new(vec![" ".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::Empty);
    }

    #[test]
    fn include_rejects_absolute_paths() {
        let error = PackageFilter::new(vec!["/tmp/SKILL.md".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::Absolute);
    }

    #[test]
    fn include_rejects_windows_absolute_paths() {
        let error = PackageFilter::new(vec!["C:\\tmp\\SKILL.md".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::Absolute);
    }

    #[test]
    fn include_rejects_parent_components() {
        let error = PackageFilter::new(vec!["../SKILL.md".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::ParentComponent);
    }

    #[test]
    fn include_rejects_windows_parent_components() {
        let error = PackageFilter::new(vec!["..\\SKILL.md".into()]).unwrap_err();
        assert_eq!(error, PackagePatternError::ParentComponent);
    }
}
