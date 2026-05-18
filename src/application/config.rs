use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

use thiserror::Error;

use crate::domain::agent::{AgentKind, AgentKindError};
use crate::domain::scope::{Scope, ScopeError};
use crate::domain::skill::{SkillName, SkillNameError, SourcePath, SourcePathError};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigResolveError {
    #[error("invalid skillDir: {0}")]
    InvalidSkillDir(#[from] SourcePathError),
    #[error("invalid agent name '{name}': {source}")]
    InvalidAgentName {
        name: String,
        #[source]
        source: AgentKindError,
    },
    #[error("invalid scope for agent '{agent}': {source}")]
    InvalidScope {
        agent: String,
        #[source]
        source: ScopeError,
    },
    #[error("invalid skill name '{name}': {source}")]
    InvalidSkillName {
        name: String,
        #[source]
        source: SkillNameError,
    },
    #[error("invalid source for skill '{skill}': {source}")]
    InvalidSkillSource {
        skill: String,
        #[source]
        source: SourcePathError,
    },
    #[error("skill '{skill}' references unknown agent '{agent}'")]
    UnknownAgent { skill: String, agent: String },
    #[error("dependency '{skill}' has no target agents")]
    MissingAgents { skill: String },
    #[error("invalid install source for skill '{skill}': {message}")]
    InvalidInstallSource { skill: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub skill_dir: SourcePath,
    pub agents: BTreeMap<String, ResolvedAgent>,
    pub skills: Vec<ResolvedSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgent {
    pub kind: AgentKind,
    pub enabled: bool,
    pub scope: Scope,
    pub target_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSkill {
    pub name: SkillName,
    pub source: SourcePath,
    pub install_source: Option<InstallSource>,
    pub agents: Vec<AgentKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSource {
    Git(GitInstallSource),
    Local(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitInstallSource {
    pub url: String,
    pub reference: Option<String>,
    pub path: PathBuf,
}

pub fn apply_agent_target_dirs(
    config: &mut ResolvedConfig,
    target_dirs: BTreeMap<String, PathBuf>,
) -> Result<(), ConfigResolveError> {
    for (name, target_dir) in target_dirs {
        let kind =
            AgentKind::from_str(&name).map_err(|source| ConfigResolveError::InvalidAgentName {
                name: name.clone(),
                source,
            })?;
        let key = kind.as_str().to_owned();
        config
            .agents
            .entry(key)
            .and_modify(|agent| agent.target_dir = Some(target_dir.clone()))
            .or_insert(ResolvedAgent {
                kind,
                enabled: true,
                scope: Scope::User,
                target_dir: Some(target_dir),
            });
    }
    Ok(())
}
