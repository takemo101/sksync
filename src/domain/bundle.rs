use std::fmt;

use thiserror::Error;

use crate::domain::skill::SkillName;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BundleNameError {
    #[error("bundle name must not be empty")]
    Empty,
    #[error("bundle name must not contain path separators")]
    ContainsPathSeparator,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BundleName(String);

impl BundleName {
    pub fn new(value: impl Into<String>) -> Result<Self, BundleNameError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(BundleNameError::Empty);
        }
        if trimmed.contains(std::path::MAIN_SEPARATOR)
            || trimmed.contains('/')
            || trimmed.contains('\\')
        {
            return Err(BundleNameError::ContainsPathSeparator);
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BundleName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BundleProvenance {
    pub name: BundleName,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleEntry {
    pub skill_name: SkillName,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleManifest {
    pub name: BundleName,
    pub description: String,
    pub entries: Vec<BundleEntry>,
}

#[cfg(test)]
mod tests {
    use super::{BundleName, BundleNameError, BundleProvenance};
    use std::collections::BTreeSet;

    #[test]
    fn bundle_name_accepts_simple_name() {
        let name = BundleName::new("review-workflow").expect("valid bundle name");
        assert_eq!(name.as_str(), "review-workflow");
    }

    #[test]
    fn bundle_name_trims_outer_whitespace() {
        let name = BundleName::new("  review-workflow  ").expect("valid bundle name");
        assert_eq!(name.as_str(), "review-workflow");
    }

    #[test]
    fn bundle_name_rejects_empty_name() {
        assert_eq!(BundleName::new("  "), Err(BundleNameError::Empty));
    }

    #[test]
    fn bundle_name_rejects_path_separators() {
        assert_eq!(
            BundleName::new("team/review"),
            Err(BundleNameError::ContainsPathSeparator)
        );
        assert_eq!(
            BundleName::new("team\\review"),
            Err(BundleNameError::ContainsPathSeparator)
        );
    }

    #[test]
    fn provenance_deduplicates_by_name_and_source() {
        let first = BundleProvenance {
            name: BundleName::new("baseline").unwrap(),
            source: "./bundles/base".to_owned(),
        };
        let same = first.clone();
        let different_source = BundleProvenance {
            name: BundleName::new("baseline").unwrap(),
            source: "./bundles/other".to_owned(),
        };

        let set = BTreeSet::from([first, same, different_source]);
        assert_eq!(set.len(), 2);
    }
}
