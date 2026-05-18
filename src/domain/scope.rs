use std::fmt;
use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScopeError {
    #[error("scope must be either 'user' or 'project'")]
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    User,
    Project,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

impl FromStr for Scope {
    type Err = ScopeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "user" => Ok(Self::User),
            "project" => Ok(Self::Project),
            _ => Err(ScopeError::Invalid),
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{Scope, ScopeError};
    use std::str::FromStr;

    #[test]
    fn parses_valid_scope_values() {
        assert_eq!(Scope::from_str("user"), Ok(Scope::User));
        assert_eq!(Scope::from_str("PROJECT"), Ok(Scope::Project));
    }

    #[test]
    fn rejects_invalid_scope_values() {
        assert_eq!(Scope::from_str("global"), Err(ScopeError::Invalid));
        assert_eq!(Scope::from_str(""), Err(ScopeError::Invalid));
    }
}
