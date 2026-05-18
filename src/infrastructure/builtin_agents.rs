use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::application::ports::{TargetResolver, TargetResolverError};
use crate::domain::agent::AgentKind;
use crate::domain::scope::Scope;
use crate::domain::target::{TargetPath, TargetPathError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BuiltinAgentMappingError {
    #[error("custom agent '{0}' requires targetDir override")]
    MissingCustomTarget(String),
    #[error("target path is invalid: {0}")]
    InvalidTarget(#[from] TargetPathError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPathResolver {
    project_root: PathBuf,
    home_dir: PathBuf,
}

impl TargetPathResolver {
    pub fn new(project_root: impl Into<PathBuf>, home_dir: impl Into<PathBuf>) -> Self {
        Self {
            project_root: project_root.into(),
            home_dir: home_dir.into(),
        }
    }

    pub fn resolve(
        &self,
        agent: &AgentKind,
        scope: Scope,
        target_dir_override: Option<&Path>,
    ) -> Result<TargetPath, BuiltinAgentMappingError> {
        let raw_target = match target_dir_override {
            Some(path) => path.to_path_buf(),
            None => default_target_dir(agent, scope)?,
        };
        let resolved = self.resolve_path(scope, &raw_target);

        TargetPath::new(resolved).map_err(BuiltinAgentMappingError::from)
    }

    fn resolve_path(&self, scope: Scope, raw_target: &Path) -> PathBuf {
        if let Ok(stripped) = raw_target.strip_prefix("~") {
            return self.home_dir.join(stripped);
        }

        if scope == Scope::Project && raw_target.is_relative() {
            return self.project_root.join(raw_target);
        }

        raw_target.to_path_buf()
    }
}

impl TargetResolver for TargetPathResolver {
    fn resolve_agent_target(
        &self,
        agent: &AgentKind,
        scope: Scope,
        target_dir_override: Option<&Path>,
    ) -> Result<TargetPath, TargetResolverError> {
        self.resolve(agent, scope, target_dir_override)
            .map_err(|error| TargetResolverError::Resolve {
                agent: agent.as_str().to_owned(),
                scope,
                message: error.to_string(),
            })
    }
}

pub fn default_target_dir(
    agent: &AgentKind,
    scope: Scope,
) -> Result<PathBuf, BuiltinAgentMappingError> {
    let path = match (agent, scope) {
        (AgentKind::Pi, Scope::User) => "~/.pi/agent/skills",
        (AgentKind::Pi, Scope::Project) => ".pi/agent/skills",
        (AgentKind::ClaudeCode, Scope::User) => "~/.claude/skills",
        (AgentKind::ClaudeCode, Scope::Project) => ".claude/skills",
        (AgentKind::Codex, Scope::User) => "~/.codex/skills",
        (AgentKind::Codex, Scope::Project) => ".codex/skills",
        (AgentKind::Gemini, Scope::User) => "~/.gemini/skills",
        (AgentKind::Gemini, Scope::Project) => ".gemini/skills",
        (AgentKind::OpenCode, Scope::User) => "~/.config/opencode/skills",
        (AgentKind::OpenCode, Scope::Project) => ".opencode/skills",
        (AgentKind::Custom(name), _) => {
            return Err(BuiltinAgentMappingError::MissingCustomTarget(
                name.as_str().to_owned(),
            ));
        }
    };

    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::{default_target_dir, BuiltinAgentMappingError, TargetPathResolver};
    use crate::domain::agent::{AgentKind, AgentName};
    use crate::domain::scope::Scope;
    use std::path::Path;

    fn resolver() -> TargetPathResolver {
        TargetPathResolver::new("/workspace/project", "/home/example")
    }

    #[test]
    fn default_mapping_covers_each_builtin_agent_and_scope() {
        let cases = [
            (AgentKind::Pi, Scope::User, "~/.pi/agent/skills"),
            (AgentKind::Pi, Scope::Project, ".pi/agent/skills"),
            (AgentKind::ClaudeCode, Scope::User, "~/.claude/skills"),
            (AgentKind::ClaudeCode, Scope::Project, ".claude/skills"),
            (AgentKind::Codex, Scope::User, "~/.codex/skills"),
            (AgentKind::Codex, Scope::Project, ".codex/skills"),
            (AgentKind::Gemini, Scope::User, "~/.gemini/skills"),
            (AgentKind::Gemini, Scope::Project, ".gemini/skills"),
            (
                AgentKind::OpenCode,
                Scope::User,
                "~/.config/opencode/skills",
            ),
            (AgentKind::OpenCode, Scope::Project, ".opencode/skills"),
        ];

        for (agent, scope, expected) in cases {
            assert_eq!(
                default_target_dir(&agent, scope).unwrap(),
                Path::new(expected)
            );
        }
    }

    #[test]
    fn user_scope_expands_home_without_touching_real_home() {
        let target = resolver()
            .resolve(&AgentKind::Pi, Scope::User, None)
            .expect("target resolves");

        assert_eq!(
            target.as_path(),
            Path::new("/home/example/.pi/agent/skills")
        );
    }

    #[test]
    fn project_scope_resolves_relative_to_project_root() {
        let target = resolver()
            .resolve(&AgentKind::ClaudeCode, Scope::Project, None)
            .expect("target resolves");

        assert_eq!(
            target.as_path(),
            Path::new("/workspace/project/.claude/skills")
        );
    }

    #[test]
    fn override_can_replace_default_target_dir() {
        let target = resolver()
            .resolve(
                &AgentKind::Codex,
                Scope::Project,
                Some(Path::new("custom/skills")),
            )
            .expect("target resolves");

        assert_eq!(
            target.as_path(),
            Path::new("/workspace/project/custom/skills")
        );
    }

    #[test]
    fn override_can_use_home_expansion() {
        let target = resolver()
            .resolve(
                &AgentKind::Gemini,
                Scope::User,
                Some(Path::new("~/custom-gemini")),
            )
            .expect("target resolves");

        assert_eq!(target.as_path(), Path::new("/home/example/custom-gemini"));
    }

    #[test]
    fn custom_agent_requires_override() {
        let custom = AgentKind::Custom(AgentName::new("my-agent").unwrap());
        assert_eq!(
            resolver().resolve(&custom, Scope::User, None),
            Err(BuiltinAgentMappingError::MissingCustomTarget(
                "my-agent".to_owned()
            ))
        );
    }
}
