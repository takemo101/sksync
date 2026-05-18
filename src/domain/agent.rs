use std::fmt;
use std::str::FromStr;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentNameError {
    #[error("agent name must not be empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentName(String);

impl AgentName {
    pub fn new(value: impl Into<String>) -> Result<Self, AgentNameError> {
        let value = value.into();
        let trimmed = value.trim();

        if trimmed.is_empty() {
            return Err(AgentNameError::Empty);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentKindError {
    #[error("agent kind must not be empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Pi,
    ClaudeCode,
    Codex,
    Gemini,
    OpenCode,
    Custom(AgentName),
}

impl AgentKind {
    pub fn custom(name: impl Into<String>) -> Result<Self, AgentNameError> {
        AgentName::new(name).map(Self::Custom)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Pi => "pi",
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Custom(name) => name.as_str(),
        }
    }
}

impl FromStr for AgentKind {
    type Err = AgentKindError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase();

        if normalized.is_empty() {
            return Err(AgentKindError::Empty);
        }

        Ok(match normalized.as_str() {
            "pi" => Self::Pi,
            "claude" | "claude-code" | "claude_code" => Self::ClaudeCode,
            "codex" => Self::Codex,
            "gemini" => Self::Gemini,
            "opencode" | "open-code" | "open_code" => Self::OpenCode,
            _ => Self::Custom(AgentName(normalized)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentKind, AgentKindError, AgentName, AgentNameError};
    use std::str::FromStr;

    #[test]
    fn agent_name_rejects_empty_name() {
        assert_eq!(AgentName::new(" "), Err(AgentNameError::Empty));
    }

    #[test]
    fn agent_kind_parses_builtin_aliases() {
        assert_eq!(AgentKind::from_str("pi"), Ok(AgentKind::Pi));
        assert_eq!(
            AgentKind::from_str("claude_code"),
            Ok(AgentKind::ClaudeCode)
        );
        assert_eq!(AgentKind::from_str("open-code"), Ok(AgentKind::OpenCode));
    }

    #[test]
    fn agent_kind_rejects_empty_value() {
        assert_eq!(AgentKind::from_str("  "), Err(AgentKindError::Empty));
    }

    #[test]
    fn agent_kind_preserves_custom_agent() {
        let kind = AgentKind::from_str("MyAgent").expect("custom agent");
        assert_eq!(kind.as_str(), "myagent");
    }
}
